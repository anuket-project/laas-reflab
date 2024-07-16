//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{
    anyhow, tokio::time::{sleep, Duration}, tracing::{self, error, info, trace, warn}
};

use metrics::prelude::*;
use models::{
    dal::{new_client, AsEasyTransaction, FKey},
    dashboard::{Aggregate, EasyLog, StatusSentiment},
    inventory::{BootTo, Host, Lab},
};
use notifications::email::send_to_admins;
use serde::{Deserialize, Serialize};

use std::sync::{atomic::AtomicBool, Arc};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use super::{
    net_config::{mgmt_network_config, prod_network_config},
    set_boot::SetBoot,
    set_host_power_state::SetPower,
};
use crate::{
    deploy_booking::{
        cobbler_set_config::*, configure_networking::ConfigureNetworking,
        wait_host_os_reachable::WaitHostOSReachable,
    },
    resource_management::{
        cobbler::*,
        ipmi_accounts::CreateIPMIAccount,
        mailbox::{Endpoint, Mailbox, MailboxMessageReceiver},
    },
    retry_for,
};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DeployHost {
    pub host_id: FKey<Host>,
    pub aggregate_id: FKey<Aggregate>,
    // pub with_config: HostConfig,
    pub using_instance: FKey<models::dashboard::Instance>,
    // pub template: FKey<Template>,
}

tascii::mark_task!(DeployHost);
/// Executes the provision process for a single host.
impl AsyncRunnable for DeployHost {
    type Output = ();

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        // reset the context as we can't recover from a partial run
        context.reset();

        let (aggregate, host_name, lab) = self.fetch_host_details().await?;
        // start time of the provision
        let start_time = Timestamp::now();

        let mut err: TaskError = TaskError::Reason(String::from("Host failed to attempt deploy."));
        for _task_retry_no in 0..(self.retry_count() + 1) {
            let result = self
                .deploy_host(context, (&aggregate, &host_name, &lab))
                .await;

            let provisioning_time_seconds = start_time.elapsed();
            self.send_provision_metric(
                &host_name,
                &aggregate,
                provisioning_time_seconds,
                result.is_ok(),
            )
            .await;

            match result {
                Ok(_) => {
                    return result
                },
                Err(e) => {
                    err = e;
                    continue
                },
            }
        }

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();
        let profile = self
            .using_instance
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner()
            .config
            .flavor
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner()
            .name;

        send_to_admins(format!(
            "Failure to provision a host for instance {:?}, this is of profile {profile}",
            self.using_instance
        ))
        .await;

        transaction.commit().await.unwrap();

        Err(err)
    }

    fn summarize(&self, id: models::dal::ID) -> String {
        format!("DeployHost with id {id}")
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("DeployHostTask").versioned(1)
    }

    fn timeout() -> std::time::Duration {
        Duration::from_secs(60 * 60) // 60 minute per individual provision try
    }

    fn retry_count(&self) -> usize {
        3
    }
}

pub enum MockInjectionResult {
    Success,
    Abort(TaskError),
    Continue,
}

impl DeployHost {
    async fn deploy_host(
        &mut self,
        context: &Context,
        (aggregate, host_name, lab): (&Aggregate, &String, &Lab),
    ) -> Result<(), TaskError> {
        match self.wait_for_mock_injection().await {
            MockInjectionResult::Success => return Ok(()),
            MockInjectionResult::Abort(err) => return Err(err),
            MockInjectionResult::Continue => {}
        }

        self.log(
            "Provision Start",
            "a task has started running to provision the host",
            StatusSentiment::in_progress,
        )
        .await;

        let (preimage_waiter, imaging_waiter, mut post_boot_waiter, mut post_provision_waiter) =
            self.generate_endpoints().await;

        self.configure_cobbler_and_set_boot(
            context,
            preimage_waiter.endpoint(),
            imaging_waiter.endpoint(),
            host_name,
        )
        .await?;

        sleep(Duration::from_secs(2)).await;

        self.set_power_on(context, host_name).await?;

        self.configure_mgmt_networking(context, lab.clone()).await?;

        self.install_os(preimage_waiter, imaging_waiter).await?;

        self.set_power_off(context, host_name).await?;

        self.boot_from_disk(context, host_name).await?;

        self.configure_postprovision_networking(context, lab.clone(), &mut post_boot_waiter)
            .await?;

        self.verify_host_provisioned(context, host_name, &mut post_provision_waiter)
            .await?;

        self.setup_ipmi_accounts(context, aggregate.clone(), host_name)
            .await?;

        self.log(
            "Successfully Provisioned",
            &format!("{} has provisioned according to configuration", host_name),
            StatusSentiment::succeeded,
        )
        .await;

        Ok(())
    }
    async fn generate_endpoints(
        &self,
    ) -> (
        MailboxMessageReceiver,
        MailboxMessageReceiver,
        MailboxMessageReceiver,
        MailboxMessageReceiver,
    ) {
        self.log(
            "Generating Endpoints",
            "generating http mailbox targets for host to interact with LibLaaS",
            StatusSentiment::in_progress,
        )
        .await;

        let preimage_waiter = self.set_endpoint_hook("pre_image").await.unwrap();

        let imaging_waiter = self.set_endpoint_hook("post_image").await.unwrap();

        let post_boot_waiter = self.set_endpoint_hook("post_boot").await.unwrap();

        let post_provision_waiter = self.set_endpoint_hook("post_provision").await.unwrap();

        (
            preimage_waiter,
            imaging_waiter,
            post_boot_waiter,
            post_provision_waiter,
        )
    }
    async fn wait_for_mock_injection(&self) -> MockInjectionResult {
        let mut mock_waiter = self.set_endpoint_hook("mock").await.unwrap();

        self.log(
            "Pre-Provision",
            "waiting for mock injection",
            StatusSentiment::in_progress,
        )
        .await;

        match mock_waiter.wait_next(Duration::from_secs(60)) {
            Ok(v) => {
                let val = v.msg.message;
                match serde_json::from_str::<bool>(val.as_str()) {
                    Ok(true) => MockInjectionResult::Success,
                    Ok(false) => MockInjectionResult::Abort(TaskError::Reason(
                        "mock indicated failure".to_string(),
                    )),
                    Err(_) => {
                        // this timed out, so just continue with the booking
                        self.log(
                            "Pre-Provision Done",
                            "no mock injection occurred, continuing with provision process",
                            StatusSentiment::in_progress,
                        )
                        .await;
                        MockInjectionResult::Continue
                    }
                }
            }
            Err(_) => MockInjectionResult::Continue,
        }
    }

    async fn fetch_host_details(&self) -> Result<(Aggregate, String, Lab), anyhow::Error> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let aggregate = self
            .aggregate_id
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner();

        let host_name = self
            .host_id
            .get(&mut transaction)
            .await?
            .server_name
            .clone();

        let lab = aggregate
            .lab
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner();

        Ok((aggregate, host_name, lab))
    }

    async fn configure_cobbler_and_set_boot(
        &self,
        context: &Context,
        preimage_endpoint: Endpoint,
        postimage_endpoint: Endpoint,
        host_name: &str,
    ) -> Result<(), TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        info!("made endpoint for host: {:?}", postimage_endpoint);

        self.log(
            "Setting Image",
            "configuring the installer service to use the selected image",
            StatusSentiment::in_progress,
        )
        .await;

        let cobbler_config_jh = context.spawn(CobblerSetConfiguration {
            host_id: self.host_id,
            config: CobblerConfig::new(
                self.using_instance
                    .get(&mut transaction)
                    .await?
                    .into_inner(),
                self.host_id,
                postimage_endpoint,
                preimage_endpoint,
            )
            .await,
            endpoint: postimage_endpoint,
        });

        warn!("setting boot dev for {} to network boot", host_name);

        self.log(
            "Network Boot Configuration",
            "configuring the host to boot the installer from network",
            StatusSentiment::in_progress,
        )
        .await;

        let res = retry_for(
            SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: BootTo::Network,
            },
            context,
            5,
            10,
        );

        // Setting boot dev before powering host off as that seems to matter to the intels.
        info!("set boot res: {:?}", res);

        sleep(Duration::from_secs(2)).await;

        self.log(
            "Powering Host Off",
            "power host off to configure boot devices",
            StatusSentiment::in_progress,
        )
        .await;

        retry_for(SetPower::off(self.host_id), context, 5, 10)?;

        info!("Making sure cobbler config is done");

        cobbler_config_jh.join()?;

        info!(
            "set cobbler configuration and finished mgmt net config, also set host next boot dev"
        );

        transaction.commit().await?;

        Ok(())
    }

    async fn set_power_on(&self, context: &Context, host_name: &str) -> Result<(), TaskError> {
        warn!("setting host {} power on", host_name);

        self.log(
            "Powering Host On",
            "power host on to boot the netinstall image",
            StatusSentiment::in_progress,
        )
        .await;

        retry_for(SetPower::on(self.host_id), context, 5, 10)?;

        info!(
            "set power on, now adding pxe nets so host can pxe (in time it takes for host to post)"
        );

        Ok(())
    }

    async fn set_power_off(&self, context: &Context, host_name: &str) -> Result<(), TaskError> {
        warn!("Setting host {} power off", host_name);
        retry_for(SetPower::off(self.host_id), context, 5, 10)?;
        warn!("Set host {} power off", host_name);

        Ok(())
    }

    async fn boot_from_disk(&self, context: &Context, host_name: &str) -> Result<(), TaskError> {
        self.log(
            "Booting From Disk",
            "host is being configured to boot the now-installed operating system",
            StatusSentiment::in_progress,
        )
        .await;

        warn!("Booting host {} from disk", host_name);
        let result = retry_for(
            SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: BootTo::Disk,
            },
            context,
            5,
            10,
        );

        match result {
            Ok(_) => {
                warn!("Set host {} to boot from disk", host_name);
            }
            Err(e) => {
                error!(
                    "Failed to set host {} to boot from disk: {:?}",
                    host_name, e
                );
                TaskError::Reason(format!("Failed to set host to boot from disk {:?}", e));
            }
        }

        warn!("Powering host {} on", host_name);
        retry_for(SetPower::on(self.host_id), context, 5, 10)?;
        warn!("Successfully set host {} power on", host_name);

        Ok(())
    }

    async fn configure_mgmt_networking(
        &self,
        context: &Context,
        lab: Lab,
    ) -> Result<(), TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        if lab.is_dynamic {
            self.log(
                "Network Backplane Configuration",
                "configuring the network backplane to allow the host to network boot",
                StatusSentiment::in_progress,
            )
            .await;

            // need mgmt nets set before we can try ipmi managing the host
            context
                .spawn(ConfigureNetworking {
                    net_config: mgmt_network_config(self.host_id, &mut transaction).await,
                })
                .join()?;
        } else {
            self.log(
                "Network Boot Configuration",
                "attempting to network boot to install image.",
                StatusSentiment::in_progress,
            )
            .await;
        }

        info!(
            "successfully configured pxe networking for {:?}",
            self.host_id
        );

        transaction.commit().await?;

        Ok(())
    }

    async fn install_os(
        &self,
        mut preimage_waiter: MailboxMessageReceiver,
        mut imaging_waiter: MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        self.log(
            "Installing OS",
            "booting the installer for the base OS and provisioning the host",
            StatusSentiment::in_progress,
        )
        .await;

        let inst_id = self.using_instance;
        let finished_imaging = Arc::new(AtomicBool::new(false));
        let fcopy = finished_imaging.clone();
        std::thread::spawn(move || {
            let mb_return = preimage_waiter.wait_next(Duration::from_secs(60 * 35));
            match (fcopy.load(std::sync::atomic::Ordering::SeqCst), mb_return) {
                (true, _) => {
                    // don't report anything, skip
                    warn!("Host didn't phone home before imaging!");
                }
                (false, Ok(_)) => {
                    tascii::executors::spawn_on_tascii_tokio("laas_notifications", async move {
                        inst_id.log("Installing OS", "host has booted into the installer, and is now installing the base OS", StatusSentiment::in_progress).await;
                    });
                }
                (false, Err(_e)) => {
                    tascii::executors::spawn_on_tascii_tokio("laas_notifications", async move {
                        inst_id
                            .log(
                                "Failed to Boot",
                                "host failed to reach the installer",
                                StatusSentiment::degraded,
                            )
                            .await;
                    });
                }
            }
        });

        match imaging_waiter.wait_next(Duration::from_secs(60 * 35)) {
            // give the host 20 minutes to boot
            // and provision
            Ok(_) => {
                // TODO: allow host to post a *failure* message, so we can detect that and save
                // those logs and force it to reboot and retry (and detect this all early)
                info!("Imaging successful!!")
            }
            Err(e) => {
                self.log(
                        "OS Install Failed",
                        "installing the OS timed out or experienced an early failure, initiating error recovery routines",
                        StatusSentiment::degraded,
                    )
                    .await;

                error!("MAILBOX FAILED with {:?}", e);
                return Err(TaskError::Reason("Mailbox error".to_owned()));
            }
        }

        finished_imaging.store(true, std::sync::atomic::Ordering::SeqCst);

        self.log(
            "OS Installed",
            "the unconfigured operating system has been installed onto the host",
            StatusSentiment::in_progress,
        )
        .await;

        Ok(())
    }

    async fn configure_postprovision_networking(
        &self,
        context: &Context,
        lab: Lab,
        post_boot_waiter: &mut MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        self.log(
            "Wait Host Pre-Configure",
            "wait for host to boot into pre-configure mode and bootstrap the configuration environment",
            StatusSentiment::in_progress,
        )
        .await;

        match post_boot_waiter.wait_next(Duration::from_secs(60 * 35)) {
            Ok(_) => {
                info!("Host came back up after imaging");
            }
            Err(e) => {
                self.log(
                    "Pre-Configure Wait Failed",
                    "host failed to boot into pre-configure mode, initiating error recovery routines",
                    StatusSentiment::degraded,
                )
                .await;
                error!("MAILBOX FAILED with {:?}", e);
                return Err(TaskError::Reason("Mailbox error".to_owned()));
            }
        }

        if lab.is_dynamic {
            self.log(
                "Host Configure",
                "host completed pre-configuration, and is now applying production network config",
                StatusSentiment::in_progress,
            )
            .await;

            warn!("about to configure prod networking");

            self.log(
                "Network Backplane Configuration",
                "setting up final networks within the backplane as configured for the template",
                StatusSentiment::in_progress,
            )
            .await;

            let mut client = new_client().await.unwrap();
            let mut transaction = client.easy_transaction().await.unwrap();

            context
                .spawn(ConfigureNetworking {
                    net_config: prod_network_config(
                        self.host_id,
                        self.using_instance,
                        &mut transaction,
                    )
                    .await,
                })
                .join()?;
        }

        Ok(())
    }

    async fn verify_host_provisioned(
        &self,
        context: &Context,
        host_name: &str,
        post_provision_waiter: &mut MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        self.log(
            "Wait Host Online",
            "wait for host to complete on-device setup steps (incl Cloud-Init)",
            StatusSentiment::in_progress,
        )
        .await;

        match post_provision_waiter.wait_next(Duration::from_secs(60 * 20)) {
            Ok(_) => {
                info!("Host came back up after applying network configs");
            }
            Err(e) => {
                self.log(
                    "On-Device Setup Failed",
                    "host failed to complete on-device configuration, initiating error recovery routines",
                    StatusSentiment::degraded,
                )
                .await;

                error!("MAILBOX FAILED with {:?}", e);
                return Err(TaskError::Reason("Mailbox error".to_owned()));
            }
        }

        self.log(
            "Verify Host Provisioned",
            &format!("check that host is reachable at {}", host_name),
            StatusSentiment::in_progress,
        )
        .await;

        context
            .spawn(WaitHostOSReachable {
                timeout: Duration::from_secs(60 * 15),
                host_id: self.host_id,
            })
            .join()?;

        warn!("Host {:?} provisioned successfully", self.host_id);

        Ok(())
    }

    async fn setup_ipmi_accounts(
        &self,
        context: &Context,
        aggregate: Aggregate,
        host_name: &str,
    ) -> Result<(), TaskError> {
        self.log(
            "Set Up IPMI Accounts",
            "IPMI accounts are being added to {host_name}",
            StatusSentiment::in_progress,
        )
        .await;

        let ipmi_res = retry_for(
            CreateIPMIAccount {
                host: self.host_id,
                password: aggregate.configuration.ipmi_password,
                username: aggregate.configuration.ipmi_username,
                userid: "4".to_string(), // TODO: look into generating this later on
            },
            context,
            5,
            30,
        );

        if let Err(e) = ipmi_res {
            send_to_admins(format!(
                "Failed to set up IPMI accounts for {}, manual intervention required",
                host_name
            ))
            .await;

            self
            .log(
                    "Failed to Set Up IPMI",
                    "IPMI accounts couldn't be set up for {host_name}, \
                            the administrators have been notified and will manually set up accounts shortly",
                            StatusSentiment::degraded,
            )
            .await;
            return Err(TaskError::Reason(format!(
                "Failed to set up IPMI accounts for {}: {:?}",
                host_name, e
            )));
        }

        Ok(())
    }

    async fn set_endpoint_hook(
        &self,
        usage: &'static str,
    ) -> Result<MailboxMessageReceiver, anyhow::Error> {
        Mailbox::set_endpoint_hook(self.using_instance, usage).await
    }
    async fn log(&self, msg: &str, desc: &str, sentiment: StatusSentiment) {
        self.using_instance.log(msg, desc, sentiment).await;
    }

    async fn send_provision_metric(
        &self,
        host_name: &str,
        aggregate: &Aggregate,
        duration: u64,
        success: bool,
    ) {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let provision_metric = ProvisionMetric {
            hostname: host_name.to_string(),
            owner: aggregate
                .metadata
                .owner
                .clone()
                .unwrap_or_else(|| "None".to_string()),
            // Hopefully the right name
            project: aggregate
                .lab
                .get(&mut transaction)
                .await
                .map_or_else(|_| "None".to_string(), |v| v.name.clone()),
            provisioning_time_seconds: duration,
            success,
            ..Default::default()
        };

        transaction.commit().await.unwrap();

        if let Err(e) = MetricHandler::send(provision_metric) {
            error!("Failed to send provision metric: {:?}", e);
        } else {
            trace!("Provision metric sent successfully");
        }
    }
}

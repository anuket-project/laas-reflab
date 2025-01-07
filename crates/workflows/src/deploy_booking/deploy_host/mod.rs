use common::prelude::{
    anyhow,
    tokio::time::{sleep, Duration},
    tracing::{self, error, info, trace, warn},
};

use config::{self, settings};
use dal::{new_client, AsEasyTransaction, FKey, ID};
use metrics::prelude::*;

use models::{
    dashboard::{Aggregate, StatusSentiment},
    inventory::{BootTo, Host, Lab},
    EasyLog,
};
use notifications::email::send_to_admins;
use serde::{Deserialize, Serialize};
use ssh2::Session;

use std::{
    net::TcpStream,
    sync::{atomic::AtomicBool, Arc},
};
use strum_macros::EnumString;
use tascii::{prelude::*, task_trait::AsyncRunnable};

use super::{
    net_config::{mgmt_network_config, prod_network_config},
    set_boot::SetBoot,
    set_host_power_state::SetPower,
};
use crate::{
    deploy_booking::{
        cobbler_set_config::*, configure_networking::ConfigureNetworking,
        net_config::mgmt_network_config_with_public, wait_host_os_reachable::WaitHostOSReachable,
    },
    resource_management::{
        cobbler::*,
        ipmi_accounts::CreateIPMIAccount,
        mailbox::{Endpoint, Mailbox, MailboxMessageReceiver},
    },
    retry_for,
};

/// A WorkflowDistro is used for path branching within a workflow.
#[derive(Debug, Hash, EnumString, Deserialize, Serialize, Clone)]
#[strum(serialize_all = "lowercase")]
pub enum WorkflowDistro {
    Ubuntu,
    Fedora,
    Eve,
}

impl WorkflowDistro {
    fn from_str(s: &str) -> Result<Self, anyhow::Error> {
        let s = s.to_lowercase();
        if s.contains("ubuntu") {
            return Ok(WorkflowDistro::Ubuntu);
        }
        if s.contains("fedora") {
            return Ok(WorkflowDistro::Fedora);
        }
        if s.contains("eve") {
            return Ok(WorkflowDistro::Eve);
        }

        Err(anyhow::anyhow!("Unable to parse WorkflowDistro from {}", s))
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DeployHost {
    pub host_id: FKey<Host>,
    pub aggregate_id: FKey<Aggregate>,
    pub using_instance: FKey<models::dashboard::Instance>,
    pub distribution: Option<WorkflowDistro>,
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
                Ok(_) => return result,
                Err(e) => {
                    err = e;
                    continue;
                }
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

    fn summarize(&self, id: ID) -> String {
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
    /// Fetches the image name for an instance and converts it to a workflow distro
    /// Returns an error if it is unable to do so
    /// Currently recomputes the workflow distro every time this is called
    async fn get_workflow_distro(&mut self) -> Result<WorkflowDistro, anyhow::Error> {
        if let Some(distro) = &self.distribution {
            return Ok(distro.clone());
        } else {
            let mut client = new_client().await.unwrap();
            let mut transaction = client.easy_transaction().await.unwrap();

            let image_name = self
                .using_instance
                .get(&mut transaction)
                .await?
                .into_inner()
                .config
                .image
                .get(&mut transaction)
                .await
                .unwrap()
                .into_inner()
                .name;
            transaction.commit().await.unwrap();
            let distro = WorkflowDistro::from_str(&image_name).unwrap();

            self.distribution = Some(distro.clone());
            return Ok(distro);
        }
    }

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
            StatusSentiment::InProgress,
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
            StatusSentiment::Succeeded,
        )
        .await;

        Ok(())
    }

    async fn generate_endpoints(
        &mut self,
    ) -> (
        MailboxMessageReceiver,
        MailboxMessageReceiver,
        MailboxMessageReceiver,
        MailboxMessageReceiver,
    ) {
        self.log(
            "Generating Endpoints",
            "generating http mailbox targets for host to interact with LibLaaS",
            StatusSentiment::InProgress,
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
    async fn wait_for_mock_injection(&mut self) -> MockInjectionResult {
        let mut mock_waiter = self.set_endpoint_hook("mock").await.unwrap();

        self.log(
            "Pre-Provision",
            "waiting for mock injection",
            StatusSentiment::InProgress,
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
                            StatusSentiment::InProgress,
                        )
                        .await;
                        MockInjectionResult::Continue
                    }
                }
            }
            Err(_) => MockInjectionResult::Continue,
        }
    }

    async fn fetch_host_details(&mut self) -> Result<(Aggregate, String, Lab), anyhow::Error> {
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
        &mut self,
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
            StatusSentiment::InProgress,
        )
        .await;

        let wfd = self.get_workflow_distro().await?;

        let cobbler_config_jh = context.spawn(CobblerSetConfiguration {
            host_id: self.host_id,
            config: match wfd {
                WorkflowDistro::Eve => {
                    CobblerConfig::new_eve_config(
                        self.using_instance
                            .get(&mut transaction)
                            .await?
                            .into_inner(),
                        self.host_id,
                        Some("sda".to_owned()),
                    )
                    .await
                }
                _ => {
                    CobblerConfig::new(
                        self.using_instance
                            .get(&mut transaction)
                            .await?
                            .into_inner(),
                        self.host_id,
                        postimage_endpoint,
                        preimage_endpoint,
                    )
                    .await
                }
            },
            endpoint: postimage_endpoint,
        });

        warn!("setting boot dev for {} to network boot", host_name);

        self.log(
            "Network Boot Configuration",
            "configuring the host to boot the installer from network",
            StatusSentiment::InProgress,
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
            StatusSentiment::InProgress,
        )
        .await;

        retry_for(SetPower::off(self.host_id), context, 5, 10)?;

        info!("Making sure cobbler config is done");

        cobbler_config_jh.join()?;

        match wfd {
            WorkflowDistro::Eve => {
                let config::CobblerConfig {
                    address,
                    url,
                    username,
                    password,
                    api_username,
                    api_password,
                } = settings().cobbler.clone();

                let mut session =
                    Session::new().expect("Failed to create a new SSH session for cobbler.");
                let connection = TcpStream::connect(format!("{address}:22")).expect(
                    format!("Failed to open TCP stream to cobbler at {address}:22.").as_str(),
                );

                session.set_tcp_stream(connection);
                session.handshake().unwrap();
                session
                    .userauth_password(username.as_str(), password.as_str())
                    .expect("SSH basic authentication failed");

                let mut channel = session
                    .channel_session()
                    .expect("Expected to connect to cobbler to fix the eve cfg files!");
                channel.exec(format!("sudo /srv/www/cobbler/distro_mirror/eve-amd64/set-eve.sh {} 12.0.3-lts", host_name).as_str()).expect("Expected to get host arch info");

                tracing::info!(
                    "set cobbler configuration, edited grub config and finished mgmt net config, also set host next boot dev"
                );
            }
            _ => {
                info!(
                    "set cobbler configuration and finished mgmt net config, also set host next boot dev"
                );
            }
        };

        transaction.commit().await?;

        Ok(())
    }

    async fn set_power_on(&mut self, context: &Context, host_name: &str) -> Result<(), TaskError> {
        warn!("setting host {} power on", host_name);

        self.log(
            "Powering Host On",
            "power host on to boot the netinstall image",
            StatusSentiment::InProgress,
        )
        .await;

        retry_for(SetPower::on(self.host_id), context, 5, 10)?;

        info!(
            "set power on, now adding pxe nets so host can pxe (in time it takes for host to post)"
        );

        Ok(())
    }

    async fn set_power_off(&mut self, context: &Context, host_name: &str) -> Result<(), TaskError> {
        match self.get_workflow_distro().await? {
            WorkflowDistro::Eve => {}
            _ => {
                warn!("Setting host {} power off", host_name);
                retry_for(SetPower::off(self.host_id), context, 5, 10)?;
                warn!("Set host {} power off", host_name);
            }
        }
        Ok(())
    }

    async fn boot_from_disk(
        &mut self,
        context: &Context,
        host_name: &str,
    ) -> Result<(), TaskError> {
        self.log(
            "Booting From Disk",
            "host is being configured to boot the now-installed operating system",
            StatusSentiment::InProgress,
        )
        .await;

        let wfd = self.get_workflow_distro().await?;

        warn!("Booting host {} from disk", host_name);
        let result = retry_for(
            SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: match wfd {
                    WorkflowDistro::Eve => BootTo::SpecificDisk,
                    _ => BootTo::Disk,
                },
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
        &mut self,
        context: &Context,
        lab: Lab,
    ) -> Result<(), TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        if lab.is_dynamic {
            self.log(
                "Network Backplane Configuration",
                "configuring the network backplane to allow the host to network boot",
                StatusSentiment::InProgress,
            )
            .await;

            // need mgmt nets set before we can try ipmi managing the host
            context
                .spawn(ConfigureNetworking {
                    net_config: match self.get_workflow_distro().await? {
                        WorkflowDistro::Eve => {
                            mgmt_network_config_with_public(
                                self.host_id,
                                self.using_instance,
                                &mut transaction,
                            )
                            .await
                        }
                        _ => mgmt_network_config(self.host_id, &mut transaction).await,
                    },
                })
                .join()?;
        } else {
            self.log(
                "Network Boot Configuration",
                "attempting to network boot to install image.",
                StatusSentiment::InProgress,
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
        &mut self,
        mut preimage_waiter: MailboxMessageReceiver,
        mut imaging_waiter: MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        self.log(
            "Installing OS",
            "booting the installer for the base OS and provisioning the host",
            StatusSentiment::InProgress,
        )
        .await;

        let wfd = self.get_workflow_distro().await?;

        match wfd {
            WorkflowDistro::Eve => {
                // wait 7 mins
                sleep(Duration::new(900, 0)).await;
            }
            _ => {
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
                            tascii::executors::spawn_on_tascii_tokio(
                                "laas_notifications",
                                async move {
                                    inst_id.log("Installing OS", "host has booted into the installer, and is now installing the base OS", StatusSentiment::InProgress).await;
                                },
                            );
                        }
                        (false, Err(_e)) => {
                            tascii::executors::spawn_on_tascii_tokio(
                                "laas_notifications",
                                async move {
                                    inst_id
                                        .log(
                                            "Failed to Boot",
                                            "host failed to reach the installer",
                                            StatusSentiment::Degraded,
                                        )
                                        .await;
                                },
                            );
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
                                StatusSentiment::Degraded,
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
                    StatusSentiment::InProgress,
                )
                .await;
            }
        }
        Ok(())
    }

    async fn configure_postprovision_networking(
        &mut self,
        context: &Context,
        lab: Lab,
        post_boot_waiter: &mut MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        let wfd = self.get_workflow_distro().await?;

        match wfd {
            WorkflowDistro::Eve => {}
            _ => {
                self.log(
                    "Wait Host Pre-Configure",
                    "wait for host to boot into pre-configure mode and bootstrap the configuration environment",
                    StatusSentiment::InProgress,
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
                            StatusSentiment::Degraded,
                        )
                        .await;
                        error!("MAILBOX FAILED with {:?}", e);
                        return Err(TaskError::Reason("Mailbox error".to_owned()));
                    }
                }
            }
        }

        if lab.is_dynamic {
            self.log(
                "Host Configure",
                "host completed pre-configuration, and is now applying production network config",
                StatusSentiment::InProgress,
            )
            .await;

            warn!("about to configure prod networking");

            self.log(
                "Network Backplane Configuration",
                "setting up final networks within the backplane as configured for the template",
                StatusSentiment::InProgress,
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
        &mut self,
        context: &Context,
        host_name: &str,
        post_provision_waiter: &mut MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        let wfd = self.get_workflow_distro().await?;

        match wfd {
            WorkflowDistro::Eve => {
                self.log(
                    "Wait Host Online",
                    "wait for host to come online",
                    StatusSentiment::InProgress,
                )
                .await;
            }
            _ => {
                self.log(
                    "Wait Host Online",
                    "wait for host to complete on-device setup steps (incl Cloud-Init)",
                    StatusSentiment::InProgress,
                )
                .await;

                match post_provision_waiter.wait_next(Duration::from_secs(60 * 30)) {
                    Ok(_) => {
                        info!("Host came back up after applying network configs");
                    }
                    Err(e) => {
                        self.log(
                            "On-Device Setup Failed",
                            "host failed to complete on-device configuration, initiating error recovery routines",
                            StatusSentiment::Degraded,
                        )
                        .await;

                        error!("MAILBOX FAILED with {:?}", e);
                        return Err(TaskError::Reason("Mailbox error".to_owned()));
                    }
                }
            }
        }

        self.log(
            "Verify Host Provisioned",
            &format!("check that host is reachable at {}", host_name),
            StatusSentiment::InProgress,
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
        &mut self,
        context: &Context,
        aggregate: Aggregate,
        host_name: &str,
    ) -> Result<(), TaskError> {
        self.log(
            "Set Up IPMI Accounts",
            &format!("IPMI accounts are being added to {host_name}"),
            StatusSentiment::InProgress,
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
                            StatusSentiment::Degraded,
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
        &mut self,
        usage: &'static str,
    ) -> Result<MailboxMessageReceiver, anyhow::Error> {
        Mailbox::set_endpoint_hook(self.using_instance, usage).await
    }
    async fn log(&mut self, msg: &str, desc: &str, sentiment: StatusSentiment) {
        self.using_instance.log(msg, desc, sentiment).await;
    }

    async fn send_provision_metric(
        &mut self,
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
            lab: aggregate
                .lab
                .get(&mut transaction)
                .await
                .map_or_else(|_| "None".to_string(), |v| v.name.clone()),
            project: aggregate
                .metadata
                .project
                .clone()
                .unwrap_or_else(|| "None".to_string()),
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

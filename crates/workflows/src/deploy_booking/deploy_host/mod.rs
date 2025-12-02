use common::prelude::{
    anyhow,
    tokio::time::{sleep, Duration},
    tracing::{error, info, warn},
};

use config::settings;
use dal::{new_client, AsEasyTransaction, FKey, ID};

use models::{
    dashboard::{types::Distro, Aggregate, Instance, NetworkAssignmentMap, StatusSentiment},
    inventory::{BootTo, Host, Lab},
    EasyLog,
};
use notifications::{email::send_to_admins, templates::render_eve_grub_config};
use serde::{Deserialize, Serialize};
use users::ipa;

use tascii::{prelude::*, task_trait::AsyncRunnable};

use super::{set_boot::SetBoot, set_host_power_state::SetPower};
use crate::{
    configure_networking::{
        mgmt_network_config, prod_network_config,
        vlan_connection::create_network_manager_vlan_connections_from_bondgroups,
        ConfigureNetworking,
    },
    deploy_booking::{
        cobbler_set_config::*,
        reachable::WaitReachable,
        set_host_power_state::{confirm_power_state, HostConfig, PowerState, TimeoutConfig},
    },
    render_kickstart_template,
    resource_management::{
        cobbler::*,
        ipmi_accounts::CreateIPMIAccount,
        mailbox::{Endpoint, Mailbox, MailboxMessageReceiver},
    },
};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DeployHost {
    pub host_id: FKey<Host>,
    pub aggregate_id: FKey<Aggregate>,
    pub using_instance: FKey<models::dashboard::Instance>,
    pub distribution: Option<Distro>,
}

tascii::mark_task!(DeployHost);
/// Executes the provision process for a single host.
impl AsyncRunnable for DeployHost {
    type Output = ();

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        // reset the context as we can't recover from a partial run
        context.reset();

        let (aggregate, host_name, lab) = self.fetch_host_details().await?;

        let deploy_host_result = self
            .deploy_host(context, (&aggregate, &host_name, &lab))
            .await;

        if let Err(e) = deploy_host_result {
            self.log(
                "Host Deployment Attempt Failed",
                "something went wrong, and admins have been notified",
                StatusSentiment::InProgress,
            )
            .await;

            tracing::error!("{e:?}");
            return Err(e);
        }

        Ok(())
    }

    fn summarize(&self, id: ID) -> String {
        format!("DeployHost with id {id}")
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("DeployHostTask").versioned(1)
    }

    fn timeout() -> std::time::Duration {
        // Any individual provision attempt won't realistically take longer than 45 minutes.
        // It isn't useful or valuable to base this timeout on the overall timeouts of child tasks
        // as something is likely wrong if it takes that long, so we're better off
        // just failing early and trying again.
        Duration::from_secs(60 * 45)
    }

    fn retry_count() -> usize {
        1
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
    async fn get_distro(&mut self) -> Result<Distro, anyhow::Error> {
        if let Some(distro) = &self.distribution {
            Ok(*distro)
        } else {
            let mut client = new_client().await.unwrap();
            let mut transaction = client.easy_transaction().await.unwrap();

            let distro = self
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
                .distro;
            transaction.commit().await.unwrap();

            Ok(distro)
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

        self.prepare_host_environment(context, host_name).await?;

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

        match mock_waiter.wait_next(Duration::from_secs(10)) {
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

        transaction.commit().await?;

        Ok((aggregate, host_name, lab))
    }

    async fn fetch_instance_image(&mut self) -> Result<models::dashboard::Image, anyhow::Error> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let image = self
            .using_instance
            .get(&mut transaction)
            .await?
            .config
            .image
            .get(&mut transaction)
            .await?
            .into_inner();

        transaction.commit().await?;

        Ok(image)
    }

    async fn fetch_instance_host(&mut self) -> Result<Host, anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;

        let host = self.host_id.get(&mut transaction).await?;
        transaction.commit().await.unwrap();

        Ok(host.into_inner())
    }

    async fn fetch_instance_config(
        &mut self,
    ) -> Result<models::dashboard::HostConfig, anyhow::Error> {
        let instance = self.fetch_instance().await?;
        let config = instance.config;

        Ok(config)
    }

    async fn fetch_instance(&mut self) -> Result<Instance, anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;

        let instance = self.using_instance.get(&mut transaction).await?;
        transaction.commit().await.unwrap();

        Ok(instance.into_inner())
    }

    async fn fetch_network_assignment_map(
        &mut self,
    ) -> Result<NetworkAssignmentMap, anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;

        let aggregate = self.aggregate_id.get(&mut transaction).await?;

        let network_assignment_map = aggregate.vlans.get(&mut transaction).await?;
        transaction.commit().await.unwrap();

        Ok(network_assignment_map.into_inner())
    }

    async fn fetch_users(&mut self) -> Result<Vec<ipa::User>, anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;

        let aggregate = self.aggregate_id.get(&mut transaction).await?;

        let mut ipa = ipa::IPA::init()
            .await
            .expect("Expected to initialize IPA connection");

        let mut ipa_users: Vec<ipa::User> = vec![];

        for username in aggregate.users.iter() {
            let user = ipa
                .find_matching_user(username.clone(), true, false)
                .await
                .unwrap();

            ipa_users.push(user);
        }

        Ok(ipa_users)
    }

    /// Updates the 'soft_serial' key with the provided value in the instance metadata.
    async fn set_soft_serial(&mut self, value: &str) -> Result<(), anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;
        let mut inst = self.using_instance.get(&mut transaction).await?;
        inst.metadata
            .insert("soft_serial".to_owned(), serde_json::to_value(value)?);
        inst.update(&mut transaction).await?;
        transaction.commit().await?;

        Ok(())
    }

    async fn prepare_host_environment(
        &mut self,
        context: &Context,
        host_name: &str,
    ) -> Result<(), TaskError> {
        self.log(
            "Preparing host environment",
            "performing additional operations to ensure a clean installation",
            StatusSentiment::InProgress,
        )
        .await;

        match self.get_distro().await? {
            Distro::Eve => self.prepare_eve_environment(context, host_name).await,
            _ => {
                self.log(
                    "Host environment ready",
                    "no additional preparations are needed to install this operating system",
                    StatusSentiment::InProgress,
                )
                .await;
                Ok(())
            }
        }
    }

    async fn prepare_eve_environment(
        &mut self,
        context: &Context,
        host_name: &str,
    ) -> Result<(), TaskError> {
        const PREINSTALL_IMAGE_NAME: &str = "ubuntu_wipefs-x86_64";

        // Set to wipefs image, which will wipe disk filesystems upon PXE booting
        self.log(
            "Setting EVE-OS pre-install image",
            "configuring netboot for EVE-OS pre-install environment",
            StatusSentiment::InProgress,
        )
        .await;

        // Todo - handle alternate case for aarch64
        let wipefs_cobbler_join_handle = context.spawn(CobblerSetConfiguration {
            host_id: self.host_id,
            config: CobblerConfig::new(
                PREINSTALL_IMAGE_NAME.to_string(),
                self.using_instance,
                self.host_id,
                None,
                None,
                vec![],
            ),
        });

        // Set device to PXE boot
        self.log(
            "Setting PXE boot device",
            "requesting PXE boot from the BMC",
            StatusSentiment::InProgress,
        )
        .await;

        context
            .spawn(SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: BootTo::Network,
            })
            .join()?;

        sleep(Duration::from_secs(2)).await;

        self.log(
            "Powering Host Off",
            "power host off to boot into the EVE-OS pre-install environment",
            StatusSentiment::InProgress,
        )
        .await;

        context.spawn(SetPower::off(self.host_id)).join()?;

        self.log(
            "Waiting for provisioning server",
            "checking boot device configuration",
            StatusSentiment::InProgress,
        )
        .await;
        wipefs_cobbler_join_handle.join()?;

        self.log(
            "Powering host on",
            "booting into pre-install environment",
            StatusSentiment::InProgress,
        )
        .await;

        // Power on and wait for early commands to run
        self.set_power_on(context, host_name).await?;

        self.log(
            "Performing pre-install cleanup",
            "finalizing environment for EVE-OS install",
            StatusSentiment::InProgress,
        )
        .await;

        sleep(Duration::from_secs(3 * 60)).await;

        self.log(
            "Confirming host state",
            "checking results of pre-install cleanup",
            StatusSentiment::InProgress,
        )
        .await;

        // todo - create a new mailbox target that is hit by the early script after wiping disks. For now wait until host is powered off.
        let host = self.fetch_instance_host().await?;

        confirm_power_state(
            &HostConfig {
                fqdn: host.ipmi_fqdn,
                user: host.ipmi_user,
                password: host.ipmi_pass,
            },
            &TimeoutConfig {
                max_retries: 20,
                retry_interval: 30,
                timeout_duration: 600,
            },
            Some(Duration::from_secs(60)),
            PowerState::Off,
        )
        .await
        .unwrap();

        self.log(
            "Host environment ready",
            "EVE-OS is now ready to be installed",
            StatusSentiment::InProgress,
        )
        .await;

        Ok(())
    }

    /// Generates a soft serial number, renders the grub config, and pushes to cobbler
    async fn configure_cobbler_for_eve(&mut self) -> Result<(), TaskError> {
        self.log(
            "Preparing netinstaller",
            "configuring EVE-OS installer arguments",
            StatusSentiment::InProgress,
        )
        .await;

        let soft_serial = generate_soft_serial(16);
        self.set_soft_serial(&soft_serial).await?;

        // Render template
        let grub_config_content = render_eve_grub_config(
            &self.fetch_host_details().await.unwrap().1,
            &self.fetch_instance_image().await.unwrap().cobbler_name, // ex: "eveos-12.0.4-lts-x86_64"
            "sda",
            &soft_serial,
        )
        .unwrap();

        // Push to cobbler
        let host = self.fetch_instance_host().await.unwrap();
        override_system_grub_config(&host, &grub_config_content).await?;

        Ok(())
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

        // Note - careful of too many open transactions at once. If things unexpectly stop working, check that first

        info!("made endpoint for host: {:?}", postimage_endpoint);

        self.log(
            "Setting Image",
            "configuring the installer service to use the selected image",
            StatusSentiment::InProgress,
        )
        .await;

        let workflow_distro = self.get_distro().await?;

        let cobbler_config_jh = context.spawn(CobblerSetConfiguration {
            host_id: self.host_id,
            config: CobblerConfig::new(
                self.fetch_instance_image().await?.cobbler_name,
                self.using_instance,
                self.host_id,
                Some(postimage_endpoint),
                Some(preimage_endpoint),
                match workflow_distro {
                    Distro::Fedora | Distro::Alma => {
                        let host = self
                            .host_id
                            .get(&mut transaction)
                            .await
                            .expect("host did not exist by given fk?");

                        let mut port_args: Vec<(String, String)> = vec![];

                        for port in host
                            .ports(&mut transaction)
                            .await
                            .expect("didn't get ports?")
                        {
                            port_args.push((
                                "ifname".to_string(),
                                format!("{}:{}", port.name, port.mac),
                            ));
                        }

                        let cobbler_address = settings().cobbler.ssh.address.clone();
                        port_args.push((
                            "inst.ks".to_string(),
                            format!("http://{cobbler_address}/laas_files/fedora-kickstarts/{host_name}.ks"),
                        ));

                        port_args
                    }
                    _ => vec![],
                },
            ),
        });

        warn!("setting boot dev for {} to network boot", host_name);

        self.log(
            "Network Boot Configuration",
            "configuring the host to boot the installer from network",
            StatusSentiment::InProgress,
        )
        .await;

        context
            .spawn(SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: BootTo::Network,
            })
            .join()?;

        sleep(Duration::from_secs(2)).await;

        self.log(
            "Powering Host Off",
            "power host off to configure boot devices",
            StatusSentiment::InProgress,
        )
        .await;

        context.spawn(SetPower::off(self.host_id)).join()?;

        info!("Making sure cobbler config is done");

        cobbler_config_jh.join()?;

        match workflow_distro {
            Distro::Eve => {
                self.configure_cobbler_for_eve().await?;
            }
            Distro::Fedora | Distro::Alma => {
                info!("Configuring host {} for Fedora on Cobbler", host_name);

                let interfaces = self
                    .host_id
                    .get(&mut transaction)
                    .await?
                    .ports(&mut transaction)
                    .await
                    .unwrap();

                let network_assignment_map = self.fetch_network_assignment_map().await?;
                let host_config = self.fetch_instance_config().await?;
                let vlan_configs: Vec<String> =
                    create_network_manager_vlan_connections_from_bondgroups(
                        &network_assignment_map,
                        &host_config.connections,
                    )
                    .await?
                    .iter()
                    .map(|nm_conn| nm_conn.render_kickstart_network_config())
                    .collect();

                let kickstart_template = render_kickstart_template(
                    self.fetch_instance_image().await.unwrap().cobbler_name,
                    self.fetch_users().await.unwrap(),
                    interfaces,
                    vlan_configs,
                    preimage_endpoint,
                    postimage_endpoint,
                )
                .unwrap();

                info!("Kickstart template successfully rendered for {}", host_name);

                let mut directory = settings().cobbler.ssh.laas_files.clone();

                directory.push_str("fedora-kickstarts/");

                let mut filename: String = "".to_string();
                filename.push_str(host_name);
                filename.push_str(".ks");

                write_file_to_cobbler(directory, filename, kickstart_template)
                    .await
                    .unwrap()
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

        context.spawn(SetPower::on(self.host_id)).join()?;

        info!(
            "set power on, now adding pxe nets so host can pxe (in time it takes for host to post)"
        );

        Ok(())
    }

    async fn set_power_off(&mut self, context: &Context, host_name: &str) -> Result<(), TaskError> {
        match self.get_distro().await? {
            Distro::Eve => {}
            _ => {
                warn!("Setting host {} power off", host_name);
                context.spawn(SetPower::off(self.host_id)).join()?;
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

        let workflow_distro = self.get_distro().await?;

        warn!("Booting host {} from disk", host_name);
        context
            .spawn(SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: match workflow_distro {
                    Distro::Eve => BootTo::SpecificDisk,
                    _ => BootTo::Disk,
                },
            })
            .join()?;

        warn!("Powering host {} on", host_name);
        context.spawn(SetPower::on(self.host_id)).join()?;
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
                    net_config: mgmt_network_config(self.host_id, &mut transaction).await,
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
        _preimage_waiter: MailboxMessageReceiver,
        mut imaging_waiter: MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        self.log(
            "Installing OS",
            "booting the installer for the base OS and provisioning the host",
            StatusSentiment::InProgress,
        )
        .await;

        let workflow_distro = self.get_distro().await?;

        match workflow_distro {
            Distro::Eve => {
                // The EVE-OS installer will turn the host off when it is done.
                // We do not have access to cloud init or late commands of any kind, so this is the working solution for now.
                // If it actually failed to install, it is highly likely that the next tasks will fail regardless.
                sleep(Duration::from_secs(4 * 60)).await;

                let host = self.fetch_instance_host().await?;

                confirm_power_state(
                    &HostConfig {
                        fqdn: host.ipmi_fqdn,
                        user: host.ipmi_user,
                        password: host.ipmi_pass,
                    },
                    &TimeoutConfig {
                        max_retries: 20,
                        retry_interval: 30,
                        timeout_duration: 600,
                    },
                    Some(Duration::from_secs(60)),
                    PowerState::Off,
                )
                .await
                .unwrap();
            }
            _ => {
                match imaging_waiter.wait_next(Duration::from_secs(60 * 35)) {
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
        let workflow_distro = self.get_distro().await?;

        match workflow_distro {
            Distro::Ubuntu => {
                self.log(
                    "Wait Host Pre-Configure",
                    "wait for host to boot into pre-configure mode and bootstrap the configuration environment",
                    StatusSentiment::InProgress,
                )
                .await;

                // This is where ubuntu waits for cloud init to finish running
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
            _ => {
                info!("No postprovision network configuration needed.")
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
        _host_name: &str,
        post_provision_waiter: &mut MailboxMessageReceiver,
    ) -> Result<(), TaskError> {
        let workflow_distro = self.get_distro().await?;

        match workflow_distro {
            Distro::Ubuntu => {
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

            _ => {
                self.log(
                    "Wait Host Online",
                    "wait for host to come online",
                    StatusSentiment::InProgress,
                )
                .await;
            }
        }

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host_public_fqdn = &self.host_id.get(&mut transaction).await.unwrap().fqdn;
        transaction.commit().await.unwrap();

        self.log(
            "Verify Host Provisioned",
            &format!("check that host is reachable at {}", host_public_fqdn),
            StatusSentiment::InProgress,
        )
        .await;

        context
            .spawn(WaitReachable {
                endpoint: host_public_fqdn.clone(),
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

        let ipmi_res = context
            .spawn(CreateIPMIAccount {
                host: self.host_id,
                password: aggregate.configuration.ipmi_password,
                username: aggregate.configuration.ipmi_username,
                userid: "4".to_string(), // TODO: look into generating this later on
            })
            .join();

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
}

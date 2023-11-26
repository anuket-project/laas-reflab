//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::tracing;

use models::{
    dal::{new_client, AsEasyTransaction, DBTable, EasyTransaction, ExistingRow, FKey},
    dashboard::{Aggregate, EasyLog, StatusSentiment},
    inventory::{BootTo, Host, HostPort},
};
use notifications::email::send_to_admins;
use serde::{Deserialize, Serialize};

use std::{
    collections::*,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use super::{set_boot::SetBoot, set_host_power_state::SetPower};
use crate::{
    deploy_booking::{
        cobbler_set_config::*,
        configure_networking::ConfigureNetworking,
        wait_host_os_reachable::WaitHostOSReachable,
    },
    resource_management::{
        cobbler::*,
        ipmi_accounts::CreateIPMIAccount,
        mailbox::Mailbox,
        network::{BondGroup, NetworkConfig, NetworkConfigBuilder, VlanConnection},
    },
    retry_for,
};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DeployHost {
    pub host_id: FKey<Host>,
    pub aggregate_id: FKey<Aggregate>,
    //pub with_config: HostConfig,
    pub using_instance: FKey<models::dashboard::Instance>,
    //pub template: FKey<Template>,
}

tascii::mark_task!(DeployHost);
impl AsyncRunnable for DeployHost {
    type Output = bool;

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        context.reset(); // we can't recover from a partial run of this

        let mut mock_waiter = Mailbox::set_endpoint_hook(self.using_instance, "mock")
            .await
            .as_taskerr()?;

        self.using_instance
            .log(
                "Pre-Provision",
                format!("waiting for mock injection"),
                StatusSentiment::in_progress,
            )
            .await;

        if let Ok(v) = mock_waiter.wait_next(Duration::from_secs(60)) {
            let val = v.msg.message;
            let val = serde_json::from_str(val.as_str());
            if let Ok(b) = val {
                match b {
                    true => return Ok(true),
                    false => return Err(TaskError::Reason(format!("mock indicated failure"))),
                }
            } else {
                // this timed out, so just continue with the booking
                self.using_instance
                    .log(
                        "Pre-Provision Done",
                        "no mock injection occurred, continuing with provision process",
                        StatusSentiment::in_progress,
                    )
                    .await;
            }
        }

        self.using_instance
            .log(
                "Provision Start",
                "a task has started running to provision the host",
                StatusSentiment::in_progress,
            )
            .await;

        self.using_instance
            .log(
                "Generating Endpoints",
                "generating http mailbox targets for host to interact with LibLaaS",
                StatusSentiment::in_progress,
            )
            .await;

        let mut preimage_waiter = Mailbox::set_endpoint_hook(self.using_instance, "pre_image")
            .await
            .as_taskerr()?;

        let mut imaging_waiter = Mailbox::set_endpoint_hook(self.using_instance, "post_image")
            .await
            .as_taskerr()?;

        let mut post_boot_waiter = Mailbox::set_endpoint_hook(self.using_instance, "post_boot")
            .await
            .as_taskerr()?;

        let mut post_provision_waiter =
            Mailbox::set_endpoint_hook(self.using_instance, "post_provision")
                .await
            .as_taskerr()?;

        let mut client = new_client().await.as_taskerr()?;
        let mut transaction = client.easy_transaction().await.as_taskerr()?;

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

        let preimage_endpoint = preimage_waiter.endpoint();
        let postimage_endpoint = imaging_waiter.endpoint();

        tracing::info!("made endpoint for host: {:?}", postimage_endpoint);

        self.using_instance
            .log(
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

        self.using_instance
            .log(
                "Powering Host Off",
                "power host off to configure boot devices",
                StatusSentiment::in_progress,
            )
            .await;

        retry_for(SetPower::off(self.host_id), context, 5, 10)?; // make sure host is off before we
                                                                 // set boot dev

        common::prelude::tokio::time::sleep(Duration::from_secs(2)).await;

        tracing::warn!("setting boot dev for {host_name} to network boot");

        self.using_instance
            .log(
                "Network Boot Configuration",
                "configuring the host to boot the installer from network",
                StatusSentiment::in_progress,
            )
            .await;

        let res = crate::retry_for(
            SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: BootTo::Network,
            },
            context,
            5,
            10,
        );

        tracing::info!("set boot res: {res:?}");

        tracing::info!("Making sure cobbler config is done");

        cobbler_config_jh.join()?;

        tracing::info!(
            "set cobbler configuration and finished mgmt net config, also set host next boot dev"
        );

        common::prelude::tokio::time::sleep(Duration::from_secs(2)).await;

        tracing::warn!("setting host {host_name} power on");

        self.using_instance
            .log(
                "Powering Host On",
                "power host on to boot the netinstall image",
                StatusSentiment::in_progress,
            )
            .await;

        retry_for(SetPower::on(self.host_id), context, 5, 10)?;

        tracing::info!(
            "set power on, now adding pxe nets so host can pxe (in time it takes for host to post)"
        );

        self.using_instance
            .log(
                "Network Backplane Configuration",
                "configuring the network backplane to allow the host to network boot",
                StatusSentiment::in_progress,
            )
            .await;

        context
            .spawn(ConfigureNetworking {
                net_config: mgmt_network_config(self.host_id, &mut transaction).await,
            })
            .join()?; // need mgmt nets set before we can try ipmi managing the host

        /*context.spawn(StashSOLOutput {
            host: self.host_id,
            instance: Some(self.using_instance),
            aggregate: Some(self.aggregate_id),
            wait: Duration::from_secs(10), // wait 2 minutes once we've applied mgmt nets so
                                            // hopefully this works
        }); // don't join, this is intentionally fallible */

        //tracing::info!("Set mgmt nets, sleeping for a bit to allow nets to stabilize before we try managing the host");

        tracing::info!(
            "successfully configured pxe networking for {:?}",
            self.host_id
        );

        tracing::warn!("wait for imaging of {host_name} to complete and mailbox to get hit"); // made it to here

        self.using_instance
            .log(
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
                    tracing::warn!("Host didn't phone home before imaging!");
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
                tracing::info!("Imaging successful!!")
            }
            Err(e) => {
                self.using_instance
                    .log(
                        "OS Install Failed",
                        "installing the OS timed out or experienced an early failure, initiating error recovery routines",
                        StatusSentiment::degraded,
                    )
                    .await;

                tracing::error!("MAILBOX FAILED with {:?}", e);
                return Err(TaskError::Reason("Mailbox error".to_owned()));
            }
        }

        finished_imaging.store(true, std::sync::atomic::Ordering::SeqCst);

        self.using_instance
            .log(
                "OS Installed",
                "the unconfigured operating system has been installed onto the host",
                StatusSentiment::in_progress,
            )
            .await;

        tracing::warn!("setting power off");
        retry_for(SetPower::off(self.host_id), context, 5, 10)?;
        tracing::warn!("set power off");

        self.using_instance
            .log(
                "Booting From Disk",
                "host is being configured to boot the now-installed operating system",
                StatusSentiment::in_progress,
            )
            .await;

        tracing::warn!("Booting from disk");
        let res = crate::retry_for(
            SetBoot {
                host_id: self.host_id,
                persistent: true,
                boot_to: BootTo::Disk,
            },
            context,
            5,
            10,
        );

        tracing::info!("set boot res: {res:?}");

        tracing::warn!("setting host power on");
        retry_for(SetPower::on(self.host_id), context, 5, 10)?;
        tracing::warn!("set power on");

        self.using_instance
            .log(
                "Wait Host Pre-Configure",
                "wait for host to boot into pre-configure mode and bootstrap the configuration environment",
                StatusSentiment::in_progress
            )
            .await;

        match post_boot_waiter.wait_next(Duration::from_secs(60 * 35)) {
            // give the host 20 minutes to boot
            // and provision
            Ok(_) => {
                // TODO: allow host to post a *failure* message, so we can detect that and save
                // those logs and force it to reboot and retry (and detect this all early)
                tracing::info!("Host came back up after imaging")
            }
            Err(e) => {
                self.using_instance
                    .log(
                        "Pre-Configure Wait Failed",
                        "host failed to boot into pre-configure mode, initiating error recovery routines",
                        StatusSentiment::degraded
                    )
                    .await;

                tracing::error!("MAILBOX FAILED with {:?}", e);
                return Err(TaskError::Reason("Mailbox error".to_owned()));
            }
        }

        self.using_instance
            .log(
                "Host Configure",
                "host completed pre-configuration, and is now applying production network config",
                StatusSentiment::in_progress,
            )
            .await;

        tracing::warn!("about to configure prod networking");

        self.using_instance
            .log(
                "Network Backplane Configuration",
                "setting up final networks within the backplane as configured for the template",
                StatusSentiment::in_progress,
            )
            .await;

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

        self.using_instance
            .log(
                "Wait Host Online",
                "wait for host to complete on-device setup steps (incl Cloud-Init)",
                StatusSentiment::in_progress,
            )
            .await;

        match post_provision_waiter.wait_next(Duration::from_secs(60 * 20)) {
            // give the host 20 minutes to boot
            // and provision
            Ok(v) => {
                // TODO: allow host to post a *failure* message, so we can detect that and save
                // those logs and force it to reboot and retry (and detect this all early)
                tracing::info!("Host came back up after applying network configs")
            }
            Err(e) => {
                self.using_instance
            .log(
                        "On-Device Setup Failed",
                        "host failed to complete on-device configuration, initiating error recovery routines",
                StatusSentiment::degraded,
                )
                .await;

                tracing::error!("MAILBOX FAILED with {:?}", e);
                return Err(TaskError::Reason("Mailbox error".to_owned()));
            }
        }

        self.using_instance
            .log(
                "Verify Host Provisioned",
                format!("check that host is reachable at {host_name}"),
                StatusSentiment::in_progress,
            )
            .await;

        context
            .spawn(WaitHostOSReachable {
                timeout: Duration::from_secs(60 * 15),
                host_id: self.host_id,
            })
            .join()?;

        tracing::warn!("Host {:?} provisioned successfully", self.host_id);

        self.using_instance
            .log(
                "Set Up IPMI Accounts",
                format!("IPMI accounts are being added to {host_name}"),
                StatusSentiment::in_progress,
            )
            .await;

        let ipmi_res = retry_for(
            CreateIPMIAccount {
                host: self.host_id,
                password: aggregate.configuration.ipmi_password,
                username: aggregate.configuration.ipmi_username,
                userid: format!("4"), // look into generating this later on
            },
            context,
            5,
            30,
        );

        if let Err(_e) = ipmi_res {
            send_to_admins(format!(
                "Failed to set up IPMI accounts for {host_name}, manual intervention required"
            ))
            .await;

            self.using_instance
            .log(
                    "Failed to Set Up IPMI",
                    format!("IPMI accounts couldn't be set up for {host_name}, \
                            the administrators have been notified and will manually set up accounts shortly"),
                            StatusSentiment::degraded,
            )
            .await;
        }

        self.using_instance
            .log(
                "Successfully Provisioned",
                format!("{host_name} has provisioned according to configuration"),
                StatusSentiment::succeeded,
            )
            .await;

        transaction.commit().await?;
        Ok(true)
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
}

pub async fn mgmt_network_config(
    host_id: FKey<Host>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    // set each iface to 98 + 99, no bond groups
    let host = host_id
        .get(t)
        .await
        .expect("host did not exist by given fk?");
    let mut builder = NetworkConfigBuilder::new();
    for port in host.ports(t).await.expect("didn't get ports?") {
        builder = builder.bond(
            BondGroup::new()
                .with_vlan(VlanConnection::from_pair(t, 99, true).await)
                .with_vlan(VlanConnection::from_pair(t, 98, false).await)
                .with_port(port.id),
        );
    }

    let v = builder.persist(false).build();

    tracing::info!("built a network config for the host: {v:#?}");

    v
}

pub async fn postprovision_network_config(
    host_id: FKey<Host>,
    aggregate_id: FKey<Aggregate>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    let networks = aggregate_id
        .get(t)
        .await
        .unwrap()
        .vlans
        .get(t)
        .await
        .unwrap()
        .into_inner();

    let mut public_vlan_id = None;

    for (net, vlan) in networks.networks {
        let net = net.get(t).await.unwrap();
        let vlan = vlan.get(t).await.unwrap();

        if net.public {
            public_vlan_id = Some(vlan.vlan_id as u16);
            break;
        }
    }

    let public_vlan_id = public_vlan_id.expect("pod contained no public networks");

    let host = host_id
        .get(t)
        .await
        .expect("host did not exist by given fk?");
    let mut builder = NetworkConfigBuilder::new();
    for port in host.ports(t).await.expect("didn't get ports?") {
        builder = builder.bond(
            BondGroup::new()
                .with_vlan(VlanConnection::from_pair(t, 99, true).await)
                .with_vlan(VlanConnection::from_pair(t, public_vlan_id, false).await)
                .with_port(port.id),
        );
    }

    let v = builder.persist(false).build();

    tracing::info!("built a network config for the host: {v:#?}");

    v
}

pub async fn empty_network_config(
    host_id: FKey<Host>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    let host = host_id
        .get(t)
        .await
        .expect("host did not exist by given fk?");
    let mut builder = NetworkConfigBuilder::new();
    for port in host.ports(t).await.expect("didn't get ports?") {
        builder = builder.bond(
            BondGroup::new()
                .with_vlan(VlanConnection::from_pair(t, 99, true).await)
                .with_port(port.id),
        );
    }

    let v = builder.persist(true).build();

    tracing::info!("built a network config for the host: {v:#?}");

    v
}

async fn prod_network_config(
    host_id: FKey<Host>,
    deployed_as: FKey<models::dashboard::Instance>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    async fn bg_config_to_bg(
        t: &mut EasyTransaction<'_>,
        bgc: &models::dashboard::BondGroupConfig,
        interfaces: &HashMap<String, HostPort>,
        networks: &models::dashboard::NetworkAssignmentMap,
    ) -> BondGroup {
        let mut bg = BondGroup::new();

        tracing::info!("Translating bg_config to bg. Bgc is {bgc:#?} while interfaces is {interfaces:#?} and networks is {networks:#?}");

        for port in bgc.member_interfaces.iter() {
            bg = bg.with_port(interfaces.get(port).unwrap().id);
        }

        for vcc in bgc.connects_to.iter() {
            let vlan = *networks.networks.get(&vcc.network).unwrap();
            let vconn = VlanConnection {
                vlan,
                tagged: vcc.tagged,
            };

            bg = bg.with_vlan(vconn);
        }

        // Make sure to keep the 99/ipmi connection on all bondgroups/ports
        bg = bg.with_vlan(VlanConnection {
            vlan: models::inventory::Vlan::select()
                .where_field("vlan_id")
                .equals(99 as i16)
                .run(t)
                .await
                .expect("need at least the ipmi vlan, hardcode requirement")
                .get(0)
                .unwrap()
                .id,
            tagged: true,
        });

        bg
    }

    let h: ExistingRow<Host> = host_id.get(t).await.unwrap();

    let instance = deployed_as.get(t).await.unwrap();

    let network_assignments = instance.network_data.get(t).await.unwrap();

    let mut builder = NetworkConfigBuilder::new();

    let mut configured_ports = HashSet::new();

    for bg_config in instance.config.connections.iter() {
        let ports_by_name: HashMap<String, HostPort> = h
            .ports(t)
            .await
            .expect("didn't get ports")
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();

        let bg = bg_config_to_bg(t, bg_config, &ports_by_name, &network_assignments).await;

        for port in bg.member_host_ports.iter() {
            configured_ports.insert(*port);
        }

        builder = builder.bond(bg);
    }

    // set the ports that aren't configured to have nothing on them
    for port in h.ports(t).await.unwrap() {
        if !configured_ports.contains(&port.id) {
            let bg = BondGroup::new().with_port(port.id);
            // don't give the bg any vlans
            builder = builder.bond(bg);
        }
    }

    builder.persist(true).build()
}

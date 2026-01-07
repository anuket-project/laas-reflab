use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    mem::swap,
    sync::atomic::{AtomicBool, AtomicU32},
    time::Duration,
};

use common::prelude::{itertools::Itertools, parking_lot::Mutex, *};

pub mod deploy_host;
pub mod notify;
pub mod reachable;
pub mod set_boot;
pub mod set_host_power_state;
pub mod grub;
pub mod ssh_server_up;

use config::Situation;

use dal::{new_client, AsEasyTransaction, EasyTransaction, FKey, NewRow, ID};
use macaddr::MacAddr6;
use maplit::hashmap;

use metrics::{MetricHandler, ProvisionMetric, Timestamp};
use models::{
    allocator::{AllocationReason, ResourceHandle, ResourceHandleInner},
    dashboard::{
        self, Aggregate, BondGroupConfig, BookingMetadata, HostConfig, Instance, LifeCycleState,
        Network, NetworkAssignmentMap, StatusSentiment, Template, VlanConnectionConfig,
    },
    inventory::{Flavor, Host, IPInfo, IPNetwork, Vlan},
    EasyLog,
};
use notifications::email::send_to_admins;
use serde_yaml::{to_value, Mapping, Value};
use tracing::info;

use crate::{
    deploy_booking::deploy_host::DeployHost,
    resource_management::{allocator::*, mailbox::Mailbox, vpn::SyncVPN},
};
use serde::{Deserialize, Serialize};

use tascii::prelude::*;
use tracing::log::warn;
use users::ipa;

use crate::resource_management::allocator;

use self::notify::Notify;

tascii::mark_task!(BookingTask);
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct BookingTask {
    pub aggregate_id: FKey<Aggregate>,
}

impl AsyncRunnable for BookingTask {
    type Output = ();

    fn summarize(&self, id: ID) -> String {
        format!("BookingTask with id {id}")
    }

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        info!("Starting Task: BookingTask");
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let agg = self.aggregate_id.get(&mut transaction).await.unwrap();

        let mut single_host_deploy_tasks = Vec::new();

        for config in agg.instances(&mut transaction).await.unwrap().into_iter() {
            tracing::info!("Doing a single host deploy: {config:?}");
            let single = SingleHostDeploy {
                instance: config.id,
                for_aggregate: agg.id,
            };

            single_host_deploy_tasks.push(context.spawn(single));
        }

        let vpn_succeeded = context
            .spawn(SyncVPN {
                users: agg.users.to_owned(),
            })
            .join();

        if let Err(e) = vpn_succeeded {
            send_to_admins(format!("Failed to sync vpn, error: {e:?}")).await;
        }

        let mut results = Vec::new();

        for single_host_deploy_task in single_host_deploy_tasks {
            results.push(single_host_deploy_task.join());
        }

        tracing::info!("VPN config succeeded, hosts have all provisioned, now notify users their booking is done");

        if !results.iter().any(|one| one.is_err()) {
            // notify booking done, since everything is committed and saved
            let notify = context.spawn(Notify {
                aggregate: self.aggregate_id,
                situation: Situation::BookingCreated,
                extra_context: vec![],
            });

            // make sure it finishes before we return
            let notifications_succeeded = notify.join();

            if let Err(e) = notifications_succeeded {
                send_to_admins(format!("Failed to notify users, error: {e:?}")).await;
            }

            // mark the aggregate as done provisioning
            let mut agg = self.aggregate_id.get(&mut transaction).await?;
            agg.state = LifeCycleState::Active; // finished provisioning
            agg.update(&mut transaction).await?;

            transaction.commit().await.unwrap();

            Ok(())
        } else {
            for handle in results.clone() {
                match handle {
                    Ok(s) => tracing::info!("Succeeded in provisioning {s}"),
                    Err(e) => {
                        send_to_admins(format!("Failed to provision a host for a deployment (aggregate id {:?}), error encountered: {e:?}", self.aggregate_id)).await;
                        tracing::error!("Failed to provision a host, error: {e:?}")
                    }
                }
            }

            if results.iter().all(|one| one.is_err()) {
                tracing::info!("All hosts failed to provision, deallocating the aggregate");
                Allocator::instance()
                    .deallocate_aggregate(&mut transaction, self.aggregate_id)
                    .await?;
                let mut agg = self.aggregate_id.get(&mut transaction).await?;
                agg.state = LifeCycleState::Done;
                agg.update(&mut transaction).await?;
            }

            transaction.commit().await.unwrap();

            Err(TaskError::Reason(
                "Failed to provision some host".to_string(),
            ))
        }
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("BookingTask").versioned(1)
    }

    fn timeout() -> std::time::Duration {
        let estimated_overhead = Duration::from_secs(5 * 60);
        SingleHostDeploy::overall_timeout() + SyncVPN::overall_timeout() + Notify::overall_timeout() + estimated_overhead
    }

    fn retry_count() -> usize {
        0
    }
}

tascii::mark_task!(AllocateHostTask);
#[derive(Clone, Serialize, Deserialize, Debug, Hash)]
pub struct AllocateHostTask {
    flavor: FKey<Flavor>,
    for_aggregate: FKey<Aggregate>,
    instance: FKey<Instance>,
}

impl AsyncRunnable for AllocateHostTask {
    type Output = (FKey<Host>, ResourceHandle);

    fn summarize(&self, id: ID) -> String {
        format!("AllocateHostTask with id {id}")
    }

    async fn execute_task(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;

        let res = allocator::Allocator::instance()
            .allocate_host(
                &mut transaction,
                self.flavor,
                self.for_aggregate,
                AllocationReason::ForBooking,
                false,
            )
            .await;

        match res {
            Ok(v) => {
                let host = v.0.get(&mut transaction).await.unwrap();

                transaction
                    .commit()
                    .await
                    .expect("Couldn't commit alloc of host");

                self.instance
                    .log(
                        "Allocation Complete",
                        format!(
                            "{} has been allocated to be configured as this resource",
                            host.server_name
                        ),
                        StatusSentiment::InProgress,
                    )
                    .await;

                tracing::info!("Allocation task allocated host {}", host.server_name);

                Ok(v)
            }
            Err(e) => {
                self.instance
                    .log(
                        "Allocation Failed",
                        "No resource was presently available to perform this role".to_string(),
                        StatusSentiment::Degraded,
                    )
                    .await;

                Err(TaskError::Reason(format!(
                    "Couldn't allocate the asked-for resource, for reason: {e:?}"
                )))
            }
        }
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("AllocationTask").versioned(1)
    }

    fn timeout() -> Duration {
        Duration::from_secs(5 * 60)
    }

    fn retry_count() -> usize {
        0
    }
}

tascii::mark_task!(SingleHostDeploy);
#[derive(Clone, Serialize, Deserialize, Debug, Hash)]
/// Task responsible for allocating and provisioning a single host within a template.
struct SingleHostDeploy {
    instance: FKey<Instance>,
    for_aggregate: FKey<Aggregate>,
}

impl AsyncRunnable for SingleHostDeploy {
    type Output = String;

    fn timeout() -> Duration {

        let estimated_overhead = Duration::from_secs(5 * 60);

        (DeployHost::overall_timeout() * 3) + (AllocateHostTask::overall_timeout() * 3) + estimated_overhead
    }

    fn summarize(&self, id: ID) -> String {
        format!("SingleHostDeploy with id {id}")
    }

    /// SingleHostDeploy should not rely on the native tascii retry loop as it needs to handle errors very differently.
    fn retry_count() -> usize {
        0
    }

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        tracing::info!("doing a SingleHostDeploy for instance: {:?}", self.instance);

        // Total number of hosts this task is willing to try to allocate and provision before giving up
        // If all hosts fail, they are all freed as the issue is likely due to infrastructure or the template config.
        // If only some fail but one eventually succeeds, the failed hosts are marked as not working.
        let max_hosts_to_try = 3;

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();
        let host_config = self
            .instance
            .get(&mut transaction)
            .await
            .unwrap()
            .config
            .clone();

        let mut maybe_bad_hosts: Vec<ResourceHandle> = Vec::new();

        transaction.commit().await.unwrap();
        for _task_retry_no in 0..max_hosts_to_try {
            match context
                .spawn(AllocateHostTask {
                    instance: self.instance,
                    for_aggregate: self.for_aggregate,
                    flavor: host_config.flavor,
                })
                .join()
            {
                Ok((host, rh)) => {
                    let mut transaction = client.easy_transaction().await.unwrap();

                    let mut inst = self.instance.get(&mut transaction).await.unwrap();
                    inst.linked_host = Some(host); // so cleanup knows what to clean up

                    inst.update(&mut transaction)
                        .await
                        .expect("Couldn't update instance with ci file");

                    transaction.commit().await.unwrap();

                    let start_time = Timestamp::now();
                    let deploy_host_result = context
                        .spawn(DeployHost {
                            host_id: host,
                            aggregate_id: self.for_aggregate,
                            using_instance: self.instance,
                            distribution: None,
                        })
                        .join();

                    let provisioning_time_seconds = start_time.elapsed();
                    send_provision_metric(
                        &inst.config.hostname,
                        &self.for_aggregate,
                        provisioning_time_seconds,
                        deploy_host_result.is_ok(),
                    )
                    .await;
                    match deploy_host_result {
                        Ok(_) => {
                            tracing::warn!(
                                "{:?} Bad Hosts: {:?}",
                                maybe_bad_hosts.len(),
                                maybe_bad_hosts
                            );
                            mark_not_working(maybe_bad_hosts, self.for_aggregate).await;

                            tracing::info!("Provisioned a host successfully");

                            return Ok("successfully provisioned".to_owned());
                        }
                        Err(_) => {
                            self.instance
                                .log(
                                    "Failed to Provision",
                                    "Failed to provision this host too many times, \
                                                    trying again with a different host",
                                    StatusSentiment::Degraded,
                                )
                                .await;

                            maybe_bad_hosts.push(rh.clone());

                            send_to_admins(format!(
                                "Failure to provision a host for instance {:?}",
                                self.instance
                            ))
                            .await;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("failed to allocate a host?");

                    self.instance
                        .log(
                            "Failed to Allocate",
                            "No hosts were available to fill this request",
                            StatusSentiment::Failed,
                        )
                        .await;

                    free_hosts(maybe_bad_hosts, self.for_aggregate).await;

                    let mut transaction = client.easy_transaction().await.unwrap();
                    let profile = self
                        .instance
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
                        "Failure to allocate a host for instance {:?}, this is of profile {profile}",
                        self.instance
                    ))
                    .await;

                    transaction.commit().await.unwrap();
                    return Err(TaskError::Reason(format!(
                        "failed to allocate a host, because: {e:?}"
                    )));
                }
            }
        }

        // if we tried *this many times* without a success,
        // we should set the hosts that we tried as unallocated
        // as there is probably a problem with the booking itself
        // rather than just problems with the individual hosts
        free_hosts(maybe_bad_hosts, self.for_aggregate).await;

        send_to_admins(format!(
            "Failure to provision instance {:?}, config may be faulty",
            self.instance
        ))
        .await;

        self.instance
            .log(
                "Failed to Provision",
                "failed to provision using this config too many times, \
                an administrator has been notified and will attend to your booking shortly",
                StatusSentiment::Failed,
            )
            .await;

        Err(TaskError::Reason(
            "failed to provision too many times".to_owned(),
        ))
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("SingleHostDeployTask").versioned(1)
    }
}

/// Call this upon a failure that indicates a problem with the booking
/// rather than the hosts themselves
async fn free_hosts(hosts: Vec<ResourceHandle>, agg: FKey<Aggregate>) {
    // we intentionally create our own client here since the hosts have been
    // allocated through allocatehosttask, our wrapping task has its own
    // transaction that it  may roll back but we want to guarantee we free the hosts
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();
    let allocator = allocator::Allocator::instance();

    for handle in hosts {
        let res = (allocator.deallocate_host(&mut transaction, handle, agg)).await;
        match res {
            Ok(_) => {
                tracing::info!("Successfully deallocated a host");
            }
            Err(e) => {
                tracing::error!("Couldn't free resources attempted to allocate, error: {e:?}")
            }
        }
        // don't wait for a response, it may never come, this is just a best effort to free them
        // if sending the message fails, we still want to continue and try again
    }

    transaction
        .commit()
        .await
        .expect("Couldn't commit freeing of hosts");
}

/// Call this in the event that provisioning succeeded with one
/// host but failed with others, passing the hosts it failed on
///
/// That indicates there is a problem with the hosts that it couldn't
/// provision on
async fn mark_not_working(hosts: Vec<ResourceHandle>, original_agg: FKey<Aggregate>) {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();
    let allocator = allocator::Allocator::instance();
    let lab = original_agg
        .get(&mut transaction)
        .await
        .expect("Expected to get original aggregate")
        .lab;

    if !hosts.is_empty() {
        let agg = Aggregate {
            id: FKey::new_id_dangling(),
            deleted: false,
            users: vec![],
            vlans: NewRow::new(NetworkAssignmentMap::empty())
                .insert(&mut transaction)
                .await
                .unwrap(),
            template: NewRow::new(Template {
                id: FKey::new_id_dangling(),
                name: String::from("bad hosts"),
                deleted: false,
                description: String::from("bad hosts"),
                owner: None,
                public: false,
                networks: vec![],
                hosts: vec![],
                lab,
            })
            .insert(&mut transaction)
            .await
            .unwrap(),
            metadata: BookingMetadata {
                booking_id: None,
                owner: None,
                lab: None,
                purpose: Some(String::from("Hold bad hosts")),
                project: None,
                details: None,
                start: None,
                end: None,
            },
            state: LifeCycleState::Active,
            configuration: dashboard::AggregateConfiguration {
                ipmi_username: String::new(),
                ipmi_password: String::new(),
            },
            lab,
        };

        let agg_id = NewRow::new(agg.clone())
            .insert(&mut transaction)
            .await
            .unwrap();

        let mut host_names = Vec::new();

        for handle in hosts {
            match allocator
                .deallocate_host(&mut transaction, handle.clone(), original_agg)
                .await
            {
                Ok(_v) => {
                    // now allocate it again
                    if let ResourceHandleInner::Host(h) = handle.tracks {
                        let host = h.get(&mut transaction).await.unwrap();

                        host_names.push(host.server_name.clone());

                        let res = allocator
                            .allocate_specific_host(
                                &mut transaction,
                                h,
                                agg_id,
                                AllocationReason::ForMaintenance,
                            )
                            .await;

                        if let Err(e) = res {
                            tracing::error!("Couldn't allocate host to a maintenance booking, error: {e:?}, host: {h:?}");
                        }
                    } else {
                        tracing::error!("Told to dealloc a host, but it wasn't a host");
                    }
                }
                Err(e) => {
                    tracing::error!("Issue: couldn't dealloc {handle:?} when marking hosts not working, err was: {e:?}");
                }
            }
        }

        transaction.commit().await.unwrap();

        send_to_admins(format!(
            "Host(s) {} failed to provision. Added to maintenance booking, Aggregate ID is {}",
            host_names.join(", "),
            agg,
        ))
        .await;
    }
}

pub async fn generate_cloud_config(
    conf: HostConfig,
    host_id: FKey<Host>,
    instance_id: FKey<Instance>,
    aggregate_id: FKey<Aggregate>,
    transaction: &mut EasyTransaction<'_>,
) -> Result<String, anyhow::Error> {
    tracing::info!("Generating cloud config");

    let mut cloud_config = serde_yaml::Mapping::new();

    // Build Map
    cloud_config.insert(
        "users".into(),
        ci_serialize_users(transaction, conf.clone(), host_id, aggregate_id).await,
    );
    cloud_config.insert("hostname".into(), conf.clone().hostname.into());
    cloud_config.insert(
        "runcmd".into(),
        ci_serialize_runcmds(
            transaction,
            conf.clone(),
            instance_id,
            host_id,
            aggregate_id,
        )
        .await,
    );
    cloud_config.insert(
        "system_info".into(),
        ci_serialize_sysinfo(transaction, conf.clone(), host_id, aggregate_id),
    );

    // Serialize to a YAML String
    let yaml = serde_yaml::to_string(&cloud_config).expect("Expected to convert to string.");
    tracing::info!("Made cloud config cloud-config:\n{yaml}");
    Ok(format!("#cloud-config\n{yaml}"))

    // TODO - output the yaml string to a file in the db that can be read later. Return a handle or something that allows us to find that yaml file
}

async fn ci_serialize_users(
    transaction: &mut EasyTransaction<'_>,
    _conf: HostConfig,
    _host_id: FKey<Host>,
    aggregate_id: FKey<Aggregate>,
) -> Value {
    let aggregate = aggregate_id.get(transaction).await.unwrap();
    let mut ipa = ipa::IPA::init()
        .await
        .expect("Expected to initialize IPA connection");

    let mut user_list: Vec<Value> = vec![Value::String("default".into())];

    let mut users: Vec<(String, ipa::User)> = vec![];
    for username in aggregate.users.iter() {
        let res = ipa.find_matching_user(username.clone(), true, false).await;

        // need to do this in serial here since ipa client is not interior mutable, so
        // no way to parallelize the awaits as they all borrow the client mutably

        match res {
            Ok(v) => users.push((username.clone(), v)),
            Err(e) => {
                panic!("{e}");
            }
        }
    }

    for user_data in users {
        let mut user_dict: Mapping = Mapping::new();
        user_dict.insert("name".into(), Value::String(user_data.1.uid.clone()));
        user_dict.insert("lock_passwd".into(), false.into());
        user_dict.insert("groups".into(), "sudo".into());
        user_dict.insert("sudo".into(), "ALL=(ALL) NOPASSWD:ALL".into());
        let mut authorized_keys: Vec<String> = Vec::new();

        let user = ipa
            .find_matching_user(user_data.1.uid.clone(), true, false)
            .await;

        let username = user_data.0;

        match user {
            Ok(user) => match user.ipasshpubkey {
                Some(k) => {
                    authorized_keys.append(&mut k.clone());
                }
                None => {
                    warn!("User '{username}' had no ssh public key on file");
                }
            },
            Err(e) => {
                tracing::error!(
                    "User lookup failed for collaborator '{username}', the error was {e:?}"
                )
            }
        }

        user_dict.insert("ssh_authorized_keys".into(), authorized_keys.into());
        user_list.push(user_dict.into());
    }

    // Value to be returned should be a list that contains "default" and then a dict for each user
    user_list.into()
}

async fn render_nmcli_commands(
    transaction: &mut EasyTransaction<'_>,
    conf: HostConfig,
    nm: NetworkAssignmentMap,
    host_id: FKey<Host>,
    aggregate_id: FKey<Aggregate>,
) -> Vec<String> {
    let host = host_id.get(transaction).await.unwrap();
    let _aggregate = aggregate_id.get(transaction).await.unwrap();

    let connections = conf.connections.clone();

    let mut sync_nm = HashMap::new();

    for (n, vl) in nm.networks.iter() {
        let vl = vl.get(transaction).await.unwrap().into_inner();
        let net = n.get(transaction).await.unwrap().into_inner();
        sync_nm.insert(n, (net, vl));
    }

    let hostports = host.ports(transaction).await.unwrap();

    let id = AtomicU32::new(0);
    let next_vif_id = || id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let _rand_mac = |for_dev: String| {
        // Locally administered macs, using `01` prefix
        // generate the other bytes randomly
        let addr = MacAddr6::new(
            0x01,
            rand::random(),
            rand::random(),
            rand::random(),
            rand::random(),
            rand::random(),
        );
        tracing::info!("Assigns mac addr {addr:?} to device {for_dev} while generating CI for host {host_id:?} within agg {aggregate_id:?}");

        addr.to_string()
    };

    fn uuid_of_connection(connection_name: &str) -> String {
        format!("$(nmcli -t -f connection.uuid con show {connection_name} | sed 's/connection.uuid://g')")
    }

    let commands = RefCell::new(Vec::new());
    let interfaces = RefCell::new(HashSet::new());

    let command = |v: String| {
        commands.borrow_mut().push(v);
    };

    let _add_interface = |v: String| {
        interfaces.borrow_mut().insert(v);
    };

    // select a device to use for the "default" route
    let mut default_iface_candidates = connections
        .iter()
        .filter(|bgc| {
            bgc.member_interfaces.len() == 1
                && bgc.connects_to.iter().any(|vl| {
                    tracing::warn!("vcc is {vl:?}");
                    tracing::warn!("network is {:?}", vl.network.clone());
                    tracing::warn!("sync nm is {:?}", sync_nm.clone());

                    sync_nm
                        .get(&vl.network)
                        .expect("Expected to find network in NetworkAssignmentMap")
                        .1
                        .public_config
                        .as_ref()
                        .map(|pc| pc.v4.is_some() || pc.v6.is_some())
                        .unwrap_or(false)
                        && !vl.tagged // for now try to prefer untagged ones so long as they
                                      // exist, to sidestep some dns issues
                })
        })
        .map(|bgc| bgc.member_interfaces.iter().next().unwrap().clone())
        .collect_vec();

    default_iface_candidates.sort();

    let _created_default_interface = AtomicBool::new(false);

    // take care of hostname setting so `sudo` doesn't take forever:
    let _hostname = &conf.hostname;
    let host_ident = host.server_name.clone();

    let public_config = |cfg: Option<IPNetwork>, _interface: &str| {
        let cfg = cfg.unwrap_or(IPNetwork { v4: None, v6: None });
        let v4 = if let Some(v) = cfg.v4 {
            let IPInfo {
                subnet: _,
                netmask: _,
                gateway,
                provides_dhcp,
            } = v;
            if provides_dhcp {
                let mut root = String::new();

                if let Some(_gw) = gateway {
                    warn!("not applying gateway without having a static ip");
                };

                root = format!("{root} ipv4.method auto ipv4.dhcp-hostname {host_ident}");

                root
            } else {
                todo!("manual address assignment")
            }
        } else {
            "ipv4.method disabled".to_string()
        };

        let v6 = if let Some(_v) = cfg.v6 {
            todo!("ipv6 support")
        } else {
            "ipv6.method disabled".to_string()
        };

        format!("{v4} {v6}")
    };

    let render_vlan = |config: &VlanConnectionConfig,
                       vlan: &Vlan,
                       network: &Network,
                       root_dev: &str,
                       root_dev_short: &str| {
        if !config.tagged {
            unreachable!("tried to make a tagged conn from an untagged vlan conn")
        }

        let network_name = network.name.as_str();
        let vif_id = next_vif_id();
        let vlan_id = vlan.vlan_id;

        let iface_nmid = format!("tagged-{network_name}-{root_dev_short}.{vlan_id}");
        let mut iface_name = format!("{network_name:.3}{vif_id}v{vlan_id}");
        // making a vlan v-if
        iface_name.truncate(15);

        let root_dev_id = uuid_of_connection(root_dev);

        command(format!(
            "nmcli con add type vlan \
              connection.id {iface_nmid} \
              ifname {iface_name} \
              vlan.id {vlan_id} \
              dev {root_dev_id} \
              connection.autoconnect yes"
        ));

        iface_nmid
    };

    let render_root = |config: &BondGroupConfig| {
        let BondGroupConfig {
            connects_to,
            member_interfaces,
        } = config.clone();

        let member_interfaces = member_interfaces.into_iter().collect_vec();

        let base_iface = match member_interfaces.as_slice() {
            [] => {
                if !connects_to.is_empty() {
                    tracing::error!("Weirdness: empty bondgroup in {connections:?} but has vlans {connects_to:?}");
                }

                None
            }
            [one] => {
                // one interface, all the vlans go off of it, no need for bondgroup complexity
                // TODO: allow supporting port types other than ethernet (like infiniband, wifi)

                // remove any existing one by this name,
                // avoid default mucking things up
                command(format!("nmcli con del {one}"));
                command(format!("ip link set dev {one} down"));

                command(format!(
                    "nmcli con add type ethernet connection.id {one} \
                    connection.interface-name {one} ipv4.method disabled ipv6.method disabled"
                ));

                Some(one.clone())
            }
            more => {
                // multiple interfaces, need to bond them together to run vlans over

                // TODO: allow naming bondgroups (pipe through front end)
                let vif_id = next_vif_id();
                let bond_nmid = format!("link-agg-{vif_id}");

                command(format!(
                    "nmcli con add type bond connection.id {bond_nmid} \
                    bond.options \"mode=balance-rr\" \
                    ipv4.method disabled ipv6.method disabled"
                ));

                for iface in more {
                    command(format!("nmcli con del {iface}"));
                    command(format!("ip link set dev {iface} down"));

                    let uuid_of_bond = uuid_of_connection(&bond_nmid);

                    command(format!(
                        "nmcli con add type ethernet ifname {iface} master {uuid_of_bond}"
                    ));
                }

                Some(bond_nmid)
            }
        };

        if let Some(b) = base_iface {
            if connects_to.iter().filter(|vcc| !vcc.tagged).count() > 1 {
                tracing::error!("Multiple untagged vlans connected to some bondgroup!");
            }

            // If this port has an untagged connection
            let (name, short_name) = if let Some(v) = connects_to.iter().find(|vcc| !vcc.tagged) {
                let (net, vlan) = sync_nm
                    .get(&v.network)
                    .expect("ref to non existent network");

                // figure out whether we should have default route through this

                let rename = format!("untagged-{}-{}.{}", net.name, b, vlan.vlan_id);
                command(format!("nmcli con mod {b} con-name {rename}"));

                if vlan.public_config.is_some() {
                    let pc = public_config(vlan.public_config.clone(), &rename);

                    command(format!(
                        "nmcli con mod {rename} {pc} connection.autoconnect yes"
                    ));
                } else {
                    // no public config, so this interface can be left largely unconfigured
                };

                (rename, b.clone())
            } else {
                (b.clone(), b.clone())
            };

            for tagged_vl in connects_to.into_iter().filter(|vcc| vcc.tagged) {
                let (net, vlan) = sync_nm
                    .get(&tagged_vl.network)
                    .expect("ref to non existent network");

                // make a bridge for the vlan iface to live under
                let br_vif_id = next_vif_id();

                let _br_ifname = format!("br{br_vif_id}");
                let _br_nmid = format!("tagged-{}-{}.{}", b, net.name, vlan.vlan_id);

                /*command(format!(
                    "nmcli con add type bridge connection.id {br_nmid} \
                    ipv4.method disabled ipv6.method disabled"
                ));*/

                let pc = public_config(vlan.public_config.clone(), &name);

                let vlan_nmid = render_vlan(&tagged_vl, vlan, net, &name, &short_name);

                command(format!("nmcli con mod {vlan_nmid} {pc}"));
            }
        }
    };

    // more work to try to get host into a canonical state
    // so we aren't fighting with existing defaults anywhere
    for hostport in hostports.iter() {
        let pn = &hostport.name;
        command(format!("ip link set dev {pn} down"));
        command(format!("nmcli con del {pn}"));
    }

    command("sleep 10".to_string());

    // clear entire routing table
    command("ip route flush default".to_string());
    command("ip route flush 0/0".to_string());

    command("sleep 5".to_string());

    // emit vdev configuration commands
    for bg in connections.iter() {
        render_root(bg);
    }

    // initial try bringup, gets everything mostly in place
    command("systemctl restart NetworkManager".to_string());

    command("sleep 10".to_string());

    // flush the defroutes that got created during the initial apply (these do not persist)
    command("ip route flush default".to_string());

    command("sleep 5".to_string());

    // re-apply the necessary defroutes
    command("systemctl restart NetworkManager".to_string());

    commands.into_inner()
}

fn val<V: Serialize>(v: V) -> serde_yaml::Value {
    serde_yaml::to_value(v).unwrap()
}

#[allow(dead_code)]
async fn ci_serialize_netconf(
    transaction: &mut EasyTransaction<'_>,
    conf: HostConfig,
    nm: NetworkAssignmentMap,
    host_id: FKey<Host>,
    aggregate_id: FKey<Aggregate>,
) -> Value {
    // Generate network configs
    let host = host_id.get(transaction).await.unwrap();
    let aggregate = aggregate_id.get(transaction).await.unwrap();
    let project = aggregate.lab;
    let project_config = config::settings()
        .projects
        .get(
            &project
                .get(transaction)
                .await
                .expect("Expected to find agg")
                .name,
        )
        .expect("no matching project for aggregate");
    let search_domains = project_config.search_domains.clone();
    let nameservers = project_config.nameservers.clone();

    let connections = conf.connections.clone();

    let mut cfgd_bridges = HashMap::new();
    let mut cfgd_vlans = HashMap::new();
    let mut cfgd_bondgroups = HashMap::new();
    let mut cfgd_ethernets = HashMap::new();

    let mut sync_nm = HashMap::new();

    for (n, vl) in nm.networks.iter() {
        let vl = vl.get(transaction).await.unwrap().into_inner();
        let net = n.get(transaction).await.unwrap().into_inner();
        sync_nm.insert(n, (net, vl));
    }

    let rand_mac = |for_dev: String| {
        // Locally administered macs, using `01` prefix
        // generate the other bytes randomly
        let addr = MacAddr6::new(
            0x00,
            rand::random(),
            rand::random(),
            rand::random(),
            rand::random(),
            rand::random(),
        );
        tracing::info!("Assigns mac addr {addr:?} to device {for_dev} while generating CI for host {host_id:?} within agg {aggregate_id:?}");

        addr.to_string()
    };

    fn val<V: Serialize>(v: V) -> serde_yaml::Value {
        serde_yaml::to_value(v).unwrap()
    }

    let mut id = 0;
    let mut next_vif_id = || {
        let r = id;
        id += 1;

        r
    };

    let mut connect_to = |root_interface: String, vlans: &HashSet<VlanConnectionConfig>| {
        for vlan_conn_cfg in vlans {
            let (network, vlan) = sync_nm
                .get(&vlan_conn_cfg.network)
                .cloned()
                .expect("netmap didn't account for all vlans");

            let mut config = HashMap::new();

            // TODO: we could actually do static IP assignment instead
            // of relying on DHCP here, eval whether this would be desired
            match vlan.public_config {
                Some(cfg) => {
                    if let Some(cfgv4) = cfg.v4 {
                        let IPInfo {
                            subnet: _,
                            netmask: _,
                            gateway: _,
                            provides_dhcp,
                        } = cfgv4;
                        config.insert(val("dhcp4"), val(provides_dhcp));
                        //config.insert(val("gateway4"), val(gateway.unwrap()));
                        config.insert(
                            val("nameservers"),
                            val(hashmap! {
                                val("search") => val(search_domains.clone()),
                                val("addresses") => val(nameservers.clone()),
                            }),
                        );
                    } else {
                        info!("No v4 config for {vlan_conn_cfg:?}");
                    }

                    if let Some(cfgv6) = cfg.v6 {
                        let IPInfo {
                            subnet: _,
                            netmask: _,
                            gateway,
                            provides_dhcp,
                        } = cfgv6;
                        config.insert(val("dhcp6"), val(provides_dhcp));
                        config.insert(val("gateway6"), val(gateway.unwrap()));
                    } else {
                        info!("No v6 config for {vlan_conn_cfg:?}");
                    }
                }
                None => {
                    config.insert(val("dhcp4"), val(false));
                    // don't emit anything for now
                }
            };

            let network_name = network.name;

            if !vlan_conn_cfg.tagged {
                // making a bridge
                let mut name = format!("{root_interface}n{network_name}");
                name.truncate(15);
                config.insert(val("interfaces"), val(vec![root_interface.clone()]));
                config.insert(val("macaddress"), val(rand_mac(name.clone())));
                /*config.insert(val("match"), val(hashmap! {
                    val("match")
                }))*/

                cfgd_bridges.insert(name, config);
            } else {
                let vlan_id = vlan.vlan_id;

                let vif_id = next_vif_id();

                let mut name = format!("{network_name:.6}{vif_id}v{vlan_id}");
                // making a vlan v-if
                name.truncate(15);
                config.insert(val("macaddress"), val(rand_mac(name.clone())));
                config.insert(val("link"), val(root_interface.clone()));
                config.insert(val("id"), val(vlan_id));

                cfgd_vlans.insert(name, config);
            }
        }
    };

    for (bg_idx, bgc) in connections.clone().into_iter().enumerate() {
        let BondGroupConfig {
            connects_to,
            member_interfaces,
        } = bgc;

        let interfaces: Vec<String> = member_interfaces.into_iter().collect_vec();

        let _public_vlan = connects_to.iter().find(|vl| !vl.tagged).cloned();

        match interfaces.as_slice() {
            [] => {
                // ignore this bg
                if !connects_to.is_empty() {
                    tracing::error!("Weirdness: empty bondgroup in {connections:?} but has vlans {connects_to:?}");
                }
            }
            [one] => {
                // use the interface directly as a base
                connect_to(one.clone(), &connects_to);
            }
            more => {
                // create a bondgroup with all the member interfaces,
                // TODO: check validity (no reuse of interfaces)
                // Connect the vlans to it
                let mut name = format!("bond{bg_idx}");
                name.truncate(15);

                let bond_config = hashmap! {
                    val("interfaces") => {
                        val(more)
                    }
                };

                cfgd_bondgroups.insert(name.clone(), val(bond_config));

                connect_to(name, &connects_to);
            }
        }
    }

    for host_port in host
        .ports(transaction)
        .await
        .expect("couldn't get ports for host")
    {
        let mut port_dict = HashMap::new();
        port_dict.insert(val("macaddress"), val(host_port.mac.to_string()));

        cfgd_ethernets.insert(host_port.name.clone(), port_dict);
    }

    let config_dict = hashmap! {
        val("ethernets") => val(cfgd_ethernets),
        val("bonds") => val(cfgd_bondgroups),
        val("bridges") => val(cfgd_bridges),
        val("vlans") => val(cfgd_vlans),
        val("version") => val(2),
        val("renderer") => val("networkd"),
    };

    // Value to be retuned should be a dict of {'version' : 1, 'config': config_arr}
    val(config_dict)
}

async fn ci_serialize_runcmds(
    transaction: &mut EasyTransaction<'_>,
    conf: HostConfig,
    instance_id: FKey<Instance>,
    host_id: FKey<Host>,
    aggregate_id: FKey<Aggregate>,
) -> Value {
    let nm = aggregate_id
        .get(transaction)
        .await
        .unwrap()
        .vlans
        .get(transaction)
        .await
        .unwrap()
        .into_inner();
    // Generate runcmd configs

    let commands = Mutex::new(Vec::new());
    let command = |v: serde_yaml::Value| {
        commands.lock().push(v);
    };

    command(val("sleep 30"));

    let host = host_id.get(transaction).await.unwrap();

    let image_name = conf
        .image
        .get(transaction)
        .await
        .unwrap()
        .name
        .clone()
        .to_lowercase();

    #[derive(Copy, Clone)]
    enum ImageVariant {
        Ubuntu,
        Fedora,
        Unknown,
    }

    // TODO: make this more robust, maybe add additional fields to images for this info
    let variant = if image_name.contains("ubuntu") {
        ImageVariant::Ubuntu
    } else if image_name.contains("fedora") {
        ImageVariant::Fedora
    } else {
        ImageVariant::Unknown
    };

    // first bring up mgmt networking
    command(val("echo 'Running dhclient on ports'".to_string()));
    if let Some(p) = host.ports(transaction).await.unwrap().into_iter().next() {
        let pn = &p.name;
        command(val(format!("echo 'doing dhclient {pn}'")));
        command(val(format!("sudo dhclient {pn} || true")));
    } else {
        let host_name = &host.server_name;
        command(val(
            "echo 'There are no ports to run dhclient on!'".to_string()
        ));
        tracing::error!(
            "Network config for {image_name} on {host_name} may fail, host had no ports"
        );
    }

    // command(val(format!("echo 'trying to run dhclient'")));
    // command(val(format!("sudo dhclient")));
    // command(val(format!("echo 'tried to run dhclient'")));

    let base_host = url::Url::parse(&config::settings().mailbox.external_url).ok();
    if let Some(v) = base_host.as_ref().and_then(|v| v.host()) {
        tracing::info!("Going to hit host at to check up {v}");
        command(val(format!("while ! ping -c 1 -W 1 {v}; do echo 'waiting for networking to come up before installing packages' && sleep 10; done")));
    }

    // on ubuntu, we need to install NetworkManager first
    if let ImageVariant::Ubuntu = variant {
        command(val("echo 'Running apt -y update'".to_string()));
        command(val("sleep 2".to_string()));
        command(val("sudo apt -y update"));

        // command(val(format!("echo 'Running apt -y upgrade'")));
        // command(val(format!("sleep 2")));
        // command(val("sudo apt -y upgrade"));

        command(val(
            "echo 'Running apt -y --fix-missing install network-manager'".to_string(),
        ));
        command(val("sleep 2".to_string()));
        command(val("sudo apt -y --fix-missing install network-manager"));

        command(val("echo 'Verifying nmcli install...'".to_string()));
        command(val("nmcli --version".to_string()));
        command(val("sleep 2".to_string()));

        command(val("echo 'Running apt -y install curl'".to_string()));
        command(val("sleep 2".to_string()));
        command(val("sudo apt -y install curl || true"));
    }

    // we've installed the packages we need to configure before going dark,
    // so we can phone home and go dark while we set up final networking

    if let Ok(ep) = Mailbox::get_endpoint_hook(instance_id, "post_boot").await {
        let url = ep.to_url();
        tracing::info!("Adding an endpoint hook to ci file, hook url is {url}");

        //command_list.push(val("))
        let curl_cmd = format!(
            r#"curl -X POST -H "Content-Type: application/json" {url}/push -d '{{"success": true}}'"#
        );

        tracing::info!("Sets curl cmd to {curl_cmd}");

        // do the first phone home
        command(val(curl_cmd));
    } else {
        tracing::error!("No post-install hook found for host {}", host.server_name);
    }

    // now go dark
    if let ImageVariant::Ubuntu = variant {
        command(val("echo 'Going dark...'".to_string()));
        command(val("sleep 3".to_string()));
        command(val("sudo systemctl disable systemd-networkd || true"));
        command(val("sudo systemctl stop systemd-networkd || true"));
        command(val("sudo rm -rf /etc/netplan || true"));
    }

    command(val("echo 'Killing dhclient'".to_string()));
    command(val("sleep 5".to_string()));
    command(val("sudo killall dhclient || true"));

    command(val("echo 'Attempting to start NetworkManager'".to_string()));
    command(val("sleep 5".to_string()));
    command(val("sudo systemctl enable NetworkManager || true"));
    command(val("sudo systemctl start NetworkManager || true"));

    let hostname = conf.hostname.clone();
    command(val(format!("echo '127.0.0.1 {hostname}' >> /etc/hosts")));

    // clear out the existing configs from NM
    command(val(r#"nmcli --terse --fields=name connection show | while read name; do nmcli connection delete "$name"; done || true"#.to_string()));

    // tell ubuntu we want to manage all interfaces
    command(val(
        "touch /etc/NetworkManager/conf.d/10-globally-managed-devices.conf || true".to_string(),
    ));

    if let ImageVariant::Ubuntu = variant {
        // fully turn off systemd-networkd
        command(val(
            "systemctl stop systemd-networkd.socket systemd-networkd || true".to_string(),
        ));
        /*command(val(format!(
            "networkd-dispatcher systemd-networkd-wait-online || true"
        )));*/
        command(val(
            "systemctl disable systemd-networkd.socket systemd-networkd ||true".to_string(),
        ));
        /*command(val(format!(
            "networkd-dispatcher systemd-networkd-wait-online || true"
        )));*/
    }

    // disable the auto-default dev creation, configure other parts of NM
    command(val(
        "rm -rf /etc/NetworkManager/NetworkManager.conf || true".to_string(),
    ));
    let append = |file, content| {
        command(val(format!("echo '{content}' >> {file}")));
    };

    for line in [
        "[main]",
        "plugins=ifupdown,keyfile",
        "no-auto-default=*",
        "[ifupdown]",
        "managed=false",
        "[device]",
    ] {
        append("/etc/NetworkManager/NetworkManager.conf", line);
    }

    command(val("systemctl restart NetworkManager".to_string()));

    // now do platform-agnostic (ish) nmcli commands
    for cmd in render_nmcli_commands(transaction, conf, nm, host_id, aggregate_id).await {
        command(val(format!("{cmd} || true")));
    }

    // wait for networking to come up after that
    if let Some(v) = base_host.as_ref().and_then(|v| v.host()) {
        tracing::info!("Going to hit host at to check up {v}");
        command(val("sleep 30"));
        command(val(format!("while ! ping -c 1 -W 1 {v}; do echo 'waiting for networking to come up after configuring production networks' && sleep 10; done || true")));
    }

    // do final phone home
    if let Ok(ep) = Mailbox::get_endpoint_hook(instance_id, "post_provision").await {
        let url = ep.to_url();
        tracing::info!("Adding an endpoint hook to ci file, hook url is {url}");

        let curl_cmd = format!(
            r#"curl -X POST -H 'Content-Type: application/json' {url}/push -d '{{"success": true}}'"#
        );

        tracing::info!("Sets curl cmd to {curl_cmd}");

        // do the first phone home
        command(val(curl_cmd));
    } else {
        tracing::error!("No post-provision hook found for host {}", host.server_name);
    }

    let commands = {
        let mut r = Vec::new();
        swap(&mut *commands.lock(), &mut r);

        r
    };

    to_value(commands).unwrap()
}

fn ci_serialize_sysinfo(
    _transaction: &mut EasyTransaction<'_>,
    _conf: HostConfig,
    _host_id: FKey<Host>,
    _aggregate_id: FKey<Aggregate>,
) -> Value {
    let m: HashMap<usize, Value> = hashmap! {};

    to_value(m).unwrap()
}

async fn send_provision_metric(
    host_name: &str,
    aggregate: &FKey<Aggregate>,
    duration: u64,
    success: bool,
) {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();

    let aggregate = aggregate.get(&mut transaction).await.unwrap();

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
        tracing::error!("Failed to send provision metric: {:?}", e);
    } else {
        tracing::trace!("Provision metric sent successfully");
    }
}

use std::
    time::Duration
;

use common::prelude::*;

pub mod cloud_init;
pub mod deploy_host;
pub mod grub;
pub mod notify;
pub mod reachable;
pub mod set_boot;
pub mod set_host_power_state;
pub mod ssh_server_up;

use config::Situation;

use dal::{new_client, AsEasyTransaction, FKey, NewRow, ID};

use metrics::{MetricHandler, ProvisionMetric, Timestamp};
use models::{
    allocator::{AllocationReason, ResourceHandle, ResourceHandleInner},
    dashboard::{
        self, Aggregate, BookingMetadata, Instance, LifeCycleState, NetworkAssignmentMap,
        StatusSentiment, Template,
    },
    inventory::{Flavor, Host},
    EasyLog,
};
use notifications::email::send_to_admins;
use tracing::info;

use crate::{
    deploy_booking::deploy_host::DeployHost,
    resource_management::{allocator::*, vpn::SyncVPN},
};
use serde::{Deserialize, Serialize};

use tascii::prelude::*;

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
        hostname: Some(host_name.to_string()).filter(|s| !s.is_empty()),
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
        project: aggregate.metadata.project.clone().filter(|s| !s.is_empty()),
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

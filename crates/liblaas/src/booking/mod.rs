use chrono::Utc;
use common::prelude::chrono::Days;
use common::prelude::*;
use dal::{new_client, AsEasyTransaction, EasyTransaction, FKey, NewRow};
use metrics::prelude::*;

use models::{
    allocator::{Allocation, AllocationReason},
    dashboard::{
        Aggregate, AggregateConfiguration, BookingMetadata, HostConfig, Instance, InstanceProvData,
        LifeCycleState, NetworkAssignmentMap, ProvEvent, StatusSentiment,
    },
    inventory::Lab,
};

use std::collections::HashMap;
use workflows::resource_management::{
    allocator::Allocator,
    ipmi_accounts::{generate_password, generate_username},
}; //, ResourceHandle, AggregateID, ResourceHandleInner};

use workflows::entry::*;

use crate::web::api;

pub async fn make_aggregate(blob: api::BookingBlob) -> Result<FKey<Aggregate>, anyhow::Error> {
    let mut client = new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    let prov_data: Vec<InstanceProvData> = Vec::new();
    let template = blob
        .template_id
        .get(&mut transaction)
        .await
        .expect("couldn't get template for booking blob");

    let netmap = NewRow::new(NetworkAssignmentMap::empty())
        .insert(&mut transaction)
        .await?;

    let _ = config::settings()
        .projects
        .get(&blob.origin)
        .ok_or(anyhow::Error::msg(format!(
            "no project supported by origin name {}",
            &blob.origin
        )))?;

    let now = Utc::now();

    let booking_id: i32 = blob
        .metadata
        .booking_id
        .clone()
        .unwrap_or_default()
        .parse()
        .unwrap_or_default();

    let booking = BookingMetric {
        booking_id,
        booking_length_days: blob.metadata.length.unwrap_or_default() as i32,
        num_hosts: template.hosts.len() as i32,
        num_collaborators: blob
            .allowed_users
            .iter()
            .filter(|x| x != &&blob.metadata.owner.clone().unwrap_or_default())
            .count() as i32,
        owner: blob.metadata.owner.clone().unwrap_or("None".to_string()),
        lab: blob.origin.clone(),
        project: blob.metadata.project.clone().unwrap_or("None".to_string()),
        purpose: blob.metadata.purpose.clone().unwrap_or("None".to_string()),
        details: blob.metadata.details.clone().unwrap_or("None".to_string()),
        mock: false,

        // defaults to current time.
        ..Default::default()
    };

    match MetricHandler::send(booking) {
        Ok(_) => {
            tracing::info!("Sent booking metric");
        }
        Err(e) => {
            tracing::error!("Failed to send booking metric with error {}", e)
        }
    }

    let agg = NewRow::new(Aggregate {
        state: LifeCycleState::New,
        lab: Lab::get_by_name(&mut transaction, blob.origin.clone())
            .await
            .expect("Expected to find lab")
            .expect("Expected lab to exist")
            .id,
        id: FKey::new_id_dangling(),
        users: blob.allowed_users,
        vlans: netmap,
        deleted: false,
        template: blob.template_id,
        configuration: AggregateConfiguration {
            ipmi_username: generate_username(10),
            ipmi_password: generate_password(15),
        },
        metadata: BookingMetadata {
            booking_id: blob.metadata.booking_id,
            owner: blob.metadata.owner,
            lab: blob.metadata.lab,
            purpose: blob.metadata.purpose,
            project: blob.metadata.project,
            details: blob.metadata.details,
            start: Some(now),
            end: blob.metadata.length.map(|l| now + Days::new(l)),
        },
    })
    .insert(&mut transaction)
    .await
    .expect("couldn't create the aggregate")
    .get(&mut transaction)
    .await
    .unwrap();

    let allocator = Allocator::instance();

    // try alloc, bailing out if this aggregate could not possibly be deployed (also letting any
    // acquired vlans roll back as we unwind)
    {
        let mut ct = transaction.easy_transaction().await?;
        let mut to_free = Vec::new();

        for inst in template.hosts.iter() {
            let hn = &inst.hostname;
            let h = allocator
                .allocate_host(
                    &mut ct,
                    inst.flavor,
                    agg.id,
                    AllocationReason::ForBooking,
                    true,
                )
                .await
                .map_err(|_| {
                    anyhow::Error::msg(format!("no host was available to fill the role of {hn}"))
                })?;

            to_free.push(h);
        }

        for (host, handle) in to_free {
            Allocator::instance()
                .deallocate_host(&mut ct, handle, agg.id)
                .await?;
        }

        // rollback if we can to not clutter allocation table (remember, transaction
        // is all or nothing, so we could end up with the first part but not the last part!)
        ct.rollback().await.unwrap();
    };

    // release those allocations
    for mut allocation in Allocation::all_for_aggregate(&mut transaction, agg.id).await? {
        allocation.ended = Some(Utc::now());
        allocation.update(&mut transaction).await?;
    }

    allocator
        .allocate_vlans_for(&mut transaction, agg.id, template.networks.clone(), netmap)
        .await?;

    for host_config in template.hosts.clone() {
        // create instance from config
    }

    for config in template.hosts.clone() {
        tracing::debug!("got config_info {config:?}");

        let mut instance = InstanceProvData {
            hostname: config.hostname.clone(),
            flavor: config.flavor,
            image: String::from(""),
            cifile: Vec::new(),
            ipmi_create: true,
            networks: Vec::new(),
        };

        // Resource processing:
        // Hardware
        hardware_conf(&mut transaction, &mut instance, config.clone()).await;
        // CI Files
        ci_processing(&mut transaction, &mut instance, config.clone()).await;
        // Finalize
        // Push prov data to vec
        let inst_id = FKey::new_id_dangling();

        let instance = Instance {
            metadata: HashMap::new(),
            aggregate: agg.id,
            id: inst_id,
            within_template: template.id,
            config: config.clone(),
            network_data: agg.vlans,
            linked_host: None,
        };

        let inst_fk = NewRow::new(instance).insert(&mut transaction).await?;

        let _ = Instance::log(
            inst_fk,
            &mut transaction,
            ProvEvent::new(
                "Pre-Provision",
                "Configuration has been created, host not yet selected",
            ),
            Some(StatusSentiment::Unknown),
        )
        .await;
    }

    transaction.commit().await?;

    // Ask tascii to provision the host
    let res = DISPATCH
        .get()
        .unwrap()
        .send(Action::DeployBooking { agg_id: agg.id });
    if let Err(e) = res {
        tracing::error!("Failed to send deploy task with error {:#?}", e)
    }

    Ok(agg.id)
}

async fn hardware_conf(
    t: &mut EasyTransaction<'_>,
    instance: &mut InstanceProvData,
    conf: HostConfig,
) {
    instance.image = conf.image.get(t).await.unwrap().name.clone();
    instance.hostname = conf.hostname;
    instance.flavor = conf.flavor;
    instance.ipmi_create = true;
}

async fn ci_processing(
    t: &mut EasyTransaction<'_>,
    instance: &mut InstanceProvData,
    conf: HostConfig,
) {
    for c in conf.cifile.clone() {
        instance.cifile.push(c.get(t).await.unwrap().into_inner())
    }
}

/// Attempts to end a booking. A booking can only be ended if the aggregate lifecycle state is "Active".
/// Does not validate the cleanup aggregate task result.
pub async fn end_booking(agg_id: FKey<Aggregate>) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await?;

    let agg = agg_id.get(&mut transaction).await?;

    match agg.state {
        LifeCycleState::Active => {
            let sender = DISPATCH.get().unwrap();
            let dispatch_result = sender.send(Action::CleanupBooking { agg_id });

            match dispatch_result {
                Ok(_) => Ok(()),
                Err(_) => Err(anyhow::anyhow!("Failed to dispatch end booking job!")),
            }
        }
        LifeCycleState::New => Err(anyhow::anyhow!(
            "Cannot end booking while still provisioning!"
        )),
        LifeCycleState::Done => {
            // Failed bookings are set to "Done"
            Ok(())
        }
    }
}

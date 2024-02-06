//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::*;

use chrono::Utc;
use common::prelude::chrono::Days;
use models::{
    dal::{
        new_client,
        AsEasyTransaction,
        EasyTransaction,
        FKey,
        NewRow,
    },
    dashboard,
    dashboard::{
        Aggregate,
        AggregateConfiguration,
        BookingMetadata,
        HostConfig,
        Instance,
        InstanceProvData,
        NetworkAssignmentMap,
        ProvEvent,
        StatusSentiment,
    }, inventory::Lab,
};


use models::allocation::{self, *};

use std::collections::HashMap;
use workflows::resource_management::{
    allocator::Allocator,
    ipmi_accounts::{generate_password, generate_username},
}; //, ResourceHandle, AggregateID, ResourceHandleInner};

use axum::extract::Json;


use workflows::entry::*;

use crate::web::api;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}

pub async fn make_aggregate(
    blob: api::BookingBlob,
) -> Result<FKey<dashboard::Aggregate>, anyhow::Error> {
    let mut client = new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    let prov_data: Vec<dashboard::InstanceProvData> = Vec::new();
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
    let agg = NewRow::new(Aggregate {
        state: dashboard::LifeCycleState::New,
        lab: Lab::get_by_name(&mut transaction, blob.origin.clone()).await.expect("Expected to find lab").expect("Expected lab to exist").id,
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
            start: Some(now.clone()),
            end: match blob.metadata.length {
                Some(l) => Some(now + Days::new(l)),
                None => None,
            },
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
    let _ = {
        let mut ct = transaction.easy_transaction().await?;
        let mut to_free = Vec::new();

        for inst in template.hosts.iter() {
            let hn = &inst.hostname;
            let h = allocator
                .allocate_host(
                    &mut ct,
                    inst.flavor,
                    agg.id,
                    AllocationReason::ForBooking(),
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
    for mut allocation in
        allocation::Allocation::all_for_aggregate(&mut transaction, agg.id).await?
    {
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

        let mut instance = dashboard::InstanceProvData {
            hostname: String::from(config.hostname.clone()),
            flavor: config.flavor.clone(),
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
            Some(StatusSentiment::unknown),
        )
        .await;
    }

    transaction.commit().await?;

    // Ask tascii to provision the host
    let res = DISPATCH
        .get()
        .unwrap()
        .send(Action::DeployBooking { agg_id: agg.id });
    match res {
        Err(e) => {
            tracing::error!("Failed to send deploy task with error {:#?}", e)
        }
        Ok(_) => {}
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

pub fn end_booking(Json(agg_id): Json<FKey<Aggregate>>) {
    DISPATCH
        .get()
        .unwrap()
        .send(Action::CleanupBooking { agg_id })
        .expect("Expected to dispatch end booking job");
}

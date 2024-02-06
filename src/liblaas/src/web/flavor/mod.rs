//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::collections::HashMap;

use crate::web::{
    api::{AggregateDescription, AllocationBlob, HostBlob, ImageBlob},
    WebError,
};

use super::api::FlavorBlob;
use aide::{
    axum::{
        routing::{get, get_with},
        ApiRouter,
    },
    transform::TransformOperation,
};
use axum::{
    extract::{Json, Path},
    http::StatusCode,
};
use common::prelude::{itertools::Itertools, *};
use models::{
    allocation::{self, AllocationReason, ResourceHandle},
    dal::*,
    dashboard::Image,
    inventory::*,
};
use tracing::debug;

use llid::LLID;
use models::{
    dal::{new_client, web::*, AsEasyTransaction, FKey, ID},
    dashboard,
    inventory,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use workflows::resource_management::allocator;

use super::{
    api::{self, InterfaceBlob},
    AppState,
};

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FlavorResponse {
    id: LLID,
    name: String,
    description: String,
    interface_names: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct IfaceDetails {
    name: String,
    speed: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FlavorDetails {
    name: String,
    cpu_count: usize,
    ram: u64,
    disk_size: u64,
    ifaces: Vec<IfaceDetails>,
}

pub async fn list_flavors(Path(lab_name): Path<String>) -> Result<Json<Vec<FlavorBlob>>, WebError> {
    let res = {
        tracing::info!("API call to list_flavors()");
        let mut client = new_client().await.expect("Expected to connect to db");
        let mut transaction = client
            .easy_transaction()
            .await
            .expect("Transaction creation error");
        let mut fbs: Vec<FlavorBlob> = Vec::new();

        let flavors = Flavor::select().run(&mut transaction).await.unwrap();

        let lab = match Lab::get_by_name(&mut transaction, lab_name.clone()).await {
            Ok(lab_option) => match lab_option {
                Some(l) => l.id,
                None => return Err((StatusCode::NOT_FOUND, format!("Failed to find lab"))),
            },
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to retrieve lab: {e}"))),
        };

        let hosts = allocator::Allocator::instance()
            .get_free_hosts(&mut transaction, lab)
            .await
            .unwrap();

        let mut available_count = HashMap::new();

        for (host, _) in hosts {
            let flavor = host.flavor;
            let ent = available_count.entry(flavor).or_insert(0);
            *ent += 1;
        }

        for f in flavors {
            let ports = f.ports(&mut transaction).await.unwrap();
            tracing::debug!("Got first set of ports");
            let images = dashboard::Image::images_for_flavor(&mut transaction, f.id, None)
                .await
                .expect("couldn't fetch images for a flavor");
            let images = images
                .into_iter()
                .map(|img| ImageBlob {
                    image_id: img.id,
                    name: img.name,
                })
                .collect_vec();

            let ports = ports
                .into_iter()
                .map(|er| {
                    let iface = er.into_inner();
                    InterfaceBlob {
                        name: iface.name,
                        speed: iface.speed,
                        cardtype: iface.cardtype,
                    }
                })
                .collect_vec();

            // get available count using allocator
            let count = available_count.get(&f.id).copied().unwrap_or(0);

            let fb = FlavorBlob {
                flavor_id: f.id,
                name: f.name.clone(),
                interfaces: ports,
                images,
                available_count: count,
                cpu_count: f.cpu_count,
                ram: f.ram.clone(),
                root_size: f.root_size.clone(),
                disk_size: f.disk_size.clone(),
                swap_size: f.swap_size.clone(),
            };

            if Host::select().where_field("flavor").equals(f.id).run(&mut transaction).await.expect("Expected to find host").get(0).unwrap().projects.get(0).unwrap().clone() == lab_name.clone() {
                fbs.push(fb);
            }
        }

        transaction.commit().await.expect("didn't commit?");

        Json(fbs)
    };

    tracing::debug!("Exit from client drop");

    Ok(res)
}

fn list_flavors_docs(op: TransformOperation) -> TransformOperation {
    op.description("Lists flavor id and names available to the user.")
        .response::<200, Json<Vec<(String, String)>>>()
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct ListFlavorsRequest {
    user_id: i64,
    flavor_id: LLID,
}

/// List hosts, filtering to only hosts for the given project (dashboard)
pub async fn list_hosts(Path(lab_name): Path<String>) -> Result<Json<Vec<api::HostBlob>>, WebError> {
    tracing::info!("API call to list_hosts()");
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;

    let hosts = inventory::Host::all_hosts(&mut transaction)
        .await
        .log_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to retrieve all hosts",
            true,
        )?;

    let mut blobs = Vec::new();

    for host in hosts {
        let host = host.into_inner();
        if !(ResourceHandle::handle_for_host(&mut transaction, host.id).await.expect("Expected lab to exist").lab.unwrap().get(&mut transaction).await.expect("Expected lab to exist").name == lab_name) {
            continue;
        }

        let handle = allocation::ResourceHandle::handle_for_host(&mut transaction, host.id).await;

        let handle = if let Ok(h) = handle {
            h
        } else {
            tracing::error!("Didn't find a handle for {host:?}");
            continue;
        };

        let allocation = if let Ok(Some(a)) =
            allocation::Allocation::find(&mut transaction, handle.id, false)
                .await
                .map(|v| v.into_iter().next())
        {
            let a = a.into_inner();

            let agg_desc = if let Some(agg) = a.for_aggregate {
                let agg = agg.get(&mut transaction).await;

                if let Ok(agg) = agg {
                    let agg = agg.into_inner();
                    Some(AggregateDescription {
                        id: agg.id,
                        purpose: agg.metadata.purpose,
                        project: agg.metadata.project,
                        origin: agg.lab.get(&mut transaction).await.expect("Expected lab to exist").name.clone(),
                    })
                } else {
                    None
                }
            } else {
                None
            };

            let reason = match a.reason_started {
                AllocationReason::ForBooking() => "booked",
                AllocationReason::ForRetiry() => "retired",
                AllocationReason::ForMaintenance() => "maintenance",
            };

            Some(AllocationBlob {
                for_aggregate: agg_desc,
                reason: reason.to_owned(),
            })
        } else {
            None
        };

        let hb = HostBlob {
            id: Some(host.id),
            name: host.server_name,
            arch: host.arch.to_string(),
            flavor: host.flavor,
            ipmi_fqdn: host.ipmi_fqdn,
            allocation,
        };

        blobs.push(hb);
    }

    transaction.commit().await.log_db_client_error()?;

    Ok(Json(blobs))
}

pub fn routes(_state: AppState) -> ApiRouter {
    return ApiRouter::new()
        .api_route("/:lab_name", get(list_flavors))
        .api_route("/:lab_name/hosts", get(list_hosts));
}

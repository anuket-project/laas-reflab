//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use super::api::FlavorBlob;
use super::{
    api::{self, InterfaceBlob},
    AppState,
};
use crate::web::{
    api::{AggregateDescription, AllocationBlob, HostBlob, ImageBlob},
    WebError,
};
use aide::{
    axum::{routing::get, ApiRouter},
    transform::TransformOperation,
};
use axum::{
    extract::{Json, Path},
    http::StatusCode,
};
use common::prelude::{itertools::Itertools, *};
use dal::{web::*, *};
use models::{
    allocator::{Allocation, AllocationReason, ResourceHandle},
    dashboard::Image,
    inventory::{Flavor, Host, Lab},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;
use workflows::resource_management::allocator;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FlavorResponse {
    id: Uuid,
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

pub async fn fetch_lab_by_name(
    transaction: &mut EasyTransaction<'_>,
    lab_name: String,
) -> Result<ExistingRow<Lab>, WebError> {
    match Lab::get_by_name(transaction, lab_name).await {
        Ok(lab_option) => match lab_option {
            Some(l) => Ok(l),
            None => Err((StatusCode::NOT_FOUND, "Failed to find lab".to_string())),
        },
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to retrieve lab: {e}"),
        )),
    }
}

async fn fetch_available_hosts_per_flavor(
    transaction: &mut EasyTransaction<'_>,
    lab_id: FKey<Lab>,
) -> Result<HashMap<FKey<Flavor>, usize>, WebError> {
    let hosts = allocator::Allocator::instance()
        .get_free_hosts(transaction, lab_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get free hosts: {e}"),
            )
        })?;

    let mut available_count = HashMap::new();
    for (host, _) in hosts {
        *available_count.entry(host.flavor).or_insert(0) += 1;
    }

    Ok(available_count)
}

async fn build_flavor_blobs(
    transaction: &mut EasyTransaction<'_>,
    flavors: Vec<Flavor>,
    available_count: HashMap<FKey<Flavor>, usize>,
    lab_name: String,
) -> Result<Vec<FlavorBlob>, WebError> {
    let mut fbs: Vec<FlavorBlob> = Vec::new();

    for f in flavors {
        let hosts = Host::select()
            .where_field("flavor")
            .equals(f.id)
            .where_field("projects")
            .equals(json!([lab_name]))
            .run(transaction)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to query hosts for flavor {}: {}", f.name, e),
                )
            })?;

        if !hosts.is_empty() {
            let interfaces: Vec<_> = f
                .ports(transaction)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to get ports: {e}"),
                    )
                })?
                .into_iter()
                .map(|er| {
                    let iface = er.into_inner();
                    InterfaceBlob {
                        name: iface.name,
                        speed: iface.speed,
                        cardtype: iface.cardtype,
                    }
                })
                .collect();

            let images: Vec<_> = Image::images_for_flavor(transaction, f.id, None)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to get images: {e}"),
                    )
                })?
                .into_iter()
                .map(|img| ImageBlob {
                    image_id: img.id,
                    name: img.name,
                })
                .collect();

            fbs.push(FlavorBlob {
                flavor_id: f.id,
                name: f.name,
                interfaces,
                images,
                available_count: available_count.get(&f.id).copied().unwrap_or(0),
                cpu_count: f.cpu_count,
                ram: f.ram,
                root_size: f.root_size,
                disk_size: f.disk_size,
                swap_size: f.swap_size,
            });
        }
    }

    Ok(fbs)
}

pub async fn list_flavors(Path(lab_name): Path<String>) -> Result<Json<Vec<FlavorBlob>>, WebError> {
    tracing::info!("API call to list_flavors()");
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;

    let lab = fetch_lab_by_name(&mut transaction, lab_name.clone()).await?;

    let available_count = fetch_available_hosts_per_flavor(&mut transaction, lab.id).await?;

    let flavors_row = Flavor::select().run(&mut transaction).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to retrieve flavors: {e}"),
        )
    })?;

    let flavors = flavors_row
        .into_iter()
        .map(|row| row.into_inner())
        .collect();

    let flavor_blobs =
        build_flavor_blobs(&mut transaction, flavors, available_count, lab_name).await?;

    transaction.commit().await.log_db_client_error()?;

    Ok(Json(flavor_blobs))
}

fn list_flavors_docs(op: TransformOperation) -> TransformOperation {
    op.description("Lists flavor id and names available to the user.")
        .response::<200, Json<Vec<(String, String)>>>()
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct ListFlavorsRequest {
    user_id: i64,
    flavor_id: Uuid,
}

/// List hosts, filtering to only hosts for the given project (dashboard)
pub async fn list_hosts(
    Path(lab_name): Path<String>,
) -> Result<Json<Vec<api::HostBlob>>, WebError> {
    tracing::info!("API call to list_hosts()");
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;

    let hosts = Host::all_hosts(&mut transaction).await.log_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to retrieve all hosts",
        true,
    )?;

    let mut blobs = Vec::new();

    for host in hosts {
        let host = host.into_inner();
        if ResourceHandle::handle_for_host(&mut transaction, host.id)
            .await
            .expect("Expected lab to exist")
            .lab
            .get(&mut transaction)
            .await
            .expect("Expected lab to exist")
            .name
            != lab_name
        {
            continue;
        }

        let handle = ResourceHandle::handle_for_host(&mut transaction, host.id).await;

        let handle = if let Ok(h) = handle {
            h
        } else {
            tracing::error!("Didn't find a handle for {host:?}");
            continue;
        };

        let allocation = if let Ok(Some(a)) = Allocation::find(&mut transaction, handle.id, false)
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
                        origin: agg
                            .lab
                            .get(&mut transaction)
                            .await
                            .expect("Expected lab to exist")
                            .name
                            .clone(),
                    })
                } else {
                    None
                }
            } else {
                None
            };

            let reason = match a.reason_started {
                AllocationReason::ForBooking => "booked",
                AllocationReason::ForRetiry => "retired",
                AllocationReason::ForMaintenance => "maintenance",
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

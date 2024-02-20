//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{aide::axum::routing::post, itertools::Itertools, *};
use models::dashboard::{AggregateConfiguration, Instance, StatusSentiment};

use super::{api, AppState, WebError};
use crate::{booking, booking::make_aggregate};
use aide::axum::{
    routing::{delete, get},
    ApiRouter,
};
// this is evil v  absolutely awful
//use anyhow::Ok;
use axum::{
    extract::{Json, Path},
    http::StatusCode,
};

use llid::LLID;
use models::{
    dal::{new_client, web::*, AsEasyTransaction, ExistingRow, FKey},
    dashboard::{self, Aggregate, ProvisionLogEvent},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;

pub mod host;

use host::{instance_power_control, instance_power_state};

#[axum::debug_handler]
async fn create_booking(
    Json(agg): Json<api::BookingBlob>,
) -> Result<Json<FKey<dashboard::Aggregate>>, WebError> {
    tracing::info!("API call to create_booking()");
    let agg = make_aggregate(agg)
        .await
        .log_server_error("unable to create the aggregate/booking", true)?;

    Ok(Json(agg))
}

async fn end_booking(Path(agg_id): Path<FKey<Aggregate>>) {
    booking::end_booking(axum::Json(agg_id))
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssignedHostInfo {
    hostname: String,
    ipmi_fqdn: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InstanceStatus {
    instance: FKey<Instance>,
    logs: Vec<InstanceStatusUpdate>,
    assigned_host_info: Option<AssignedHostInfo>,
    host_alias: String,

    #[deprecated]
    /// field, please reference assigned_host_info instead (if available)
    assigned_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusInfo {
    headline: String,
    subline: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InstanceStatusUpdate {
    pub status_info: StatusInfo,
    pub sentiment: StatusSentiment,
    pub time: String,

    #[deprecated]
    /// use status_info instead
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct BookingStatus {
    // map from <assigned hostname> to <list of status objects>
    instances: HashMap<FKey<Instance>, InstanceStatus>,
    config: AggregateConfiguration,
}

async fn booking_status(Path(agg_id): Path<LLID>) -> Result<Json<BookingStatus>, WebError> {
    tracing::debug!("API call to booking_status()");
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;
    // instance id, instance hostname, status

    let agg: ExistingRow<dashboard::Aggregate> = models::dal::FKey::from_id(agg_id.into())
        .get(&mut transaction)
        .await
        .log_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to look up aggregate by given ID",
            true,
        )?;

    let mut statuses = HashMap::new();

    for instance in &agg
        .instances(&mut transaction)
        .await
        .log_db_client_error()?
    {
        let mut logs_for_instance =
            ProvisionLogEvent::all_for_instance(&mut transaction, instance.id)
                .await
                .log_db_client_error()?;

        logs_for_instance.sort_by_key(|v| v.time);

        let inst_hn = instance.config.hostname.clone();

        let (assigned_host, assigned_host_info) = if let Some(v) = instance.linked_host {
            let host = v
                .get(&mut transaction)
                .await
                .log_db_client_error()?
                .into_inner();

            let host_info = AssignedHostInfo {
                hostname: host.server_name.clone(),
                ipmi_fqdn: host.ipmi_fqdn,
            };

            (Some(host.server_name), None)
        } else {
            (None, None)
        };

        #[allow(deprecated)] // deprecated on front end, but we need to keep back-compat
        let logs = logs_for_instance
            .into_iter()
            .map(|log| InstanceStatusUpdate {
                sentiment: log.sentiment,

                status: log.prov_status.to_string(),
                status_info: StatusInfo {
                    headline: log.prov_status.event.clone(),
                    subline: log.prov_status.details.clone(),
                },
                time: log.time.to_rfc2822(),
            })
            .collect_vec();

        #[allow(deprecated)] // deprecated on front end, but we need to keep back-compat
        let inst_stat = InstanceStatus {
            instance: instance.id,
            assigned_host_info,
            assigned_host,
            host_alias: inst_hn,
            logs,
        };

        statuses.insert(instance.id, inst_stat);
    }

    transaction.commit().await.log_db_client_error()?;

    Ok(Json(BookingStatus {
        instances: statuses,
        config: agg.configuration.clone(),
    }))
}

pub fn routes(state: AppState) -> ApiRouter {
    ApiRouter::new() // remember that in order to have the Handler trait, all inputs for
        // a handler need to implement FromRequest, and all outputs need to implement IntoResponse
        .route("/:agg_id/end", delete(end_booking))
        .route("/:agg_id/status", get(booking_status))
        .route("/create", post(create_booking))
        .route(
            "/ipmi/:instance_id/powerstatus",
            axum::routing::get(instance_power_state),
        )
        .route(
            "/ipmi/:instance_id/setpower",
            axum::routing::post(instance_power_control),
        )
}

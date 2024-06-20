//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use self::host::fetch_ipmi_fqdn;
use super::{api, AppState, WebError};
use crate::{booking, booking::make_aggregate};
use aide::{axum::{routing::{delete, get}, ApiRouter}, OperationIo};
use axum::{
    extract::{Json, Path},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use common::prelude::{aide::axum::routing::post, itertools::Itertools, *};
use host::{instance_power_control, instance_power_state};
use models::dashboard::{AggregateConfiguration, Instance, StatusSentiment, Template};
use models::{
    dal::{new_client, web::*, AsEasyTransaction, ExistingRow, FKey},
    dashboard::{self, Aggregate, ProvisionLogEvent},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use workflows::entry::DISPATCH;
use std::collections::HashMap;
use uuid::Uuid;

pub mod host;

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

#[axum::debug_handler]
async fn end_booking(Path(agg_id): Path<FKey<Aggregate>>) -> Json<EndBookingResponse> {
    tracing::info!("Received call to end booking for {:?}", agg_id);
    match booking::end_booking(agg_id).await {
        Ok(_) => {
            Json(EndBookingResponse { success: true, details: format!("Successfully ended booking with agg_id {:?}", agg_id)})
        },
        Err(error) => {
            Json(EndBookingResponse { success: false, details: format!("{}", error.to_string())})
        },
    }
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
    template: Template,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, OperationIo)]
pub struct EndBookingResponse {
    pub success: bool,
    pub details: String,
}

async fn booking_status(Path(agg_id): Path<Uuid>) -> Result<Json<BookingStatus>, WebError> {
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

    let template = agg
        .template
        .get(&mut transaction)
        .await
        .expect("Expected to find template")
        .into_inner()
        .clone();

    transaction.commit().await.log_db_client_error()?;

    Ok(Json(BookingStatus {
        instances: statuses,
        config: agg.configuration.clone(),
        template,
    }))
}

#[axum::debug_handler]
async fn notify_aggregate_expiring(
    Path(agg_id): Path<Uuid>,
    Json(date_string): Json<String>
) -> Result<(), WebError> {

    tracing::info!("Call to notify_aggregate_expiring() for {agg_id} with date_string {date_string}");

    let ending_override = match DateTime::parse_from_rfc2822(&date_string) {
        Ok(datetime) => Some(datetime.with_timezone(&Utc)),
        Err(_) => {
            tracing::error!("Unable to parse date string {date_string}! Defaulting to aggregate metadata...");
            None
        },
    };

    let agg_id: FKey<Aggregate> = FKey::from_id(agg_id.into());

    let dispatch = DISPATCH.get().ok_or((StatusCode::INTERNAL_SERVER_ERROR, format!("Unable to get dispatcher")))?;

    dispatch.send(
        workflows::entry::Action::NotifyExpiring {
            agg_id, ending_override
        })
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, format!("Unable to execute notify task!")))?;

    Ok(())
}

pub fn routes(state: AppState) -> ApiRouter {
    ApiRouter::new() // remember that in order to have the Handler trait, all inputs for
        // a handler need to implement FromRequest, and all outputs need to implement IntoResponse
        .route("/:agg_id/status", get(booking_status))
        .route("/create", post(create_booking))
        .route("/:agg_id/end", delete(end_booking))
        .route("/ipmi/:instance_id/powerstatus", get(instance_power_state))
        .route("/ipmi/:instance_id/setpower", post(instance_power_control))
        .route("/ipmi/:instance_id/getfqdn", get(fetch_ipmi_fqdn))
        .route("/:agg_id/notify/expiring", post(notify_aggregate_expiring))
}

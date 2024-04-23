use axum::{routing::post, Json, Router};
use metrics::prelude::*;

async fn handle_booking(Json(booking): Json<BookingMetric>) -> Result<String, MetricError> {
    MetricHandler::send(booking)?;

    Ok("Received booking".to_string())
}

async fn handle_provision(Json(provision): Json<ProvisionMetric>) -> Result<String, MetricError> {
    MetricHandler::send(provision)?;

    Ok("Received provision".to_string())
}

pub fn routes(_state: super::AppState) -> Router {
    Router::new()
        .route("/booking", post(handle_booking))
        .route("/provision", post(handle_provision))
}

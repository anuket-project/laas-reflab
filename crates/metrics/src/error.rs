//! Metrics Error Module
//!
//! defines [`MetricError`] enum which represents various errors that can occur
//! during metric processing and communication. It also implements the [`IntoResponse`] trait from
//! [`axum`] to convert errors into HTTP responses, which is useful when integrating this library
//! into an `axum` HTTP server.
use crate::message::MetricMessage;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

/// Represents errors that can occur during metric processing and communication.
#[derive(Debug, Clone, Error)]
pub enum MetricError {
    /// Error occurred while sending a [`MetricMessage`] through an [`mspc`] channel.
    #[error("Failed to send metric {0}")]
    SendError(#[from] Box<SendError<MetricMessage>>),

    /// Error occurred while creating the Telegraf [`Client`].
    #[error("Failed to create client with connection url: {0}. Retries exceeded.")]
    ClientError(String),

    /// Error occurred while pushing a metric to the Telegraf [`Client`].
    #[error("Failed to push metric to TCP Listener: {0}")]
    WriteError(String),

    #[error("Invalid or missing metrics configuration. Metrics will not be sent.")]
    ConfigError,
}

/// Implements the [`IntoResponse`] trait from [`axum`] to convert [`MetricError`] into an HTTP response.
impl IntoResponse for MetricError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            MetricError::SendError(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            MetricError::WriteError(e) | MetricError::ClientError(e) => {
                (StatusCode::INTERNAL_SERVER_ERROR, e)
            }
            MetricError::ConfigError => (StatusCode::BAD_REQUEST, Self::ConfigError.to_string()),
        };
        (status, Json(json!({ "message": error_message }))).into_response()
    }
}

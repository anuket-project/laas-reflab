//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::*;

use axum::{http::StatusCode, response::IntoResponse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{backtrace::Backtrace, collections::HashMap};

pub trait ErrorToResponse {
    type Output: IntoResponse;
    fn with_code<T>(self, code: T) -> Self::Output
    where
        T: Into<axum::http::StatusCode>;
}

impl ErrorToResponse for anyhow::Error {
    type Output = (axum::http::StatusCode, String);

    fn with_code<T>(self, code: T) -> Self::Output
    where
        T: Into<axum::http::StatusCode>,
    {
        let code: axum::http::StatusCode = code.into();
        (code, format!("Error handling request: {self}"))
    }
}

pub struct LLStatusCode {
    code: u16,
}

impl From<StatusCode> for LLStatusCode {
    fn from(value: StatusCode) -> Self {
        Self {
            code: value.as_u16(),
        }
    }
}

impl From<u16> for LLStatusCode {
    fn from(value: u16) -> Self {
        Self { code: value }
    }
}

impl From<LLStatusCode> for StatusCode {
    fn from(value: LLStatusCode) -> Self {
        StatusCode::from_u16(value.code)
            .expect("tried to construct a StatusCode from an invalid number")
    }
}

pub trait ResultWithCode<V>: Sized {
    fn log_error<S>(
        self,
        code: StatusCode,
        outward_message: S,
        should_log: bool,
    ) -> Result<V, (StatusCode, String)>
    where
        S: Into<String>;

    fn log_server_error<S>(
        self,
        outward_message: S,
        should_log: bool,
    ) -> Result<V, (StatusCode, String)>
    where
        S: Into<String>,
    {
        self.log_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            outward_message,
            should_log,
        )
    }

    fn log_opaque_server_error(self, should_log: bool) -> Result<V, (StatusCode, String)> {
        self.log_server_error("the server ran into an unrecoverable error, this event has been logged and will be reviewed by the site administrators shortly", should_log)
    }

    fn log_db_client_error(self) -> Result<V, (StatusCode, String)> {
        self.log_server_error("the server was unable to establish a proper internal connection to the database, the administrators will review this event shortly", true)
    }
}

impl<V> ResultWithCode<V> for Result<V, anyhow::Error> {
    fn log_error<S>(
        self,
        code: StatusCode,
        outward_message: S,
        _should_log: bool,
    ) -> Result<V, (StatusCode, String)>
    where
        S: Into<String>,
    {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                tracing::info!("Got an error: {e:?}");
                //let conv_e = anyhow::Error::new(e);
                let outward_message: String = outward_message.into();
                let outward_message = format!("Error handling request: {outward_message}");

                tracing::error!("{e:?}");

                //todo!("Log errors before returning from API");

                Err((code, outward_message))
            }
        }
    }
}

pub trait ResultWithCodeSpecialization<V> {
    fn log_error<S>(
        self,
        code: StatusCode,
        outward_message: S,
        should_log: bool,
    ) -> Result<V, (StatusCode, String)>
    where
        S: Into<String>;
}

impl<V> ResultWithCode<V> for Option<V> {
    fn log_error<S>(
        self,
        code: StatusCode,
        outward_message: S,
        should_log: bool,
    ) -> Result<V, (StatusCode, String)>
    where
        S: Into<String>,
    {
        match self {
            Some(v) => Ok(v),
            None => {
                let conv_e = anyhow::Error::msg("object did not exist");
                let conv_c: LLStatusCode = code.into();
                let conv_c: axum::http::StatusCode = conv_c.into();

                let outward_message: String = outward_message.into();
                let outward_message = format!("Error handling request: {outward_message}");

                if should_log {
                    tracing::error!("Error occurred while handling a request: {conv_e}");
                }

                return Err((code, outward_message));
            }
        }
    }
}

pub trait AnyWay<T> {
    fn anyway(self) -> Result<T, anyhow::Error>;
}

pub trait AnyWaySpecStr<T> {
    fn anyway(self) -> Result<T, anyhow::Error>;
}

impl<T, E> AnyWay<T> for Result<T, E>
where
    E: std::error::Error + Sync + Send + 'static,
{
    default fn anyway(self) -> Result<T, anyhow::Error> {
        self.map_err(|e| anyhow::Error::new(e))
    }
}

impl<T> AnyWaySpecStr<T> for Result<T, &'static str> {
    fn anyway(self) -> Result<T, anyhow::Error> {
        self.map_err(|e| anyhow::Error::msg(format!("{e}")))
    }
}

impl<T> AnyWaySpecStr<T> for Result<T, String> {
    fn anyway(self) -> Result<T, anyhow::Error> {
        self.map_err(|e| anyhow::Error::msg(format!("{e}")))
    }
}

#[derive(Clone, Deserialize, Serialize, Debug)] //JsonSchema
pub struct IDPayload<T>
where
    T: JsonSchema,
{
    pub id: uuid::Uuid,
    pub payload: T,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema, Debug)]
pub struct ApiError {
    pub http_code: String,
    pub error_message: String,
    pub related_data: HashMap<String, Value>, // Value name and value
    pub trace: Option<String>,                // backtrace as a string
}

impl ApiError {
    pub fn new(
        code: StatusCode,
        msg: String,
        data: HashMap<String, Value>,
        trace: Option<Backtrace>,
    ) -> ApiError {
        ApiError {
            http_code: code.to_string(),
            error_message: msg,
            related_data: data,
            trace: trace.map(|bt| format!("{bt}")),
        }
    }

    pub fn trivial(code: StatusCode, msg: String) -> Self {
        Self::new(
            code,
            msg,
            Default::default(),
            Some(Backtrace::force_capture()),
        )
    }
}

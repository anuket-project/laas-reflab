//! # Overview
//!
//! This crate provides an easy abstraction for asynchronously submitting metrics to
//! [Telegraf](https://github.com/influxdata/telegraf)
//! within a [`tokio`] runtime. It primarily consists of two main components: the
//! [`MetricHandler`] and the [`MetricConsumer`].
//!
//! The `MetricHandler` is responsible for initializing the metric processing infrastructure,
//! including the creation of a `MetricConsumer`. The `MetricConsumer` asynchronously listens for incoming
//! `MetricMessage`'s within an isolated tokio task and processes them by pushing them to a Telegraf
//!
//! This crate requires a configuration file to be present that contains a valid [`MetricsConfig`].
//! It will not panic if the configuration is missing, but it will return an error when trying to send
//! metrics.
//!
//! # Usage
//!
//! Sending a metric message is super easy, only two steps:
//! 1. Create an instance of your metric struct, this can be done in anyway you like. ex.
//!    `MyMetric::new()` or
//!    `MyMetric { /* ...fields */ }`.
//!    There are convienent default impl's and a [`Timestamp`] struct provided for you.
//! 4. Send your [`MetricMessage`] using the [`send()`] method on [`MetricHandler`]
//!
//! ```rust
//! use metrics::prelude::*;
//!
//! //... MyMetric definition
//!
//! let metric = MyMetric  {
//!     // you could also use `Default::default()` if you want the current timestamp.
//!     ts: Timestamp::now(),
//!     example_tag: "Foo".to_string(),
//!     example_field: 42,
//!  }
//!
//! // if you get an error while trying to use this, you forgot to add your new metric
//! // to the `MetricMessage` enum
//! // `send()` returns a Result<(), MetricError>
//! sender.send(message).unwrap();
//! ```
//! # Design
//!
//! The design is centered around asynchronous message processing using `tokio`'s
//! [`UnboundedReceiver`] and [`UnboundedSender`] types. The `MetricHandler` is responsible for initializing
//! an [`unbounded_channel`] and decouples message submission from processing. The
//! `MetricConsumer` then takes control of the initialized `UnboundedReceiver` and loops through incoming
//! [`MetricMessage`]'s. This allows the sender to be accessed from anywhere in the codebase
//! including other async tasks.
//!
//! # Further Reading
//!
//! - [`tokio`]: Especially `sync::mspc`,
//! - [`telegraf`]: Telegraf client library
//! - [`serde`]: Serialization and deserialization.
//! - [`tracing`]: Logging and tracing.
//! - [`enum_dispatch`]: Automatically unwrapping enum variants.
//! - [`config`]: Configuration file parsing.
//!
//! [`Metric`]: telegraf::Metric
//! [`metrics`]: crate::metrics
//! [`MetricsConfig`]: config::MetricsConfig
//! [`send()`]: tokio::sync::mpsc::UnboundedSender::send()
#![allow(unused_imports)]
use config::MetricsConfig;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, ErrorKind};
use std::sync::OnceLock;
use telegraf::{Client, Metric};
use tokio::sync::mpsc::{self, unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

pub mod error;
pub mod message;
pub mod metrics;
pub mod prelude;

use error::MetricError;
use message::{MetricMessage, MetricWrapper};
pub use metrics::*;

static METRIC_HANDLER: OnceLock<MetricHandler> = OnceLock::new();

/// Represents a handler for managing metric communication.
/// It provides a global sender for submitting metrics and handles the
/// lifecycle of a [`MetricConsumer`] task.
pub struct MetricHandler {
    tx: UnboundedSender<MetricMessage>,
    cancel: CancellationToken,
}

#[allow(clippy::new_without_default)]
impl MetricHandler {
    /// Returns a global sender for submitting metric messages.
    /// This should be the only method used to submit metrics outside of this crate.
    ///
    /// # Returns
    ///
    /// A reference to a global [`UnboundedSender`] for submitting [`MetricMessage`]'s.
    ///
    /// # Examples
    /// ```rust no_run
    /// fn main() {
    ///     let metric = MyMetric  {
    ///         example_tag: "Foo".to_string(),
    ///         example_field: 42,
    ///     }
    ///
    ///     MetricHandler::send(metric).unwrap();
    /// }
    /// ```
    fn global_sender() -> &'static UnboundedSender<MetricMessage> {
        let handler = METRIC_HANDLER.get_or_init(Self::new);
        &handler.tx
    }

    /// Sends a metric message to the global [`UnboundedSender`]. This method is the primary
    /// way to submit metrics to Telegraf.
    ///
    /// # Errors
    /// Returns a [`MetricError`] if the message cannot be sent.
    /// This could be due to a closed receiver, write error, missing config etc.
    ///
    /// # Examples
    /// ```rust
    /// let metric = Provision::default();
    /// MetricHandler::send(metric).unwrap();
    /// ```
    pub fn send(message: impl Into<MetricMessage>) -> Result<(), MetricError> {
        Self::global_sender().send(message.into())?;
        Ok(())
    }

    /// Creates a new [`MetricHandler`] and starts the [`MetricConsumer`] task. This should not be
    /// called directly. Please use [`MetricHandler::send()`] which wraps this functions
    /// in a static [`OnceLock`] if you would like to send metrics.
    pub fn new() -> Self {
        let (tx, rx) = unbounded_channel();
        let cancel = CancellationToken::new();

        tokio::spawn(MetricConsumer::new(rx, cancel.clone()).unwrap().run());

        Self { tx, cancel }
    }

    /// Cancels the [`CancellationToken`], causing the [`MetricConsumer`] to stop its event loop.
    pub fn cancel(&self) {
        self.cancel.cancel()
    }
}

/// Consumes and processes metric messages asynchronously. Responsible
/// for pushing metrics to a Telegraf [`Client`].
pub struct MetricConsumer {
    /// The [`UnboundedReceiver`] for incoming [`MetricMessage`]'s.
    pub rx: UnboundedReceiver<MetricMessage>,
    /// The Telegraf [`Client`] for pushing metrics.
    pub client: Client,
    /// A [`CancellationToken`] for stopping the event loop.
    pub cancel: CancellationToken,
    /// Configuration defined in the `config` module.
    pub config: MetricsConfig,
    /// failover counter for handling client recovery.
    pub failover_counter: usize,
}

impl MetricConsumer {
    /// Creates a new [`MetricConsumer`] with the given receiver and cancellation
    /// token. It initializes the Telegraf [`Client`] which may fail.
    pub fn new(
        rx: UnboundedReceiver<MetricMessage>,
        cancel: CancellationToken,
    ) -> Result<Self, MetricError> {
        let config = match &config::settings().metrics {
            Some(config) => config,
            None => return Err(MetricError::ConfigError),
        };

        let client = get_client(config)?;

        Ok(Self {
            rx,
            client,
            cancel,
            config: config.clone(),
            failover_counter: 0,
        })
    }

    /// Starts the asynchronous loop for consuming and processing [`MetricMessage`]'s.
    /// This method is called by [`Self::new()`] and runs until cancelled.
    pub async fn run(mut self) -> Result<(), MetricError> {
        while !self.cancel.is_cancelled() {
            if let Some(message) = self.rx.recv().await {
                self.process_message(message);
            }

            if self.failover_counter > 0 {
                if let Err(e) = self.attempt_client_recovery().await {
                    warn!(
                        "Client recovery failed after {} retries: {}",
                        self.failover_counter, e
                    );
                }
            }
        }
        Ok(())
    }

    /// Processes a single [`MetricMessage`] by recieved from [`Self::rx`]
    /// Drops messages on write error, and increments the failover counter.
    pub fn process_message(&mut self, message: MetricMessage) {
        match message.write_to_client(&mut self.client) {
            Ok(_) => {
                info!("Metric successfully sent to Telegraf.");
            }
            Err(e) => {
                warn!("Failed to send metric to Telegraf: {}", e);
                info!("Dropped Metric: {:?}", message);
                self.failover_counter += 1;
            }
        }
    }

    /// Attempts to recover the Telegraf [`Client`] by creating a new instance
    /// from the same configuration. If successful, resets [`Self::failover_counter`].
    pub async fn attempt_client_recovery(&mut self) -> Result<(), MetricError> {
        match get_client(&self.config) {
            Ok(new_client) => {
                self.client = new_client;
                self.failover_counter = 0;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// Returns a new instance of the Telegraf [`Client`] based on the configuration settings.
///
/// # Errors
///
/// Returns a [`MetricError::ClientError`] if the client cannot be created after the
/// configured number of retries and with the provided connection string.
///
/// See [`MetricsConfig`] and [`telegraf`] documentation for reference.
pub fn get_client(config: &MetricsConfig) -> Result<Client, MetricError> {
    let connection_str = &config.url;
    let max_retries = config.client_retries;
    let mut retries = 0;

    while retries < max_retries {
        match Client::new(connection_str) {
            Ok(client) => return Ok(client),
            Err(_) => {
                warn!("Failed to create client. Retrying...");
                retries += 1;
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }

    Err(MetricError::ClientError(connection_str.to_string()))
}

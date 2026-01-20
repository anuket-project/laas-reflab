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
use config::MetricsConfig;
use std::sync::OnceLock;
use telegraf::Client;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

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
        Self::global_sender()
            .send(message.into())
            .map_err(|e| MetricError::SendError(Box::new(e)))?;
        Ok(())
    }

    /// Creates a new [`MetricHandler`] and starts the [`MetricConsumer`] task.
    /// This should not be called directly, only indirectly through [`MetricHandler::send()`]
    /// which ensures the handler is only intialized once globally.
    pub fn new() -> Self {
        let (tx, rx) = unbounded_channel::<MetricMessage>();
        let cancel = CancellationToken::new();
        let cancel_cloned = cancel.clone();

        tokio::spawn(async move {
            Self::initialize_consumer(rx, cancel_cloned).await;
        });

        Self { tx, cancel }
    }

    /// Asynchronously initializes [`MetricConsumer`] with the given receiver and cancellation token.
    async fn initialize_consumer(rx: UnboundedReceiver<MetricMessage>, cancel: CancellationToken) {
        match MetricConsumer::new(rx, cancel.clone()).await {
            Ok(consumer) => {
                tokio::spawn(consumer.run());
            }
            Err(e) => {
                warn!(
                    "Could not initialize metric consumer. Metrics will not be sent: {:?}",
                    e
                );
            }
        }
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
}

impl MetricConsumer {
    /// Creates a new [`MetricConsumer`] with the given receiver and cancellation
    /// token. It initializes the Telegraf [`Client`] which may fail.
    pub async fn new(
        rx: UnboundedReceiver<MetricMessage>,
        cancel: CancellationToken,
    ) -> Result<Self, MetricError> {
        let config = match &config::settings().metrics {
            Some(config) => config,
            None => return Err(MetricError::ConfigError),
        };

        let client = get_client(config).await?;

        Ok(Self {
            rx,
            client,
            cancel,
            config: config.clone(),
        })
    }

    /// Starts the asynchronous loop for consuming and processing [`MetricMessage`]'s.
    /// This method is called by [`Self::new()`] and runs until cancelled.
    pub async fn run(mut self) -> Result<(), MetricError> {
        while !self.cancel.is_cancelled() {
            if let Some(message) = self.rx.recv().await {
                self.process_message(message).await;
            }
        }
        Ok(())
    }

    /// Processes a single [`MetricMessage`] received from [`Self::rx`].
    /// If the write fails, attempts recovery and retries once before dropping.
    pub async fn process_message(&mut self, message: MetricMessage) {
        // First attempt
        if self.try_write(&message).is_ok() {
            return;
        }

        // First attempt failed, try recovery and retry
        warn!("Write failed, attempting recovery and retry...");
        if let Err(e) = self.reconnect().await {
            warn!("Recovery failed: {}", e);
            info!("Dropped Metric: {:?}", message);
            return;
        }

        // Retry after successful recovery
        if let Err(e) = self.try_write(&message) {
            warn!("Retry after recovery also failed: {}", e);
            info!("Dropped Metric: {:?}", message);
        }
    }

    /// Attempts to write a metric to the Telegraf client.
    fn try_write(&mut self, message: &MetricMessage) -> Result<(), MetricError> {
        message.write_to_client(&mut self.client)?;
        info!("Metric successfully sent to Telegraf.");
        Ok(())
    }

    /// Reconnects to Telegraf by creating a new client.
    async fn reconnect(&mut self) -> Result<(), MetricError> {
        self.client = get_client(&self.config).await?;
        info!("Successfully reconnected to Telegraf.");
        Ok(())
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
pub async fn get_client(config: &MetricsConfig) -> Result<Client, MetricError> {
    let connection_str = &config.url;
    let max_retries = config.client_retries;
    let mut last_error = None;

    for attempt in 1..=max_retries {
        match Client::new(connection_str) {
            Ok(client) => return Ok(client),
            Err(e) => {
                warn!(
                    "Failed to create client (attempt {}/{}): {}",
                    attempt, max_retries, e
                );
                last_error = Some(e);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }

    if let Some(e) = last_error {
        warn!("All {} connection attempts failed. Last error: {}", max_retries, e);
    }
    Err(MetricError::ClientError(connection_str.to_string()))
}

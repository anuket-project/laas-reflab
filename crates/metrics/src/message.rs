//! [`MetricMessage`] implementation, which is a wrapper around different types of metrics. Each
//! variant of the enum corresponds to a different metric struct defined in the `metrics` module.
//!
//! This module also contains the [`MetricWrapper`] trait, which is a marker trait implemented for
//! all types that implement the [`Metric`] trait.
//!
//! The [`enum_dispatch::enum_dispatch`] attribute macro is used on the [`MetricMessage`] enum to generate the
//! associated trait implementations for the enum variants.
use crate::error::MetricError;
use crate::metrics::*;
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use telegraf::{Client, Metric};

/// A trait that wraps the functionality to write a metric to a Telegraf [`Client`].
/// This trait is implemented for all types that implement the [`Metric`] trait.
#[enum_dispatch]
pub trait MetricWrapper {
    /// Writes the metric to the given Telegraf [`Client`].
    ///
    /// # Errors
    ///
    /// Returns a [`MetricError::WriteError`] if the metric cannot be written to the client.
    fn write_to_client(&self, client: &mut Client) -> Result<(), MetricError>;
}

impl<T: Metric> MetricWrapper for T {
    fn write_to_client(&self, client: &mut Client) -> Result<(), MetricError> {
        client
            .write(self)
            .map_err(|e| MetricError::WriteError(e.to_string()))
    }
}

/// An enum that represents different types of [`MetricMessage`]'s.
/// Each variant corresponds to a different struct defined in the [`metrics`] module.
///
/// # Examples
/// ```rust no_run
/// use metrics::prelude::*;
///
/// let booking = booking::default();
/// let metric_message = MetricMessage::Booking(booking);
/// let metric_message = MetricMessage::Provision(provision::default());
///
/// // you can also just use the generated `From` impl
/// let provision = provision::default();
/// let metric_message: MetricMessage = provision.into();
/// let metric_message = MetricMessage::from(provision);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[enum_dispatch(MetricWrapper)]
pub enum MetricMessage {
    Booking(BookingMetric),
    Provision(ProvisionMetric),
    BookingExpired(BookingExpiredMetric),
    // ...add additional metrics defined in the metrics module here
}

//! This module contains metric structs designed for use with the [`telegraf`] client library.
//!
//! Each metric struct is annotated with the [`Metric`] derive macro, which automatically
//! implements the necessary traits for integration with Telegraf.
//!
//! Fields within each metric struct are annotated with either `#[telegraf(tag)]` or `#[telegraf(field)]`
//! to indicate how they should be treated by Telegraf:
//!
//! - **Tag**: Used for metadata that is useful for grouping and filtering but not for aggregation.
//!   Tags are indexed, so they should be used for data with low cardinality.
//!
//! - **Field**: Used for data points that will be aggregated over time. Fields are not indexed
//!   and can handle high(er) cardinality data, while sacrificing query time. Fields can be overwritten
//!   by new metrics with identical timestamps, while differing tags will result in a different
//!   series.
//!
//! It's important to choose the appropriate type based on the nature of the data and the intended
//! use in queries. For more information, see the [Telegraf documentation](https://docs.influxdata.com/telegraf/).
//!
//! # Metric Types
//!
//! Currently, the following metrics are defined:
//!
//! - **[`BookingMetric`]**: Represents a booking logging event, tracking booking activity and usage.
//! - **[`ProvisionMetric`]**: Represents a provisioning event, tracking provisioning failures, retries,
//!   and host usage.
//!
//! # Usage
//!
//! You can create a metric by defining a new struct that implements the [`Metric`] trait using the
//! available derive and attribute macros from [`telegraf`]. All metrics are currently
//! defined in this module.
//!  
//! While the timestamp field is optional, you should always include it. This
//! is important for two reasons:
//! - It ensures correct times in the event the telegraf client fails and
//!   processing is shifted to a deserialized file. If a timestamp field is not provided to telegraf,
//!   it will insert the **time that telegraf recieves the metric** as the timestamp. This can
//!   cause all sorts of synchronization issues.
//! - Manually storing the timestamp allows rewriting a metric if the timestamp is known. This is useful
//!   for metrics like [`ProvisionMetric`] where we may want to update the timestamp if a retry is successful.
//!  
//! Every tag field must be annotated with `#[telegraf(tag)]`. Tags are indexed, fields are not.
//!
//! Every field must be annotated with `#[telegraf(field)]`. Each metric must contain at least one field.
//!
//! There is a dedicated [`Timestamp`] type that is implemented in this module, while I recommend using
//! it, you can optionally use any type that implements [`Into<u64>`] in the timestamp field.
//!
//! Here is an example metric definition:
//!
//! ```rust
//! // Every metric should derive at least these traits.
//! #[derive(Metric, Serialize, Deserialize, Clone)]
//! #[measurement = "my_custom_metric_name"]
//! pub struct MyMetric {
//!     #[telegraf(timestamp)]
//!     ts: Timestamp,
//!
//!     #[telegraf(tag)]
//!     example_tag: String,
//!
//!     #[telegraf(field)]
//!     example_field: i32,
//!}
//! ```
//!
//! > For more information on creating metrics, see the [`telegraf`] crate documentation.
//!
//! After defining a metric, you have to add it as a variant to the [`MetricMessage`] enum.
//!
//! ```rust
//! // Add the new metric as a variant to the MetricMessage enum.
//! pub enum MetricMessage {
//!    MyMetric(MyMetric),
//!    // ...other metrics
//! }
//! ```
//!
//! [`MetricMessage`] uses the [`enum_dispatch`] crate to derive [`From`] impl's for
//! enum variants. This is also how [`MetricWrapper::write_to_client()`] is available automatically
//! on `MetricMessage`, even though it's only generically implemented on types that implement [`Metric`].
//!
use super::message::*;
use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use telegraf::*;

/// Enables calling [`Timestampable::update`] and [`Timestampable::set`]
/// on [`Metric`] types instead of having to access the timestamp field directly.
pub trait Timestampable {
    /// Updates the timestamp to the current time.
    ///
    /// # Example
    /// ```rust
    /// use metrics::prelude::*;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let booking = Booking::default();
    /// sleep(Duration::from_secs(5));
    /// // This will be 5 seconds later than the original timestamp
    /// booking.update();
    /// ```
    fn update(self);
    /// Sets the timestamp to the given value.
    ///
    /// # Arguments
    /// * `ts` - The value to set the timestamp to.
    ///
    /// # Example
    /// ```rust
    /// use metrics::prelude::*;
    ///
    /// let booking = Booking::default();
    /// booking.set(Timestamp::now());
    ///
    /// let custom_ts = Timestamp::new(Utc.ymd(2021, 8, 12).and_hms(12, 0, 0));
    ///
    /// booking.set(custom_ts);
    /// ```
    fn set(self, ts: Timestamp);
}

/// A wrapper around the [`DateTime<Utc>`] type from the [`chrono`] crate.
///
/// [`telegraf`] requires it's timestamp field to implement [`Into<u64>`], because we can't implement a trait for a type
/// defined in another crate, we have to wrap it.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Timestamp(pub DateTime<Utc>);

impl Timestamp {
    /// Creates a new [`Timestamp`] from any type that implements [`Into<DateTime<Utc>>`].
    ///
    /// # Arguments
    ///
    /// * `dt` - The value to convert to a [`Timestamp`].
    ///
    /// # Returns
    /// A new [`Timestamp`] instance from the given value.
    ///
    pub fn new(dt: impl Into<DateTime<Utc>>) -> Self {
        Timestamp(dt.into())
    }

    /// Returns the current time as a [`Timestamp`].
    ///
    /// # Returns
    /// The current time as a [`Timestamp`].
    pub fn now() -> Self {
        Timestamp(Utc::now())
    }

    /// Returns a duration in seconds between the current time and the
    /// wrapped timestamp.
    pub fn elapsed(&self) -> u64 {
        Utc::now().signed_duration_since(self.0).num_seconds() as u64
    }
}

/// [`Default`] implementation for [`Timestamp`]. This sets the timestamp to the current time.
impl Default for Timestamp {
    fn default() -> Self {
        Timestamp::now()
    }
}

/// Implement the [`From`] for [`Timestamp`] to convert it to a [`u64`].
/// This is required by the [`telegraf::Metric`] derive macro.
impl From<Timestamp> for u64 {
    fn from(ts: Timestamp) -> Self {
        ts.0.timestamp_nanos_opt()
            .expect("ERROR: Out of range Timestamp") as u64
    }
}

/// Represents a booking metric event.
#[derive(Metric, Debug, Serialize, Deserialize, Clone, Default)]
#[measurement = "booking"]
pub struct BookingMetric {
    #[telegraf(timestamp)]
    #[serde(default)]
    /// The timestamp of the booking event.
    pub ts: Timestamp,

    #[telegraf(field)]
    #[serde(default)]
    pub booking_id: i32,

    #[telegraf(field)]
    #[serde(default)]
    /// **Field**: The duration of the booking in days.
    pub booking_length_days: i32,

    #[telegraf(field)]
    #[serde(default)]
    /// **Field:** The number of hosts associated with the booking.
    pub num_hosts: i32,

    #[telegraf(field)]
    #[serde(default)]
    /// **Field:** The number of collaborators associated with the booking.
    pub num_collaborators: i32,

    #[telegraf(field)]
    #[serde(default)]
    /// **Field:** The owner of the booking.
    pub owner: String,

    #[telegraf(tag)]
    #[serde(default)]
    /// **Tag:** The lab associated with this booking.
    pub lab: String,

    #[telegraf(tag)]
    #[serde(default)]
    /// **Tag:** The project associated with this booking.
    pub project: String,

    #[telegraf(field)]
    #[serde(default)]
    /// **Tag:** The purpose of this booking. ex. "ONAP_DEV"
    pub purpose: String,

    /// **Tag:** Metadata tag to differentiate between fake/mock data and real data in the
    /// dashboard.
    #[telegraf(tag)]
    #[serde(default)]
    pub mock: bool,
}

impl Timestampable for BookingMetric {
    fn update(mut self) {
        self.ts = Timestamp::now();
    }
    fn set(mut self, ts: Timestamp) {
        self.ts = ts;
    }
}

/// Represents a provisioning metric event.
#[derive(Metric, Default, Debug, Serialize, Deserialize, Clone)]
#[measurement = "provision"]
pub struct ProvisionMetric {
    #[telegraf(timestamp)]
    #[serde(default)]
    /// The timestamp of the provisioning event.
    pub ts: Timestamp,

    /// **Tag:** The hostname of the provisioned host. ie. "ampere-1-ampere-2"
    #[telegraf(tag)]
    #[serde(default)]
    pub hostname: String,

    /// **Tag:** Indicates whether the provisioning was successful.
    #[telegraf(tag)]
    #[serde(default)]
    pub success: bool,

    /// **Field:** Indicates whether this provisioning attempt is a retry.
    #[telegraf(field)]
    #[serde(default)]
    pub retries: i32,

    /// **Field:** Provisioning time in seconds. This must be calculated before sending the metric.
    #[telegraf(field)]
    #[serde(default)]
    pub provisioning_time_seconds: u64,

    /// **Field:** The owner of the host being provisioned.
    #[telegraf(field)]
    #[serde(default)]
    pub owner: String,

    /// **Tag:** The originating lab associated with the host being provisioned.
    #[telegraf(tag)]
    #[serde(default)]
    pub lab: String,

    /// **Tag:** The project associated with the host being provisioned.
    #[telegraf(tag)]
    #[serde(default)]
    pub project: String,

    /// **Tag:** Metadata tag to differentiate between fake/mock data and real data in the
    /// dashboard.
    #[telegraf(tag)]
    #[serde(default)]
    pub mock: bool,
}

impl Timestampable for ProvisionMetric {
    fn update(mut self) {
        self.ts = Timestamp::now();
    }
    fn set(mut self, ts: Timestamp) {
        self.ts = ts;
    }
}

/// Represents a booking expired metric event
#[derive(Metric, Default, Debug, Serialize, Deserialize, Clone)]
#[measurement = "booking_expired"]
pub struct BookingExpiredMetric {
    #[telegraf(timestamp)]
    #[serde(default)]
    pub ts: Timestamp,

    #[telegraf(field)]
    #[serde(default)]
    pub owner: String,

    #[telegraf(field)]
    #[serde(default)]
    pub booking_id: i32,

    #[telegraf(field)]
    #[serde(default)]
    pub extension_length_days: i32,

    #[telegraf(field)]
    #[serde(default)]
    pub project: String,

    #[telegraf(tag)]
    #[serde(default)]
    pub lab: String,

    #[telegraf(tag)]
    #[serde(default)]
    pub mock: bool,
}

impl Timestampable for BookingExpiredMetric {
    fn update(mut self) {
        self.ts = Timestamp::now();
    }
    fn set(mut self, ts: Timestamp) {
        self.ts = ts;
    }
}

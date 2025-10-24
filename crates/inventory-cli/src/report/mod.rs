//! Report generation and display for inventory changes
//!
//! This module provides types and traits for representing changes to inventory items.
//! Each type of inventory item (flavor, image, switch, host) has a corresponding
//! `Report` variant that describes whether it was created, modified, removed, or unchanged.
//!
//! ## Report Types
//!
//! - [`FlavorReport`]
//! - [`ImageReport`]
//! - [`SwitchReport`]
//! - [`HostReport`]
//! - [`SwitchportReport`]
//! - [`InterfaceReport`]
//! - [`KernelArgReport`]

use enum_dispatch::enum_dispatch;
use sqlx::{Postgres, Transaction};
use std::fmt::{self, Display};

mod flavor;
mod host;
mod image;
mod interface;
mod kernel_arg;
mod lab;
mod order;
mod switch;
mod switchport;

pub use order::SortOrder;

pub use flavor::FlavorReport;
pub use host::HostReport;
pub use image::ImageReport;
pub use interface::InterfaceReport;
pub use kernel_arg::KernelArgReport;
pub use lab::LabReport;
pub use switch::SwitchReport;
pub use switchport::SwitchportReport;

use crate::prelude::InventoryError;

/// Common interface for all report types
#[enum_dispatch]
pub trait Reportable {
    /// Returns a sort priority for ordering reports in output
    ///
    /// Lower numbers are displayed first. This allows us to sort all reports in a single
    /// collection by execution order.
    fn sort_order(&self) -> u8;

    /// Returns true if this report represents an unchanged item
    fn is_unchanged(&self) -> bool {
        false
    }

    /// Returns true if this report represents a newly created item
    fn is_created(&self) -> bool {
        false
    }

    /// Returns true if this report represents a modified item
    fn is_modified(&self) -> bool {
        false
    }

    /// Returns true if this report represents a removed item
    fn is_removed(&self) -> bool {
        false
    }

    /// Execute the changes represented by this report
    ///
    /// created items: inserts new records.
    /// modified items: updates existing records.
    /// removed items: soft-deletes or removes records.
    /// unchanged items: no-op
    #[allow(async_fn_in_trait)]
    async fn execute(
        &self,
        _transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        Err(InventoryError::NotImplemented(
            "execute method not implemented for this report type".to_string(),
        ))
    }
}

#[derive(Debug, Clone)]
#[enum_dispatch(Reportable)]
pub enum Report {
    HostReport(HostReport),
    SwitchReport(SwitchReport),
    FlavorReport(FlavorReport),
    ImageReport(ImageReport),
    LabReport(LabReport),
}

impl Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Report::HostReport(report) => write!(f, "{}", report),
            Report::SwitchReport(report) => write!(f, "{}", report),
            Report::FlavorReport(report) => write!(f, "{}", report),
            Report::ImageReport(report) => write!(f, "{}", report),
            Report::LabReport(report) => write!(f, "{}", report),
        }
    }
}

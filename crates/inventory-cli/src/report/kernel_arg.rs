use crate::{
    prelude::{InventoryError, KernelArg, Reportable},
    report::SortOrder,
};
use colored::Colorize;
use models::dashboard::ImageKernelArg;
use sqlx::{Postgres, Transaction};
use std::fmt;

#[derive(Debug, Clone)]
pub enum KernelArgReport {
    Created {
        image_name: String,
        arg: KernelArg,
    },
    Removed {
        image_name: String,
        arg: ImageKernelArg,
    },
    Unchanged,
}

impl KernelArgReport {
    pub fn new_created(image_name: String, arg: KernelArg) -> Self {
        KernelArgReport::Created { image_name, arg }
    }
    pub fn new_deleted(image_name: String, arg: ImageKernelArg) -> Self {
        KernelArgReport::Removed { image_name, arg }
    }

    pub async fn execute_removed(
        &self,
        _transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let KernelArgReport::Removed { .. } = self {
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Removed",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_created(
        &self,
        _transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let KernelArgReport::Created { .. } = self {
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Created",
                actual: self.report_name(),
            })
        }
    }
    pub async fn execute_unchanged(
        &self,
        _transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let KernelArgReport::Unchanged = self {
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Unchanged",
                actual: self.report_name(),
            })
        }
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            KernelArgReport::Created { .. } => "Created",
            KernelArgReport::Removed { .. } => "Removed",
            KernelArgReport::Unchanged => "Unchanged",
        }
    }
}

impl Reportable for KernelArgReport {
    fn is_unchanged(&self) -> bool {
        matches!(self, KernelArgReport::Unchanged)
    }

    fn is_created(&self) -> bool {
        matches!(self, KernelArgReport::Created { .. })
    }

    fn is_removed(&self) -> bool {
        matches!(self, KernelArgReport::Removed { .. })
    }

    #[allow(async_fn_in_trait)]
    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            KernelArgReport::Created { .. } => self.execute_created(transaction).await,
            KernelArgReport::Removed { .. } => self.execute_removed(transaction).await,
            KernelArgReport::Unchanged => self.execute_unchanged(transaction).await,
        }
    }

    fn sort_order(&self) -> u8 {
        match self {
            KernelArgReport::Created { .. } => SortOrder::KernelArg as u8,
            KernelArgReport::Removed { .. } => SortOrder::KernelArg as u8 + 1,
            KernelArgReport::Unchanged => SortOrder::KernelArg as u8 + 2,
        }
    }
}

impl fmt::Display for KernelArgReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelArgReport::Created { arg, .. } => {
                let arg_str = match arg {
                    KernelArg::Flag(flag) => flag.clone(),
                    KernelArg::KeyValue { key, value } => format!("{}={}", key, value),
                };
                write!(f, "{}{}", "+".green(), arg_str)
            }
            KernelArgReport::Removed { arg, .. } => {
                write!(f, "{}{}", "-".red(), arg.render_to_kernel_arg())
            }
            KernelArgReport::Unchanged => write!(f, "{}{}", "=".dimmed(), "unchanged".dimmed()),
        }
    }
}

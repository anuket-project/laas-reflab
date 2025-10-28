use colored::Colorize;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::fmt;

use crate::prelude::{
    HostPort, InterfaceYaml, InventoryError, ModifiedFields, Reportable, SortOrder, hostport,
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum InterfaceReport {
    Created {
        server_name: String,
        flavor_name: String,
        interface_yaml: InterfaceYaml,
    },
    Modified {
        server_name: String,
        flavor_name: String,
        fields: ModifiedFields,
        interface_yaml: InterfaceYaml,
    },
    Removed {
        server_name: String,
        db_interface: HostPort,
    },
    Unchanged,
}

impl InterfaceReport {
    pub fn new_created(
        server_name: String,
        flavor_name: String,
        interface_yaml: InterfaceYaml,
    ) -> Self {
        Self::Created {
            server_name,
            flavor_name,
            interface_yaml,
        }
    }

    pub fn new_modified(
        server_name: String,
        flavor_name: String,
        fields: ModifiedFields,
        interface_yaml: InterfaceYaml,
    ) -> Self {
        Self::Modified {
            server_name,
            flavor_name,
            fields,
            interface_yaml,
        }
    }

    pub fn new_removed(server_name: String, db_interface: HostPort) -> Self {
        Self::Removed {
            server_name,
            db_interface,
        }
    }

    pub fn new_unchanged() -> Self {
        InterfaceReport::Unchanged
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            InterfaceReport::Created { .. } => "Created",
            InterfaceReport::Modified { .. } => "Modified",
            InterfaceReport::Removed { .. } => "Removed",
            InterfaceReport::Unchanged => "Unchanged",
        }
    }

    /// Execute a created interface report
    pub async fn execute_created(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let InterfaceReport::Created {
            server_name,
            flavor_name,
            interface_yaml,
        } = self
        {
            hostport::create_hostport_from_iface(
                transaction,
                interface_yaml,
                server_name,
                flavor_name,
            )
            .await?;

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Created",
                actual: self.report_name(),
            })
        }
    }

    /// Execute a modified interface report
    pub async fn execute_modified(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let InterfaceReport::Modified {
            server_name,
            flavor_name,
            fields: _,
            interface_yaml,
        } = self
        {
            hostport::update_hostport_by_name(
                transaction,
                interface_yaml,
                server_name,
                flavor_name,
            )
            .await?;

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Modified",
                actual: self.report_name(),
            })
        }
    }

    /// Execute a removed interface report
    pub async fn execute_removed(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let InterfaceReport::Removed {
            server_name,
            db_interface,
        } = self
        {
            hostport::delete_hostport_by_name(transaction, server_name, db_interface).await?;

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Removed",
                actual: self.report_name(),
            })
        }
    }

    /// Execute a an unchanged interface report
    pub fn execute_unchanged(&self) -> Result<(), InventoryError> {
        if let InterfaceReport::Unchanged = self {
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Unchanged",
                actual: self.report_name(),
            })
        }
    }
}

impl Reportable for InterfaceReport {
    fn sort_order(&self) -> u8 {
        match self {
            InterfaceReport::Created { .. } => SortOrder::HostPort as u8,
            InterfaceReport::Modified { .. } => SortOrder::HostPort as u8 + 1,
            InterfaceReport::Removed { .. } => SortOrder::HostPort as u8 + 2,
            InterfaceReport::Unchanged => SortOrder::HostPort as u8 + 3,
        }
    }

    fn is_unchanged(&self) -> bool {
        matches!(self, InterfaceReport::Unchanged)
    }
    fn is_created(&self) -> bool {
        matches!(self, InterfaceReport::Created { .. })
    }
    fn is_modified(&self) -> bool {
        matches!(self, InterfaceReport::Modified { .. })
    }
    fn is_removed(&self) -> bool {
        matches!(self, InterfaceReport::Removed { .. })
    }

    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            InterfaceReport::Created { .. } => self.execute_created(transaction).await,
            InterfaceReport::Modified { .. } => self.execute_modified(transaction).await,
            InterfaceReport::Removed { .. } => self.execute_removed(transaction).await,
            InterfaceReport::Unchanged => self.execute_unchanged(),
        }
    }
}

impl fmt::Display for InterfaceReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterfaceReport::Created {
                server_name: _,
                flavor_name: _,
                interface_yaml,
            } => write!(f, "{}{}", "+".green(), interface_yaml.name),
            InterfaceReport::Removed {
                server_name: _,
                db_interface,
            } => write!(f, "{}{}", "-".red(), db_interface.name),
            InterfaceReport::Modified {
                flavor_name: _,
                fields,
                server_name: _,
                interface_yaml,
            } => {
                write!(f, "{}{}", "~".yellow(), interface_yaml.name)?;
                let db_report = fields.to_string();
                for line in db_report.lines() {
                    write!(f, " {}", line)?;
                }
                Ok(())
            }

            // ignore unchanged
            _ => Ok(()),
        }
    }
}

use colored::Colorize;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::fmt;

use crate::prelude::{Host, HostYaml, InterfaceReport, InventoryError, ModifiedFields, host};

use super::Reportable;

/// Represents a constructed diff between the inventory and the database state for a host.
/// Each variant wraps its own data needed to both display the report and to write it the changes
/// to the database.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum HostReport {
    Created {
        // we want `HostYaml` because it's what we need to write to DB
        host_yaml: HostYaml,
        interface_reports: Vec<InterfaceReport>,
    },
    Modified {
        // we want `HostYaml` because it's what we need to write to DB
        host_yaml: HostYaml,
        fields: ModifiedFields,
        interface_reports: Vec<InterfaceReport>,
    },
    Removed {
        // we don't need `HostYaml` here, we just need the server name and the ability to display
        // the `Host` to be deleted
        db_host: Host,
        interface_reports: Vec<InterfaceReport>,
    },
    Unchanged {
        host_yaml: HostYaml,
    },
}

impl Reportable for HostReport {
    fn sort_order(&self) -> u8 {
        match self {
            HostReport::Created { .. } => 4,
            HostReport::Removed { .. } => 5,
            HostReport::Modified { .. } => 6,
            HostReport::Unchanged { .. } => 7,
        }
    }

    fn is_unchanged(&self) -> bool {
        matches!(self, HostReport::Unchanged { .. })
    }
    fn is_created(&self) -> bool {
        matches!(self, HostReport::Created { .. })
    }
    fn is_modified(&self) -> bool {
        matches!(self, HostReport::Modified { .. })
    }
    fn is_removed(&self) -> bool {
        matches!(self, HostReport::Removed { .. })
    }

    async fn execute(&self, pool: &PgPool) -> Result<(), InventoryError> {
        match self {
            HostReport::Created { .. } => self.execute_created(pool).await,
            HostReport::Modified { .. } => self.execute_modified(pool).await,
            HostReport::Removed { .. } => self.execute_removed(pool).await,
            HostReport::Unchanged { .. } => self.execute_unchanged(),
        }
    }
}
impl HostReport {
    pub fn new_created(host_yaml: HostYaml, interface_reports: Vec<InterfaceReport>) -> Self {
        HostReport::Created {
            host_yaml,
            interface_reports,
        }
    }

    pub fn new_modified(
        host_yaml: HostYaml,
        fields: ModifiedFields,
        interface_reports: Vec<InterfaceReport>,
    ) -> Self {
        HostReport::Modified {
            host_yaml,
            fields,
            interface_reports,
        }
    }

    pub fn new_removed(db_host: Host, interface_reports: Vec<InterfaceReport>) -> Self {
        HostReport::Removed {
            db_host,
            interface_reports,
        }
    }

    pub fn new_unchanged(host_yaml: HostYaml) -> Self {
        HostReport::Unchanged { host_yaml }
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            HostReport::Created { .. } => "Created",
            HostReport::Modified { .. } => "Modified",
            HostReport::Removed { .. } => "Removed",
            HostReport::Unchanged { .. } => "Unchanged",
        }
    }

    /// No‑op for unchanged hosts.
    pub fn execute_unchanged(&self) -> Result<(), InventoryError> {
        if let HostReport::Unchanged { .. } = self {
            // nothing to do!
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Unchanged",
                actual: self.report_name(),
            })
        }
    }

    /// Insert a new host based on the YAML.
    pub async fn execute_created(&self, pool: &PgPool) -> Result<(), InventoryError> {
        if let HostReport::Created {
            host_yaml,
            interface_reports,
        } = self
        {
            // insert the host row
            host::create_host(pool, host_yaml).await?;

            // let interfaces handle themselves
            for iface_r in interface_reports {
                iface_r.execute(pool).await?;
            }

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Created",
                actual: self.report_name(),
            })
        }
    }

    /// Apply an update to an existing host.
    pub async fn execute_modified(&self, pool: &PgPool) -> Result<(), InventoryError> {
        if let HostReport::Modified {
            host_yaml,
            fields: _,
            interface_reports,
        } = self
        {
            // update the host in the database
            host::update_host(pool, host_yaml).await?;

            // let interfaces handle themselves
            for iface_r in interface_reports {
                iface_r.execute(pool).await?;
            }

            println!("Updating host {}...", host_yaml.server_name);

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Modified",
                actual: self.report_name(),
            })
        }
    }

    /// Delete a host and its interfaces.
    pub async fn execute_removed(&self, pool: &PgPool) -> Result<(), InventoryError> {
        if let HostReport::Removed {
            db_host,
            interface_reports,
        } = self
        {
            println!(
                "Removing host {} with {} interfaces...",
                db_host.server_name,
                interface_reports.len()
            );

            for iface_r in interface_reports {
                iface_r.execute(pool).await?;
            }

            println!("Removing host {}...", db_host.server_name);

            // delete the host after its interfaces are taken care of
            host::delete_host_by_name(pool, &db_host.server_name).await?;

            // TODO: finish
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Removed",
                actual: self.report_name(),
            })
        }
    }

    pub fn push_interface_report(&mut self, report: InterfaceReport) {
        match self {
            HostReport::Created {
                interface_reports, ..
            } => {
                interface_reports.push(report);
            }
            HostReport::Modified {
                interface_reports, ..
            } => {
                interface_reports.push(report);
            }
            HostReport::Removed {
                interface_reports, ..
            } => {
                interface_reports.push(report);
            }
            HostReport::Unchanged { host_yaml } => {
                panic!(
                    "Cannot push interface report to an unchanged host report ({})",
                    host_yaml.server_name
                );
            }
        }
    }
}

impl fmt::Display for HostReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostReport::Created {
                host_yaml,
                interface_reports,
            } => {
                writeln!(
                    f,
                    " {} {}",
                    "Created Host:".green().bold(),
                    host_yaml.server_name.green()
                )?;
                for interface_report in interface_reports {
                    writeln!(f, "  - {}", interface_report)?;
                }
                Ok(())
            }
            HostReport::Removed {
                db_host,
                interface_reports,
            } => {
                writeln!(
                    f,
                    " {} {}",
                    "Removed Host:".red().bold(),
                    db_host.server_name.red()
                )?;
                for interface_report in interface_reports {
                    writeln!(f, "  - {}", interface_report)?;
                }
                Ok(())
            }
            HostReport::Modified {
                host_yaml,
                fields,
                interface_reports: interfaces,
            } => {
                // header
                writeln!(
                    f,
                    " {} {}",
                    "Modified:".yellow().bold(),
                    host_yaml.server_name.yellow()
                )?;

                let db_report = fields.to_string();
                for line in db_report.lines() {
                    writeln!(f, "{}", line)?;
                }
                for interface_report in interfaces {
                    if !interface_report.is_unchanged() {
                        writeln!(f, "  - {}", interface_report)?;
                    }
                }

                Ok(())
            }

            // ignore unchanged
            _ => Ok(()),
        }
    }
}

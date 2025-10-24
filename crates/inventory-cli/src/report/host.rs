use colored::Colorize;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::fmt;

use crate::prelude::{
    Host, HostYaml, InterfaceReport, InventoryError, ModifiedFields, Reportable, SortOrder, host,
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum HostReport {
    Created {
        host_yaml: HostYaml,
        interface_reports: Vec<InterfaceReport>,
    },
    Modified {
        host_yaml: HostYaml,
        fields: ModifiedFields,
        interface_reports: Vec<InterfaceReport>,
    },
    Removed {
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
            HostReport::Created { .. } => SortOrder::Host as u8,
            HostReport::Removed { .. } => SortOrder::Host as u8 + 1,
            HostReport::Modified { .. } => SortOrder::Host as u8 + 2,
            HostReport::Unchanged { .. } => SortOrder::Host as u8 + 3,
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

    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            HostReport::Created { .. } => self.execute_created(transaction).await,
            HostReport::Modified { .. } => self.execute_modified(transaction).await,
            HostReport::Removed { .. } => self.execute_removed(transaction).await,
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

    pub fn item_name(&self) -> Option<&str> {
        match self {
            HostReport::Created { host_yaml, .. } => Some(&host_yaml.server_name),
            HostReport::Modified { host_yaml, .. } => Some(&host_yaml.server_name),
            HostReport::Removed { db_host, .. } => Some(&db_host.server_name),
            HostReport::Unchanged { host_yaml } => Some(&host_yaml.server_name),
        }
    }

    pub fn execute_unchanged(&self) -> Result<(), InventoryError> {
        if let HostReport::Unchanged { .. } = self {
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Unchanged",
                actual: self.report_name(),
            })
        }
    }

    /// Insert a new host based on the YAML.
    pub async fn execute_created(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let HostReport::Created {
            host_yaml,
            interface_reports,
        } = self
        {
            // insert the host row
            host::create_host(transaction, host_yaml).await?;

            // let interfaces handle themselves
            for iface_r in interface_reports {
                iface_r.execute(transaction).await?;
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
    pub async fn execute_modified(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let HostReport::Modified {
            host_yaml,
            fields: _,
            interface_reports,
        } = self
        {
            // update the host in the database
            host::update_host(transaction, host_yaml).await?;

            // let interfaces handle themselves
            for iface_r in interface_reports {
                iface_r.execute(transaction).await?;
            }

            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Modified",
                actual: self.report_name(),
            })
        }
    }

    /// Delete a host and its interfaces.
    pub async fn execute_removed(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let HostReport::Removed {
            db_host,
            interface_reports,
        } = self
        {
            for iface_r in interface_reports {
                iface_r.execute(transaction).await?;
            }

            // delete the host after its interfaces are taken care of
            host::delete_host_by_name(transaction, &db_host.server_name).await?;

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
                    "  {} {} [{}: {}]",
                    "+".green().bold(),
                    host_yaml.server_name.bright_white().bold(),
                    "interfaces".dimmed(),
                    interface_reports
                        .iter()
                        .filter(|r| !r.is_unchanged())
                        .count()
                )?;

                let active_interfaces: Vec<String> = interface_reports
                    .iter()
                    .filter(|r| !r.is_unchanged())
                    .map(|r| format!("{}", r))
                    .collect();

                if !active_interfaces.is_empty() {
                    write!(f, "      ")?;
                    writeln!(f, "{}", active_interfaces.join(" "))?;
                }
                Ok(())
            }
            HostReport::Removed {
                db_host,
                interface_reports,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "-".red().bold(),
                    db_host.server_name.bright_white().bold()
                )?;
                for interface_report in interface_reports {
                    if !interface_report.is_unchanged() {
                        writeln!(f, "      {}: {}", "interface".dimmed(), interface_report)?;
                    }
                }
                Ok(())
            }
            HostReport::Modified {
                host_yaml,
                fields,
                interface_reports: interfaces,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "~".yellow().bold(),
                    host_yaml.server_name.bright_white().bold()
                )?;

                let db_report = fields.to_string();
                for line in db_report.lines() {
                    writeln!(f, "{}", line)?;
                }
                for interface_report in interfaces {
                    if !interface_report.is_unchanged() {
                        writeln!(f, "      {}: {}", "interface".dimmed(), interface_report)?;
                    }
                }

                Ok(())
            }

            // ignore unchanged
            _ => Ok(()),
        }
    }
}

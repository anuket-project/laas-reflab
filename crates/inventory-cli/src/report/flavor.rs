use crate::{
    handlers::flavor,
    prelude::{FlavorYaml, InventoryError, ModifiedFields, Reportable, SortOrder},
};

use colored::Colorize;
use models::inventory::Flavor;
use sqlx::{Postgres, Transaction};
use std::fmt;

// TODO: move outside of flavor
fn format_bytes(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = KB * 1024;
    const GB: i64 = MB * 1024;
    const TB: i64 = GB * 1024;

    if bytes >= TB {
        format!("{} TB", bytes / TB)
    } else if bytes >= GB {
        format!("{} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{} B", bytes)
    }
}

// TODO: move outside of flavor
fn format_optional_string(opt: &Option<String>) -> &str {
    match opt {
        Some(s) if !s.is_empty() => s.as_str(),
        _ => "?",
    }
}

#[derive(Debug, Clone)]
pub enum FlavorReport {
    Created {
        flavor_yaml: FlavorYaml,
    },
    Removed {
        db_flavor: Flavor,
    },
    Modified {
        flavor_yaml: FlavorYaml,
        modified_fields: ModifiedFields,
    },
    Unchanged {
        name: String,
    },
}

impl FlavorReport {
    pub fn new_created(flavor_yaml: FlavorYaml) -> Self {
        FlavorReport::Created { flavor_yaml }
    }

    pub fn new_modified(flavor_yaml: FlavorYaml, modified_fields: ModifiedFields) -> Self {
        FlavorReport::Modified {
            flavor_yaml,
            modified_fields,
        }
    }

    pub fn new_removed(db_flavor: Flavor) -> Self {
        FlavorReport::Removed { db_flavor }
    }

    pub fn new_unchanged(name: String) -> Self {
        FlavorReport::Unchanged { name }
    }

    pub fn is_unchanged(&self) -> bool {
        matches!(self, FlavorReport::Unchanged { .. })
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            FlavorReport::Created { .. } => "Created",
            FlavorReport::Modified { .. } => "Modified",
            FlavorReport::Removed { .. } => "Removed",
            FlavorReport::Unchanged { .. } => "Unchanged",
        }
    }

    pub fn item_name(&self) -> &str {
        match self {
            FlavorReport::Created { flavor_yaml } => &flavor_yaml.name,
            FlavorReport::Modified { flavor_yaml, .. } => &flavor_yaml.name,
            FlavorReport::Removed { db_flavor } => &db_flavor.name,
            FlavorReport::Unchanged { name } => name,
        }
    }

    pub async fn execute_modified(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let FlavorReport::Modified { flavor_yaml, .. } = self {
            flavor::update_flavor(transaction, flavor_yaml).await?;
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Modified",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_created(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let FlavorReport::Created { flavor_yaml } = self {
            flavor::create_flavor(transaction, flavor_yaml).await?;
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Created",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_removed(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let FlavorReport::Removed { db_flavor } = self {
            flavor::delete_flavor_by_name(transaction, &db_flavor.name).await?;
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Removed",
                actual: self.report_name(),
            })
        }
    }

    pub fn execute_unchanged(&self) -> Result<(), InventoryError> {
        if self.is_unchanged() {
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Unchanged",
                actual: self.report_name(),
            })
        }
    }
}

impl Reportable for FlavorReport {
    fn sort_order(&self) -> u8 {
        match self {
            FlavorReport::Created { .. } => SortOrder::Flavor as u8,
            FlavorReport::Removed { .. } => SortOrder::Flavor as u8 + 1,
            FlavorReport::Modified { .. } => SortOrder::Flavor as u8 + 2,
            FlavorReport::Unchanged { .. } => SortOrder::Flavor as u8 + 3,
        }
    }

    fn is_unchanged(&self) -> bool {
        matches!(self, FlavorReport::Unchanged { .. })
    }
    fn is_created(&self) -> bool {
        matches!(self, FlavorReport::Created { .. })
    }
    fn is_modified(&self) -> bool {
        matches!(self, FlavorReport::Modified { .. })
    }
    fn is_removed(&self) -> bool {
        matches!(self, FlavorReport::Removed { .. })
    }

    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            FlavorReport::Created { .. } => self.execute_created(transaction).await,
            FlavorReport::Modified { .. } => self.execute_modified(transaction).await,
            FlavorReport::Removed { .. } => self.execute_removed(transaction).await,
            FlavorReport::Unchanged { .. } => self.execute_unchanged(),
        }
    }
}

impl fmt::Display for FlavorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlavorReport::Created { flavor_yaml } => {
                write!(
                    f,
                    "  {} {} ",
                    "+".green().bold(),
                    flavor_yaml.name.bright_white().bold()
                )?;

                let mut parts = vec![];

                // description?
                parts.push(format!(
                    "{}?: {}",
                    "description".dimmed(),
                    format_optional_string(&flavor_yaml.description)
                ));

                // brand?
                parts.push(format!(
                    "{}?: {}",
                    "brand".dimmed(),
                    format_optional_string(&flavor_yaml.brand)
                ));

                // model?
                parts.push(format!(
                    "{}?: {}",
                    "model".dimmed(),
                    format_optional_string(&flavor_yaml.model)
                ));

                // arch (required)
                parts.push(format!("{}: {}", "arch".dimmed(), flavor_yaml.arch));

                // cpu_count?
                parts.push(format!(
                    "{}?: {}",
                    "cpu_count".dimmed(),
                    flavor_yaml
                        .cpu_count
                        .map_or("?".to_string(), |v| v.to_string())
                ));

                // cpu_frequency_mhz?
                parts.push(format!(
                    "{}?: {}",
                    "cpu_frequency_mhz".dimmed(),
                    flavor_yaml
                        .cpu_frequency_mhz
                        .map_or("?".to_string(), |v| v.to_string())
                ));

                // cpu_model?
                parts.push(format!(
                    "{}?: {}",
                    "cpu_model".dimmed(),
                    format_optional_string(&flavor_yaml.cpu_model)
                ));

                // ram_bytes?
                parts.push(format!(
                    "{}?: {}",
                    "ram_bytes".dimmed(),
                    flavor_yaml.ram_bytes.map_or("?".to_string(), format_bytes)
                ));

                // root_size_bytes?
                parts.push(format!(
                    "{}?: {}",
                    "root_size_bytes".dimmed(),
                    flavor_yaml
                        .root_size_bytes
                        .map_or("?".to_string(), format_bytes)
                ));

                // disk_size_bytes?
                parts.push(format!(
                    "{}?: {}",
                    "disk_size_bytes".dimmed(),
                    flavor_yaml
                        .disk_size_bytes
                        .map_or("?".to_string(), format_bytes)
                ));

                // storage_type?
                parts.push(format!(
                    "{}?: {}",
                    "storage_type".dimmed(),
                    flavor_yaml
                        .storage_type
                        .as_ref()
                        .map_or("?".to_string(), |v| format!("{:?}", v))
                ));

                // network_speed_mbps?
                parts.push(format!(
                    "{}?: {}",
                    "network_speed_mbps".dimmed(),
                    flavor_yaml
                        .network_speed_mbps
                        .map_or("?".to_string(), |v| v.to_string())
                ));

                // network_interfaces?
                parts.push(format!(
                    "{}?: {}",
                    "network_interfaces".dimmed(),
                    flavor_yaml
                        .network_interfaces
                        .map_or("?".to_string(), |v| v.to_string())
                ));

                writeln!(f, "[{}]", parts.join(", "))
            }
            FlavorReport::Modified {
                flavor_yaml,
                modified_fields,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "~".yellow().bold(),
                    flavor_yaml.name.bright_white().bold()
                )?;
                write!(f, "{}", modified_fields)
            }
            FlavorReport::Removed { db_flavor } => {
                writeln!(
                    f,
                    "  {} {}",
                    "-".red().bold(),
                    db_flavor.name.bright_white().bold()
                )?;
                writeln!(f, "      {}: {}", "arch".dimmed(), db_flavor.arch)?;
                if let Some(cpu_count) = db_flavor.cpu_count {
                    writeln!(f, "      {}: {}", "cpu_count".dimmed(), cpu_count)?;
                }
                Ok(())
            }
            FlavorReport::Unchanged { name } => {
                write!(f, "  {} {}", "=".dimmed(), name.dimmed())
            }
        }
    }
}

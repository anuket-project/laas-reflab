use crate::{
    handlers::lab,
    prelude::{InventoryError, LabYaml, ModifiedFields, Reportable, SortOrder},
};

use colored::Colorize;
use models::inventory::Lab;
use sqlx::{Postgres, Transaction};
use std::fmt;

#[derive(Debug, Clone)]
pub enum LabReport {
    Created {
        lab_yaml: LabYaml,
    },
    Removed {
        db_lab: Lab,
    },
    Modified {
        lab_yaml: LabYaml,
        modified_fields: ModifiedFields,
    },
    Unchanged {
        name: String,
    },
}

impl LabReport {
    pub fn new_created(lab_yaml: LabYaml) -> Self {
        LabReport::Created { lab_yaml }
    }

    pub fn new_modified(lab_yaml: LabYaml, modified_fields: ModifiedFields) -> Self {
        LabReport::Modified {
            lab_yaml,
            modified_fields,
        }
    }

    pub fn new_removed(db_lab: Lab) -> Self {
        LabReport::Removed { db_lab }
    }

    pub fn new_unchanged(name: String) -> Self {
        LabReport::Unchanged { name }
    }

    pub fn is_unchanged(&self) -> bool {
        matches!(self, LabReport::Unchanged { .. })
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            LabReport::Created { .. } => "Created",
            LabReport::Modified { .. } => "Modified",
            LabReport::Removed { .. } => "Removed",
            LabReport::Unchanged { .. } => "Unchanged",
        }
    }

    pub fn item_name(&self) -> &str {
        match self {
            LabReport::Created { lab_yaml } => &lab_yaml.name,
            LabReport::Modified { lab_yaml, .. } => &lab_yaml.name,
            LabReport::Removed { db_lab } => &db_lab.name,
            LabReport::Unchanged { name } => name,
        }
    }

    pub async fn execute_modified(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let LabReport::Modified { lab_yaml, .. } = self {
            lab::update_lab(transaction, lab_yaml).await?;
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
        if let LabReport::Created { lab_yaml } = self {
            lab::create_lab(transaction, lab_yaml).await?;
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
        if let LabReport::Removed { db_lab } = self {
            lab::delete_lab_by_name(transaction, &db_lab.name).await?;
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

impl Reportable for LabReport {
    fn sort_order(&self) -> u8 {
        match self {
            LabReport::Created { .. } => SortOrder::Lab as u8,
            LabReport::Removed { .. } => SortOrder::Lab as u8 + 1,
            LabReport::Modified { .. } => SortOrder::Lab as u8 + 2,
            LabReport::Unchanged { .. } => SortOrder::Lab as u8 + 3,
        }
    }

    fn is_unchanged(&self) -> bool {
        matches!(self, LabReport::Unchanged { .. })
    }
    fn is_created(&self) -> bool {
        matches!(self, LabReport::Created { .. })
    }
    fn is_modified(&self) -> bool {
        matches!(self, LabReport::Modified { .. })
    }
    fn is_removed(&self) -> bool {
        matches!(self, LabReport::Removed { .. })
    }

    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            LabReport::Created { .. } => self.execute_created(transaction).await,
            LabReport::Modified { .. } => self.execute_modified(transaction).await,
            LabReport::Removed { .. } => self.execute_removed(transaction).await,
            LabReport::Unchanged { .. } => self.execute_unchanged(),
        }
    }
}

impl fmt::Display for LabReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabReport::Created { lab_yaml } => {
                write!(
                    f,
                    "  {} {} ",
                    "+".green().bold(),
                    lab_yaml.name.bright_white().bold()
                )?;

                let mut parts = vec![];

                parts.push(format!(
                    "{}: {}",
                    "location".dimmed(),
                    lab_yaml.location
                ));

                parts.push(format!(
                    "{}: {}",
                    "email".dimmed(),
                    lab_yaml.email
                ));

                parts.push(format!(
                    "{}: {}",
                    "phone".dimmed(),
                    lab_yaml.phone
                ));

                parts.push(format!(
                    "{}: {}",
                    "is_dynamic".dimmed(),
                    lab_yaml.is_dynamic
                ));

                writeln!(f, "[{}]", parts.join(", "))
            }
            LabReport::Modified {
                lab_yaml,
                modified_fields,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "~".yellow().bold(),
                    lab_yaml.name.bright_white().bold()
                )?;
                write!(f, "{}", modified_fields)
            }
            LabReport::Removed { db_lab } => {
                writeln!(
                    f,
                    "  {} {}",
                    "-".red().bold(),
                    db_lab.name.bright_white().bold()
                )?;
                writeln!(f, "      {}: {}", "location".dimmed(), db_lab.location)?;
                writeln!(f, "      {}: {}", "email".dimmed(), db_lab.email)?;
                writeln!(f, "      {}: {}", "phone".dimmed(), db_lab.phone)?;
                writeln!(f, "      {}: {}", "is_dynamic".dimmed(), db_lab.is_dynamic)?;
                Ok(())
            }
            LabReport::Unchanged { name } => {
                write!(f, "  {} {}", "=".dimmed(), name.dimmed())
            }
        }
    }
}

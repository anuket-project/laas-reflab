use crate::{
    handlers::image,
    prelude::{ImageYaml, InventoryError, KernelArgReport, ModifiedFields, Reportable, SortOrder},
};
use colored::Colorize;
use models::dashboard::Image;
use sqlx::{Postgres, Transaction};
use std::fmt;

#[derive(Debug, Clone)]
pub enum ImageReport {
    Created {
        image_yaml: ImageYaml,
        kernel_arg_reports: Vec<KernelArgReport>,
    },
    Removed {
        db_image: Image,
        kernel_arg_reports: Vec<KernelArgReport>,
    },
    Modified {
        image_yaml: ImageYaml,
        modified_fields: ModifiedFields,
        kernel_arg_reports: Vec<KernelArgReport>,
    },
    Unchanged {
        name: String,
    },
}

impl Reportable for ImageReport {
    fn sort_order(&self) -> u8 {
        match self {
            ImageReport::Created { .. } => SortOrder::Image as u8,
            ImageReport::Removed { .. } => SortOrder::Image as u8 + 1,
            ImageReport::Modified { .. } => SortOrder::Image as u8 + 2,
            ImageReport::Unchanged { .. } => SortOrder::Image as u8 + 3,
        }
    }

    fn is_unchanged(&self) -> bool {
        matches!(self, ImageReport::Unchanged { .. })
    }
    fn is_created(&self) -> bool {
        matches!(self, ImageReport::Created { .. })
    }
    fn is_modified(&self) -> bool {
        matches!(self, ImageReport::Modified { .. })
    }
    fn is_removed(&self) -> bool {
        matches!(self, ImageReport::Removed { .. })
    }

    async fn execute(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        match self {
            ImageReport::Created { .. } => self.execute_created(transaction).await,
            ImageReport::Modified { .. } => self.execute_modified(transaction).await,
            ImageReport::Removed { .. } => self.execute_removed(transaction).await,
            ImageReport::Unchanged { .. } => self.execute_unchanged(),
        }
    }
}

impl ImageReport {
    pub fn new_created(image_yaml: ImageYaml, kernel_arg_reports: Vec<KernelArgReport>) -> Self {
        ImageReport::Created {
            image_yaml,
            kernel_arg_reports,
        }
    }

    pub fn new_modified(
        image_yaml: ImageYaml,
        modified_fields: ModifiedFields,
        kernel_arg_reports: Vec<KernelArgReport>,
    ) -> Self {
        ImageReport::Modified {
            image_yaml,
            modified_fields,
            kernel_arg_reports,
        }
    }

    pub fn new_removed(db_image: Image, kernel_arg_reports: Vec<KernelArgReport>) -> Self {
        ImageReport::Removed {
            db_image,
            kernel_arg_reports,
        }
    }

    pub fn new_unchanged(name: String) -> Self {
        ImageReport::Unchanged { name }
    }

    pub fn report_name(&self) -> &'static str {
        match self {
            ImageReport::Created { .. } => "Created",
            ImageReport::Modified { .. } => "Modified",
            ImageReport::Removed { .. } => "Removed",
            ImageReport::Unchanged { .. } => "Unchanged",
        }
    }

    pub fn item_name(&self) -> &str {
        match self {
            ImageReport::Created { image_yaml, .. } => &image_yaml.name,
            ImageReport::Modified { image_yaml, .. } => &image_yaml.name,
            ImageReport::Removed { db_image, .. } => &db_image.name,
            ImageReport::Unchanged { name } => name,
        }
    }

    pub fn execute_unchanged(&self) -> Result<(), InventoryError> {
        if let ImageReport::Unchanged { .. } = self {
            // TODO:
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Unchanged",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_created(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let ImageReport::Created { image_yaml, .. } = self {
            image::create_image(transaction, image_yaml).await?;
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Created",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_modified(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let ImageReport::Modified { image_yaml, .. } = self {
            image::update_image(transaction, image_yaml).await?;
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Modified",
                actual: self.report_name(),
            })
        }
    }

    pub async fn execute_removed(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<(), InventoryError> {
        if let ImageReport::Removed { db_image, .. } = self {
            image::delete_image_by_name(transaction, &db_image.name).await?;
            Ok(())
        } else {
            Err(InventoryError::InvalidReportType {
                expected: "Removed",
                actual: self.report_name(),
            })
        }
    }

    pub fn push_kernel_arg_report(&mut self, report: KernelArgReport) {
        match self {
            ImageReport::Created {
                kernel_arg_reports, ..
            } => {
                kernel_arg_reports.push(report);
            }
            ImageReport::Modified {
                kernel_arg_reports, ..
            } => {
                kernel_arg_reports.push(report);
            }
            ImageReport::Removed {
                kernel_arg_reports, ..
            } => {
                kernel_arg_reports.push(report);
            }
            ImageReport::Unchanged { name } => {
                panic!(
                    "Cannot push kernel arg report to an unchanged image report ({})",
                    name
                );
            }
        }
    }
}

impl fmt::Display for ImageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageReport::Created {
                image_yaml,
                kernel_arg_reports,
            } => {
                write!(
                    f,
                    "  {} {} ",
                    "+".green().bold(),
                    image_yaml.name.bright_white().bold()
                )?;

                let mut parts = vec![
                    format!("{}: {}", "distro".dimmed(), image_yaml.distro),
                    format!("{}: {}", "version".dimmed(), image_yaml.version),
                    format!("{}: {}", "arch".dimmed(), image_yaml.arch),
                    format!("{}: {}", "flavors".dimmed(), image_yaml.flavors.len()),
                ];

                if !kernel_arg_reports.is_empty() {
                    parts.push(format!(
                        "{}: {}",
                        "kernel_args".dimmed(),
                        kernel_arg_reports.len()
                    ));
                }

                writeln!(f, "[{}]", parts.join(", "))?;

                // display kernel_args
                if !kernel_arg_reports.is_empty() {
                    write!(f, "      ")?;
                    let args: Vec<String> = kernel_arg_reports
                        .iter()
                        .map(|r| format!("{}", r))
                        .collect();
                    writeln!(f, "{}", args.join(" "))?;
                }
                Ok(())
            }
            ImageReport::Modified {
                image_yaml,
                modified_fields,
                kernel_arg_reports,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "~".yellow().bold(),
                    image_yaml.name.bright_white().bold()
                )?;
                write!(f, "{}", modified_fields)?;
                if !kernel_arg_reports.is_empty() {
                    writeln!(f, "      {}:", "kernel_args".dimmed())?;
                    for kernel_arg_report in kernel_arg_reports {
                        writeln!(f, "        {}", kernel_arg_report)?;
                    }
                }
                Ok(())
            }
            ImageReport::Removed {
                db_image,
                kernel_arg_reports,
            } => {
                writeln!(
                    f,
                    "  {} {}",
                    "-".red().bold(),
                    db_image.name.bright_white().bold()
                )?;
                writeln!(f, "      {}: {}", "distro".dimmed(), db_image.distro)?;
                writeln!(f, "      {}: {}", "version".dimmed(), db_image.version)?;
                writeln!(f, "      {}: {}", "arch".dimmed(), db_image.arch)?;
                if !kernel_arg_reports.is_empty() {
                    writeln!(
                        f,
                        "      {} {} kernel_args",
                        "removing".dimmed(),
                        kernel_arg_reports.len()
                    )?;
                }
                Ok(())
            }
            ImageReport::Unchanged { name } => {
                write!(f, "  {} {}", "=".dimmed(), name.dimmed())
            }
        }
    }
}

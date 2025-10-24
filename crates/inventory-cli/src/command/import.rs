use std::io::{self, Write};

use crate::{
    command::print_reports,
    prelude::{InventoryError, Reportable, generate_reports, get_db_pool},
};

/// Import inventory from YAML files into the database
///
/// Process
/// - Connects to database
/// - Generates reports by comparing YAML with DB state
/// - Displays formatted diff to the user
/// - Executes reports according to `Reportable` impl
///
/// # Arguments
///
/// * `dir` - Path to directory containing inventory YAML files
/// * `auto_yes` - Whether to skip confirmation prompts (use --yes flag)
///
/// # Errors
///
/// Returns an error if:
/// - Database connection fails
/// - YAML parsing/loading fails
/// - Any database operation fails during execution
pub async fn import_inventory(
    dir: &str,
    auto_yes: bool,
    verbose: bool,
) -> Result<(), InventoryError> {
    use crate::prelude::Report;
    use colored::Colorize;
    use indicatif::{ProgressBar, ProgressStyle};

    let pool = get_db_pool().await?;
    let reports = generate_reports(dir, verbose).await?;

    print_reports(&reports);

    let has_changes = reports.iter().any(|r| !r.is_unchanged());
    if !has_changes {
        println!("\n{}", "No changes to apply.".dimmed());
        return Ok(());
    }

    // prompt for confirmation
    if !auto_yes {
        print!("\n{} ", "Apply these changes? [y/N]:".yellow().bold());
        io::stdout().flush().map_err(InventoryError::StdoutFlush)?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(InventoryError::StdinRead)?;

        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("{}", "Import cancelled.".dimmed());
            return Ok(());
        }
    }

    let total = reports.len() as u64;

    println!("\n{}", "Applying changes...".bold());

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut transaction = pool.begin().await.map_err(|e| InventoryError::Sqlx {
        context: "Failed to begin transaction".to_string(),
        source: e,
    })?;

    let execute_result: Result<(), InventoryError> = async {
        for report in reports {
            // set status message
            let msg = match &report {
                Report::LabReport(r) => {
                    let name = r.item_name();
                    if r.is_created() {
                        format!("Creating lab '{}'", name)
                    } else if r.is_modified() {
                        format!("Updating lab '{}'", name)
                    } else if r.is_removed() {
                        format!("Removing lab '{}'", name)
                    } else {
                        String::new()
                    }
                }
                Report::FlavorReport(r) => {
                    let name = r.item_name();
                    if r.is_created() {
                        format!("Creating flavor '{}'", name)
                    } else if r.is_modified() {
                        format!("Updating flavor '{}'", name)
                    } else if r.is_removed() {
                        format!("Removing flavor '{}'", name)
                    } else {
                        String::new()
                    }
                }
                Report::ImageReport(r) => {
                    let name = r.item_name();
                    if r.is_created() {
                        format!("Creating image '{}'", name)
                    } else if r.is_modified() {
                        format!("Updating image '{}'", name)
                    } else if r.is_removed() {
                        format!("Removing image '{}'", name)
                    } else {
                        String::new()
                    }
                }
                Report::SwitchReport(r) => {
                    if let Some(name) = r.item_name() {
                        if r.is_created() {
                            format!("Creating switch '{}'", name)
                        } else if r.is_modified() {
                            format!("Updating switch '{}'", name)
                        } else if r.is_removed() {
                            format!("Removing switch '{}'", name)
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                }
                Report::HostReport(r) => {
                    if let Some(name) = r.item_name() {
                        if r.is_created() {
                            format!("Creating host '{}'", name)
                        } else if r.is_modified() {
                            format!("Updating host '{}'", name)
                        } else if r.is_removed() {
                            format!("Removing host '{}'", name)
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                }
            };

            pb.set_message(msg);
            report.execute(&mut transaction).await?;
            pb.inc(1);
        }
        Ok(())
    }
    .await;

    match execute_result {
        Ok(_) => {
            transaction
                .commit()
                .await
                .map_err(|e| InventoryError::Sqlx {
                    context: "Failed to commit transaction".to_string(),
                    source: e,
                })?;
            pb.finish_and_clear();
            println!("{}", "Import complete.".green().bold());
            Ok(())
        }
        Err(e) => {
            transaction.rollback().await.map_err(|rollback_err| {
                InventoryError::TransactionRollback {
                    rollback_error: rollback_err.to_string(),
                    original_error: e.to_string(),
                }
            })?;
            pb.finish_and_clear();
            Err(e)
        }
    }
}

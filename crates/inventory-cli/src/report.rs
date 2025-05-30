use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{self, Write},
};

use crate::prelude::ModifiedFields;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum Report {
    Created {
        server_name: String,
    },
    Modified {
        server_name: String,
        fields: ModifiedFields,
    },
    Removed {
        server_name: String,
    },
    Unchanged {
        server_name: String,
    },
}

impl Report {
    pub fn new_created(server_name: String) -> Self {
        Report::Created { server_name }
    }

    pub fn new_modified(server_name: String, fields: ModifiedFields) -> Self {
        Report::Modified {
            server_name,
            fields,
        }
    }

    pub fn new_removed(server_name: String) -> Self {
        Report::Removed { server_name }
    }

    pub fn new_unchanged(server_name: String) -> Self {
        Report::Unchanged { server_name }
    }

    pub fn report_sort_order(&self) -> u8 {
        match self {
            Report::Created { .. } => 0,
            Report::Removed { .. } => 1,
            Report::Modified { .. } => 2,
            Report::Unchanged { .. } => 3,
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Report::Created { server_name } => {
                write!(f, " {} {}", "Created:".green().bold(), server_name.green())
            }
            Report::Removed { server_name } => {
                write!(f, " {} {}", "Removed:".red().bold(), server_name.red())
            }
            Report::Modified {
                server_name,
                fields,
            } => {
                // header
                writeln!(
                    f,
                    " {} {}",
                    "Modified:".yellow().bold(),
                    server_name.yellow()
                )?;

                let db_report = fields.to_string();
                for line in db_report.lines() {
                    writeln!(f, "{}", line)?;
                }

                Ok(())
            }

            // ignore unchanged
            _ => Ok(()),
        }
    }
}

pub struct ReportSummary {
    pub has_changes: bool,
    pub unchanged: Vec<String>,
}

pub fn print_reports(reports: &[Report], detailed: bool) -> ReportSummary {
    println!("{}", "Inventory Diff:".white().bold().underline());

    let mut unchanged_hosts: Vec<String> = reports
        .iter()
        .filter_map(|r| {
            if let Report::Unchanged { server_name } = r {
                Some(server_name.clone())
            } else {
                None
            }
        })
        .collect();

    let has_changes = reports
        .iter()
        .any(|r| !matches!(r, Report::Unchanged { .. }));

    if !has_changes {
        println!("{}", "No changes detected".dimmed());
        return ReportSummary {
            has_changes: false,
            unchanged: unchanged_hosts,
        };
    }

    for report in reports {
        if !matches!(report, Report::Unchanged { .. }) {
            println!("{}", report);
        }
    }

    if detailed && !unchanged_hosts.is_empty() {
        unchanged_hosts.sort();
        println!(
            " {} {} hosts were unchanged:",
            "Unchanged:".cyan().bold(),
            unchanged_hosts.len()
        );
        for host in &unchanged_hosts {
            println!("  - {}", host.dimmed());
        }
    }

    ReportSummary {
        has_changes: true,
        unchanged: unchanged_hosts,
    }
}
pub fn confirm_and_proceed(summary: ReportSummary, auto_yes: bool) -> bool {
    if !summary.has_changes {
        return false;
    }

    if auto_yes {
        return true;
    }

    print!("{}", "Apply changes? [y/N]: ".cyan().bold());

    io::stdout().flush().unwrap();

    let mut input = String::new();

    if io::stdin().read_line(&mut input).is_err() {
        println!("Failed to read input.");
        return false;
    }

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

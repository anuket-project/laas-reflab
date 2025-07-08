use enum_dispatch::enum_dispatch;
use sqlx::PgPool;
use std::fmt::{self, Display};

mod host;
mod interface;
mod switch;
mod switchport;

pub use host::HostReport;
pub use interface::InterfaceReport;
pub use switch::SwitchReport;
pub use switchport::SwitchportReport;

use crate::prelude::InventoryError;

#[enum_dispatch]
pub trait Reportable {
    fn sort_order(&self) -> u8;

    fn is_unchanged(&self) -> bool {
        false
    }
    fn is_created(&self) -> bool {
        false
    }
    fn is_modified(&self) -> bool {
        false
    }
    fn is_removed(&self) -> bool {
        false
    }

    #[allow(async_fn_in_trait)]
    async fn execute(&self, _pool: &PgPool) -> Result<(), InventoryError> {
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
}

impl Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Report::HostReport(report) => write!(f, "{}", report),
            Report::SwitchReport(report) => write!(f, "{}", report),
        }
    }
}

// pub struct ReportSummary {
//     pub has_changes: bool,
//     pub unchanged: Vec<String>,
// }
//
// pub fn print_reports(reports: &[Report], detailed: bool) -> ReportSummary {
//     println!("{}", "Inventory Diff:".white().bold().underline());
//
//     let mut unchanged_hosts: Vec<String> = reports
//         .iter()
//         .filter_map(|r| {
//             if let Report::HostReport(HostReport::Unchanged { server_name }) = r {
//                 Some(server_name.clone())
//             } else {
//                 None
//             }
//         })
//         .collect();
//
//     let has_changes = reports
//         .iter()
//         .any(|r| !matches!(r, Report::HostReport(HostReport::Unchanged { .. })));
//
//     if !has_changes {
//         println!("{}", "No changes detected".dimmed());
//         return ReportSummary {
//             has_changes: false,
//             unchanged: unchanged_hosts,
//         };
//     }
//
//     for report in reports {
//         if !matches!(report, Report::HostReport(HostReport::Unchanged { .. })) {
//             println!("{}", report);
//         }
//     }
//
//     if detailed && !unchanged_hosts.is_empty() {
//         unchanged_hosts.sort();
//         println!(
//             " {} {} hosts were unchanged:",
//             "Unchanged:".cyan().bold(),
//             unchanged_hosts.len()
//         );
//         for host in &unchanged_hosts {
//             println!("  - {}", host.dimmed());
//         }
//     }
//
//     ReportSummary {
//         has_changes: true,
//         unchanged: unchanged_hosts,
//     }
// }
// pub fn confirm_and_proceed(summary: ReportSummary, auto_yes: bool) -> bool {
//     if !summary.has_changes {
//         return false;
//     }
//
//     if auto_yes {
//         return true;
//     }
//
//     print!("{}", "Apply changes? [y/N]: ".cyan().bold());
//
//     io::stdout().flush().unwrap();
//
//     let mut input = String::new();
//
//     if io::stdin().read_line(&mut input).is_err() {
//         println!("Failed to read input.");
//         return false;
//     }
//
//     matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
// }

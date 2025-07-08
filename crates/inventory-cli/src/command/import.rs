use colored::Colorize;
use std::io::{self, Write};

use crate::prelude::{InventoryError, Reportable, generate_reports, get_db_pool};

use super::print_reports;

pub async fn import_inventory(
    dir: &str,
    _auto_yes: bool,
    _detailed: bool,
    _ignore_errors: bool,
) -> Result<(), InventoryError> {
    let pool = get_db_pool().await?;
    let reports = generate_reports(dir).await?;

    print_reports(&reports);

    for report in reports {
        report.execute(&pool).await?;
    }

    Ok(())
}

#[allow(dead_code)]
/// Returns `true` if we should continue to the next host,
/// or `false` if we should abort import entirely.
fn handle_import_error(err: InventoryError, ignore_errors: bool) -> bool {
    eprintln!("{}", format!("{}", err).red());

    if ignore_errors {
        return true;
    }

    print!("{}", "Continue with next host? [y/N]: ".cyan().bold());
    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

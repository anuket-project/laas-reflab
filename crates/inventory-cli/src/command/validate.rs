use crate::prelude::{InventoryError, generate_reports};

use super::print_reports;

/// Validate inventory changes without applying them
///
/// This command performs all the same analysis as `import` but does not
/// execute any database operations. It's useful for:
/// - Previewing changes before applying them
/// - Checking YAML syntax and validity
/// - Verifying that references (flavors, switches, etc.) are correct
///
/// # Arguments
///
/// * `dir` - Path to directory containing inventory YAML files
/// * `verbose` - Debug output
///
/// # Errors
///
/// Returns an error if:
/// - YAML files cannot be parsed/loaded
/// - Database connection fails
/// - Invalid references are found in YAML
pub async fn validate_inventory(dir: &str, verbose: bool) -> Result<(), InventoryError> {
    let reports = generate_reports(dir, verbose).await?;
    print_reports(&reports);
    Ok(())
}

use crate::prelude::{InventoryError, generate_reports};

use super::print_reports;

pub async fn validate_inventory(dir: &str, _detailed: bool) -> Result<(), InventoryError> {
    let reports = generate_reports(dir).await?;

    print_reports(&reports);
    // we don't need to confirm anything since validate is just a dry run
    // let _summary = print_reports(&reports, detailed);
    Ok(())
}

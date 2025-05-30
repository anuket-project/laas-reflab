use crate::prelude::{InventoryError, generate_reports, print_reports};

pub async fn validate_inventory(dir: &str, detailed: bool) -> Result<(), InventoryError> {
    let reports = generate_reports(dir).await?;

    // we don't need to confirm anything since validate is just a dry run
    let _summary = print_reports(&reports, detailed);
    Ok(())
}

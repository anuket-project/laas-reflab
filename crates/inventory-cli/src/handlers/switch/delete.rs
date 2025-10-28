use sqlx::{Postgres, Transaction};

#[allow(unused_imports)]
use crate::prelude::{InventoryError, Switch};

/// Delete a [`Switch`] by its name.
pub async fn delete_switch_by_name(
    transaction: &mut Transaction<'_, Postgres>,
    switch: &Switch,
) -> Result<(), InventoryError> {
    sqlx::query!(r#"DELETE FROM switches WHERE name = $1"#, switch.name)
        .execute(&mut **transaction)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: format!("Deleting switch `{}`", switch.name),
            source: e,
        })?;
    Ok(())
}

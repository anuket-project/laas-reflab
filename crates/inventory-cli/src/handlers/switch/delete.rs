use sqlx::PgPool;

#[allow(unused_imports)]
use crate::prelude::{InventoryError, Switch};

/// Delete a [`Switch`] by its name.
pub async fn delete_switch_by_name(pool: &PgPool, switch: &Switch) -> Result<(), InventoryError> {
    sqlx::query!(r#"DELETE FROM switches WHERE name = $1"#, switch.name)
        .execute(pool)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: format!("Deleting switch `{}`", switch.name),
            source: e,
        })?;
    Ok(())
}

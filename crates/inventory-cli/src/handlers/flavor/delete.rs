use crate::prelude::InventoryError;

use sqlx::{Postgres, Transaction};

pub async fn delete_flavor_by_name(
    transaction: &mut Transaction<'_, Postgres>,
    flavor_name: &str,
) -> Result<(), InventoryError> {
    let flavor = sqlx::query!(
        r#"
        SELECT id FROM flavors WHERE name = $1 AND deleted = false
"#,
        flavor_name
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While fetching flavor '{}'", flavor_name),
        source: e,
    })?
    .ok_or_else(|| {
        InventoryError::NotFound(format!(
            "Flavor '{}' not found or already deleted",
            flavor_name
        ))
    })?;

    sqlx::query!(
        r#"UPDATE flavors SET deleted = true WHERE id = $1"#,
        flavor.id
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While soft-deleting flavor".into(),
        source: e,
    })?;

    Ok(())
}

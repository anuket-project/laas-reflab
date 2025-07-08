use crate::prelude::InventoryError;

use sqlx::PgPool;

pub async fn delete_host_by_name(pool: &PgPool, server_name: &str) -> Result<(), InventoryError> {
    let host = sqlx::query!(
        r#"
        SELECT id
        FROM hosts
        WHERE server_name = $1
          AND deleted = false
        "#,
        server_name
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While fetching host `{}`", server_name),
        source: e,
    })?
    .ok_or_else(|| {
        InventoryError::NotFound(format!(
            "Host '{}' not found or already deleted",
            server_name
        ))
    })?;

    sqlx::query!(
        r#"
        UPDATE hosts
        SET deleted = true
        WHERE id = $1
        "#,
        host.id
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While soft‐deleting host".into(),
        source: e,
    })?;

    Ok(())
}

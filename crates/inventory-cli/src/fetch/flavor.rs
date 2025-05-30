use sqlx::PgPool;
use uuid::Uuid;

use crate::prelude::InventoryError;

pub async fn fetch_flavor_name(pool: &PgPool, flavor_id: &Uuid) -> Result<String, InventoryError> {
    let row = sqlx::query_scalar!(
        r#"
        SELECT name
          FROM flavors
         WHERE id = $1
        "#,
        flavor_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While fetching flavor name for id {}", flavor_id),
        source: e,
    })?;

    Ok(row)
}

use sqlx::PgPool;
use std::collections::HashMap;

use models::inventory::Lab;

use crate::prelude::InventoryError;

pub async fn fetch_lab_map(pool: &PgPool) -> Result<HashMap<String, Lab>, InventoryError> {
    let labs = sqlx::query_as!(
        Lab,
        r#"
        SELECT
            id as "id: dal::FKey<Lab>",
            name,
            location,
            email,
            phone,
            is_dynamic
        FROM labs
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching lab_map".to_string(),
        source: e,
    })?;

    Ok(labs.into_iter().map(|l| (l.name.clone(), l)).collect())
}

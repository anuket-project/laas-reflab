use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use models::inventory::{Arch, Flavor, StorageType};

use crate::prelude::InventoryError;

pub async fn fetch_flavor_name_by_id(
    pool: &PgPool,
    flavor_id: &Uuid,
) -> Result<String, InventoryError> {
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

pub async fn fetch_flavor_map(pool: &PgPool) -> Result<HashMap<String, Flavor>, InventoryError> {
    let flavors = sqlx::query_as!(
        Flavor,
        r#"
        SELECT
            id as "id: dal::FKey<Flavor>",
            name,
            description,
            arch as "arch: Arch",
            cpu_count,
            cpu_frequency_mhz,
            cpu_model,
            ram_bytes,
            root_size_bytes,
            disk_size_bytes,
            storage_type as "storage_type: StorageType",
            network_speed_mbps,
            network_interfaces,
            brand,
            model
        FROM flavors
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching flavor_map".to_string(),
        source: e,
    })?;

    Ok(flavors.into_iter().map(|f| (f.name.clone(), f)).collect())
}

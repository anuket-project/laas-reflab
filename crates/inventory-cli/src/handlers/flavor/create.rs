use crate::prelude::{FlavorYaml, InventoryError};

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub async fn create_flavor(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &FlavorYaml,
) -> Result<(), InventoryError> {
    let id = Uuid::new_v4();

    sqlx::query!(
        r#"
            INSERT INTO flavors (
                id,
                name,
                arch,
                brand,
                model,
                cpu_count,
                description,
                cpu_frequency_mhz,
                cpu_model,
                ram_bytes,
                root_size_bytes,
                disk_size_bytes,
                storage_type,
                network_speed_mbps,
                network_interfaces,
                deleted
            )
            VALUES (
                $1, $2, $3::text::arch, $4, $5, $6, $7, $8,
                $9, $10, $11, $12, $13::text::storage_type, $14, $15, $16
            )
        "#,
        id,
        yaml.name,
        yaml.arch.to_string(),
        yaml.brand,
        yaml.model,
        yaml.cpu_count,
        yaml.description,
        yaml.cpu_frequency_mhz,
        yaml.cpu_model,
        yaml.ram_bytes,
        yaml.root_size_bytes,
        yaml.disk_size_bytes,
        yaml.storage_type.map(|st| st.to_string()),
        yaml.network_speed_mbps,
        yaml.network_interfaces,
        false
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While inserting new flavor".to_string(),
        source: e,
    })?;

    Ok(())
}

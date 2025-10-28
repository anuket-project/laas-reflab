use crate::prelude::{FlavorYaml, InventoryError};

use sqlx::{Postgres, Transaction};

pub async fn update_flavor(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &FlavorYaml,
) -> Result<(), InventoryError> {
    sqlx::query!(
        r#"
        UPDATE flavors
        SET
            arch = $2::text::arch,
            brand = $3,
            model = $4,
            cpu_count = $5,
            description = $6,
            cpu_frequency_mhz = $7,
            cpu_model = $8,
            ram_bytes = $9,
            root_size_bytes = $10,
            disk_size_bytes = $11,
            storage_type = $12::text::storage_type,
            network_speed_mbps = $13,
            network_interfaces = $14,
            deleted = false
        WHERE name = $1;
        "#,
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
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While updating flavor record".into(),
        source: e,
    })?;

    Ok(())
}

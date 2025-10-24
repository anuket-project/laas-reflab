use crate::prelude::{ImageYaml, InventoryError, KernelArg};

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub async fn create_image(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &ImageYaml,
) -> Result<(), InventoryError> {
    let id = Uuid::new_v4();

    // lookup flavor uuids from names
    let mut flavor_uuids: Vec<Uuid> = Vec::new();
    for flavor_name in &yaml.flavors {
        let uuid = sqlx::query_scalar!(
            "SELECT id FROM flavors WHERE name = $1 AND deleted = false",
            flavor_name
        )
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: format!("While looking up flavor '{}'", flavor_name),
            source: e,
        })?
        .ok_or_else(|| InventoryError::NotFound(format!("Flavor '{}' not found", flavor_name)))?;
        flavor_uuids.push(uuid);
    }

    let tftp_initrd_paths: Vec<String> = yaml
        .tftp_initrd_paths
        .iter()
        .map(|uri| uri.to_string())
        .collect();

    sqlx::query!(
        r#"
        INSERT INTO images (
                id,
                name,
                cobbler_name,
                deleted,
                flavors,
                distro,
                version,
                arch,
                http_unattended_install_config_path,
                http_iso_path,
                tftp_kernel_path,
                tftp_initrd_paths
        )
        VALUES (
            $1, $2, $3, $4, $5, $6::text::distro, $7, $8::text::arch, $9, $10, $11, $12
        )
        "#,
        id,
        yaml.name,
        yaml.cobbler_name,
        false,
        &flavor_uuids[..],
        yaml.distro.to_string(),
        yaml.version,
        yaml.arch.to_string(),
        yaml.http_unattended_install_config_path.to_string(),
        yaml.http_iso_path.to_string(),
        yaml.tftp_kernel_path.to_string(),
        &tftp_initrd_paths[..],
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While inserting new image".to_string(),
        source: e,
    })?;

    for kernel_arg in &yaml.kernel_args {
        match kernel_arg {
            KernelArg::Flag(flag) => {
                sqlx::query!(
                    r#"
                    INSERT INTO image_kernel_args (for_image, _key, _value)
                    VALUES ($1, $2, NULL)
                    "#,
                    id,
                    flag
                )
                .execute(&mut **transaction)
                .await
                .map_err(|e| InventoryError::Sqlx {
                    context: format!("While inserting kernel arg flag '{}'", flag),
                    source: e,
                })?;
            }
            KernelArg::KeyValue { key, value } => {
                sqlx::query!(
                    r#"
                    INSERT INTO image_kernel_args (for_image, _key, _value)
                    VALUES ($1, $2, $3)
                    "#,
                    id,
                    key,
                    value
                )
                .execute(&mut **transaction)
                .await
                .map_err(|e| InventoryError::Sqlx {
                    context: format!("While inserting kernel arg '{}={}'", key, value),
                    source: e,
                })?;
            }
        }
    }

    Ok(())
}

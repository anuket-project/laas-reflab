use crate::prelude::{ImageYaml, InventoryError, KernelArg};

use sqlx::{Postgres, Transaction};

pub async fn update_image(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &ImageYaml,
) -> Result<(), InventoryError> {
    let tftp_initrd_paths: Vec<String> = yaml
        .tftp_initrd_paths
        .iter()
        .map(|uri| uri.to_string())
        .collect();

    let image_id = sqlx::query_scalar!("SELECT id FROM images WHERE name = $1", yaml.name)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: format!("While fetching image ID for '{}'", yaml.name),
            source: e,
        })?;

    sqlx::query!(
        r#"
            UPDATE images
            SET
                cobbler_name = $2,
                flavors = (
                    SELECT array_agg(id::uuid)
                    FROM flavors
                    WHERE name = ANY($3) AND deleted = false
                ),
                distro = $4::text::distro,
                version = $5,
                arch = $6::text::arch,
                http_unattended_install_config_path = $7,
                http_iso_path = $8,
                tftp_kernel_path = $9,
                tftp_initrd_paths = $10
            WHERE name = $1;
        "#,
        yaml.name,
        yaml.cobbler_name,
        &yaml.flavors[..],
        yaml.distro.to_string(),
        yaml.version,
        yaml.arch.to_string(),
        yaml.http_unattended_install_config_path
            .as_ref()
            .map(|u| u.to_string()),
        yaml.http_iso_path.as_ref().map(|u| u.to_string()),
        yaml.tftp_kernel_path.to_string(),
        &tftp_initrd_paths[..],
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While updating image".to_string(),
        source: e,
    })?;

    // delete all existing kernel args for image
    sqlx::query!(
        "DELETE FROM image_kernel_args WHERE for_image = $1",
        image_id
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While deleting kernel args for image '{}'", yaml.name),
        source: e,
    })?;

    // insert new kernel_args
    for kernel_arg in &yaml.kernel_args {
        match kernel_arg {
            KernelArg::Flag(flag) => {
                sqlx::query!(
                    r#"
                    INSERT INTO image_kernel_args (for_image, _key, _value)
                    VALUES ($1, $2, NULL)
                    "#,
                    image_id,
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
                    image_id,
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

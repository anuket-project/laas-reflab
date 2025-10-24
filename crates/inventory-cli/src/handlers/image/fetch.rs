use dal::*;
use http::Uri;
use models::dashboard::ImageKernelArg;
use models::dashboard::image::Distro;
use models::inventory::Arch;
use sqlx::PgPool;
use std::collections::HashMap;

use crate::prelude::InventoryError;
use models::dashboard::Image;

pub async fn fetch_image_map(pool: &PgPool) -> Result<HashMap<String, Image>, InventoryError> {
    let rows = sqlx::query!(
        r#"
        SELECT
            id,
            name,
            deleted,
            flavors,
            distro as "distro: Distro",
            version,
            cobbler_name,
            arch as "arch: Arch",
            http_unattended_install_config_path as "http_unattended_install_config_path!",
            http_iso_path as "http_iso_path!",
            tftp_kernel_path as "tftp_kernel_path!",
            tftp_initrd_paths as "tftp_initrd_paths!"
        FROM images
        WHERE deleted = false
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching images".to_string(),
        source: e,
    })?;

    let mut images = HashMap::new();
    for row in rows {
        let flavors: Vec<FKey<models::inventory::Flavor>> = row
            .flavors
            .into_iter()
            .map(|uuid| FKey::from_id(ID::from(uuid)))
            .collect();

        let tftp_initrd_paths: Vec<Uri> = row
            .tftp_initrd_paths
            .into_iter()
            .filter_map(|s| s.parse().ok())
            .collect();

        let image = Image {
            id: FKey::from_id(ID::from(row.id)),
            name: row.name.clone(),
            deleted: row.deleted,
            flavors,
            distro: row.distro,
            version: row.version,
            arch: row.arch,
            cobbler_name: row.cobbler_name,
            http_unattended_install_config_path: row
                .http_unattended_install_config_path
                .parse()
                .unwrap_or_else(|_| "/".parse().unwrap()),
            http_iso_path: row
                .http_iso_path
                .parse()
                .unwrap_or_else(|_| "/".parse().unwrap()),
            tftp_kernel_path: row
                .tftp_kernel_path
                .parse()
                .unwrap_or_else(|_| "/".parse().unwrap()),
            tftp_initrd_paths,
        };
        images.insert(row.name, image);
    }

    Ok(images)
}

pub async fn fetch_kernel_args_map(
    pool: &PgPool,
) -> Result<HashMap<String, Vec<ImageKernelArg>>, InventoryError> {
    let rows = sqlx::query_as!(
        ImageKernelArg,
        r#"
        SELECT ika.id, ika.for_image, ika._key, ika._value
        FROM image_kernel_args ika
        INNER JOIN images i ON ika.for_image = i.id
        WHERE i.deleted = false
        ORDER BY i.name, ika._key
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching kernel args".to_string(),
        source: e,
    })?;

    // Group by image name
    let mut kernel_args_map: HashMap<String, Vec<ImageKernelArg>> = HashMap::new();

    for row in rows {
        // Need to fetch image name for this kernel arg
        let image_name =
            sqlx::query_scalar!("SELECT name FROM images WHERE id = $1", row.for_image)
                .fetch_one(pool)
                .await
                .map_err(|e| InventoryError::Sqlx {
                    context: "While fetching image name for kernel arg".to_string(),
                    source: e,
                })?;

        kernel_args_map.entry(image_name).or_default().push(row);
    }

    Ok(kernel_args_map)
}

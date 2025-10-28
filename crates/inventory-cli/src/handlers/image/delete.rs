use crate::prelude::InventoryError;

use sqlx::{Postgres, Transaction};

pub async fn delete_image_by_name(
    transaction: &mut Transaction<'_, Postgres>,
    image_name: &str,
) -> Result<(), InventoryError> {
    let image = sqlx::query!(
        r#"SELECT id FROM images WHERE name = $1 AND deleted = false"#,
        image_name
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching image to delete".to_string(),
        source: e,
    })?
    .ok_or_else(|| InventoryError::NotFound("Image not found".to_string()))?;

    sqlx::query!(
        r#"UPDATE images SET deleted = true WHERE id = $1"#,
        image.id
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While soft-deleting image".to_string(),
        source: e,
    })?;

    Ok(())
}

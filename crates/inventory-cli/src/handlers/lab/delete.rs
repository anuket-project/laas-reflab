use crate::prelude::InventoryError;

use sqlx::{Postgres, Transaction};

pub async fn delete_lab_by_name(
    transaction: &mut Transaction<'_, Postgres>,
    lab_name: &str,
) -> Result<(), InventoryError> {
    let lab = sqlx::query!(
        r#"
        SELECT id FROM labs WHERE name = $1
"#,
        lab_name
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While fetching lab '{}'", lab_name),
        source: e,
    })?
    .ok_or_else(|| {
        InventoryError::NotFound(format!(
            "Lab '{}' not found",
            lab_name
        ))
    })?;

    sqlx::query!(
        r#"DELETE FROM labs WHERE id = $1"#,
        lab.id
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While deleting lab".into(),
        source: e,
    })?;

    Ok(())
}

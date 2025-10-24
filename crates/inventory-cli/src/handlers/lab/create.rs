use crate::prelude::{InventoryError, LabYaml};

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub async fn create_lab(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &LabYaml,
) -> Result<(), InventoryError> {
    let id = Uuid::new_v4();

    sqlx::query!(
        r#"
            INSERT INTO labs (
                id,
                name,
                location,
                email,
                phone,
                is_dynamic
            )
            VALUES (
                $1, $2, $3, $4, $5, $6
            )
        "#,
        id,
        yaml.name,
        yaml.location,
        yaml.email,
        yaml.phone,
        yaml.is_dynamic
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While inserting new lab".to_string(),
        source: e,
    })?;

    Ok(())
}

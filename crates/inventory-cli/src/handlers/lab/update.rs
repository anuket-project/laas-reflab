use crate::prelude::{InventoryError, LabYaml};

use sqlx::{Postgres, Transaction};

pub async fn update_lab(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &LabYaml,
) -> Result<(), InventoryError> {
    sqlx::query!(
        r#"
        UPDATE labs
        SET
            location = $2,
            email = $3,
            phone = $4,
            is_dynamic = $5
        WHERE name = $1;
        "#,
        yaml.name,
        yaml.location,
        yaml.email,
        yaml.phone,
        yaml.is_dynamic,
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While updating lab record".into(),
        source: e,
    })?;

    Ok(())
}

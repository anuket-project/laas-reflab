use crate::prelude::InventoryError;
use sqlx::{Postgres, Transaction};

/// Delete a [`SwitchPort`] by its switch's name and the port's name.
pub async fn delete_switchport(
    transaction: &mut Transaction<'_, Postgres>,
    switch_name: &str,
    switchport_name: &str,
) -> Result<(), InventoryError> {
    let result = sqlx::query!(
        r#"
        DELETE FROM switchports
        WHERE for_switch = (
            SELECT id
            FROM switches
            WHERE name = $1
            LIMIT 1
        )
        AND name = $2
        "#,
        switch_name,     // $1: switch name to look up
        switchport_name  // $2: port name to delete
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "While deleting switchport '{}' on switch '{}'",
            switchport_name, switch_name
        ),
        source: e,
    })?;

    if result.rows_affected() == 0 {
        return Err(InventoryError::NotFound(format!(
            "No switchport '{}' found on switch '{}'",
            switchport_name, switch_name
        )));
    }

    Ok(())
}

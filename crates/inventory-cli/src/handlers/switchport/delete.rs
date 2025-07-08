#[allow(unused_imports)]
use crate::prelude::{InventoryError, SwitchPort, hostport};
use sqlx::PgPool;

/// Delete a [`SwitchPort`] by its switch’s name and the port’s name.
pub async fn delete_switchport(
    pool: &PgPool,
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
        switch_name,     // $1 → switch name to look up
        switchport_name  // $2 → port name to delete
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "While deleting switchport '{}' on switch '{}'",
            switchport_name, switch_name
        ),
        source: e,
    })?;

    // TODO: handle deleting hostport reference to this switchport?

    if result.rows_affected() == 0 {
        return Err(InventoryError::NotFound(format!(
            "No switchport '{}' found on switch '{}'",
            switchport_name, switch_name
        )));
    }

    Ok(())
}

/// Delete all switchports.
#[allow(dead_code)]
pub async fn delete_all_switchports(pool: &PgPool) -> Result<(), InventoryError> {
    // clear foreign keys to switchports in hostports
    hostport::clear_switchport_foreignkeys(pool).await?;

    let result = sqlx::query!("DELETE FROM switchports")
        .execute(pool)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: "While deleting all switchports".to_string(),
            source: e,
        })?;

    println!("Cleared {} switchports...", result.rows_affected());

    Ok(())
}

use crate::prelude::{InterfaceYaml, InventoryError};
use sqlx::PgPool;
use uuid::Uuid;

/// Rename a [`SwitchPort`] from `old_port_name` to `new_port_name`
/// on the switch identified by `switch_name`.
#[allow(dead_code)]
pub async fn update_switchport(
    pool: &PgPool,
    switch_name: &str,
    old_port_name: &str,
    new_port_name: &str,
) -> Result<(), InventoryError> {
    let result = sqlx::query!(
        r#"
        UPDATE switchports
        SET name = $3
        WHERE for_switch = (
            SELECT id
            FROM switches
            WHERE name = $1
            LIMIT 1
        )
        AND name = $2
        "#,
        switch_name,   // $1 → switch to look up
        old_port_name, // $2 → existing port name
        new_port_name  // $3 → the new port name
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "While updating switchport '{}' → '{}' on switch '{}'",
            old_port_name, new_port_name, switch_name
        ),
        source: e,
    })?;

    if result.rows_affected() == 0 {
        return Err(InventoryError::Conflict(format!(
            "No switchport '{}' found on switch '{}', so nothing was updated",
            old_port_name, switch_name
        )));
    }

    Ok(())
}

#[allow(dead_code)]
pub async fn update_switch_on_switchport(
    pool: &PgPool,
    port_name: &str,
    _interface_yaml: &InterfaceYaml,
    old_switch_name: &str,
    new_switch_name: &str,
) -> Result<(), InventoryError> {
    // 1) create new `SwitchPort` on new switch, make sure it doesn't exist already. (if
    //    new_port_name)
    let new_switchport_row = sqlx::query!(
        r#"
            INSERT INTO switchports (id, for_switch, name) VALUES ($1, (SELECT id FROM switches WHERE name = $2), $3) RETURNING id
        "#,
        Uuid::new_v4(),  // $1 → new port ID
        new_switch_name, // $2 → new switch name
        port_name,
    )
    .fetch_one(pool)
    .await
    .map_err(|_| {
        InventoryError::Conflict(format!(
            "Attempting to create a switchport that already exists: switch: {}, switchport: {}",
            new_switch_name, port_name
        ))
    })?;

    // 2) update `HostPort` to point to the new `SwitchPort`
    sqlx::query!(
        r#"
        UPDATE host_ports SET switchport = $1
        "#,
        new_switchport_row.id, // $1 → new switchport ID
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "While updating host port to point to new switchport '{}' on switch '{}'",
            port_name, new_switch_name
        ),
        source: e,
    })?;

    // 3) Delete old `SwitchPort` if it exists
    sqlx::query!(
        r#"
        DELETE FROM switchports
        WHERE for_switch = (SELECT id FROM switches WHERE name = $1)
        AND name = $2
        "#,
        old_switch_name, // $1 → old switch name
        port_name,       // $2 → port name to delete
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "While deleting old switchport '{}' on switch '{}'",
            port_name, old_switch_name
        ),
        source: e,
    })?;

    Ok(())
}

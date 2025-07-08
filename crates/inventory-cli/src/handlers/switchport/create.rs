use sqlx::PgPool;
use uuid::Uuid;

#[allow(unused_imports)]
use crate::prelude::{InventoryError, SwitchPort, hostport};

/// Insert a new [`SwitchPort`] given a `switch_name` and `switchport_name`.
pub async fn create_switchport(
    pool: &PgPool,
    switch_name: &str,
    switchport_name: &str,
) -> Result<(), InventoryError> {
    let new_id = Uuid::new_v4();

    println!(
        "Creating switchport '{}' on switch '{}'.",
        switchport_name, switch_name
    );

    let result = sqlx::query!(
        r#"
        INSERT INTO switchports (id, for_switch, name)
        VALUES (
            $1,
            (SELECT id FROM switches WHERE name = $2 LIMIT 1),
            $3
        )
        "#,
        new_id,          // $1 → new switchport ID
        switch_name,     // $2 → switch name to look up
        switchport_name  // $3 → port name
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While inserting SwitchPort".to_string(),
        source: e,
    })?;

    if result.rows_affected() == 0 {
        return Err(InventoryError::Conflict(format!(
            "Switchport '{}' already exists on switch '{}'",
            switchport_name, switch_name
        )));
    }

    Ok(())
}

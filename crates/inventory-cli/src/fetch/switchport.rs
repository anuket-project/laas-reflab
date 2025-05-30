use sqlx::PgPool;
use uuid::Uuid;

use crate::prelude::InventoryError;

#[allow(dead_code)]
pub async fn fetch_switchport_uuid_from_switchport_names(
    pool: &PgPool,
    switchport_name: &str,
    switch_name: &str,
) -> Result<Uuid, InventoryError> {
    // query joining switchports → switches
    let port_id = sqlx::query_scalar!(
        r#"
        SELECT sp.id
          FROM switchports sp
          JOIN switches    s  ON sp.for_switch = s.id
         WHERE sp.name  = $1
           AND s.name   = $2
        "#,
        switchport_name,
        switch_name
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "While verifying switchport '{}' on switch '{}'",
            switchport_name, switch_name
        ),
        source: e,
    })?
    .ok_or_else(|| {
        InventoryError::NotFound(format!(
            "No switchport '{}' found on switch '{}'",
            switchport_name, switch_name
        ))
    })?;

    Ok(port_id)
}

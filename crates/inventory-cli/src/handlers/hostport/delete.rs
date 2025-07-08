#[allow(unused_imports)]
use crate::prelude::{HostPort, InterfaceYaml, InventoryError};

use sqlx::PgPool;
use uuid::Uuid;

/// Delete a single [`HostPort`] by host_name + port name.
pub async fn delete_hostport_by_name(
    pool: &PgPool,
    server_name: &str,
    db_interface: &HostPort,
) -> Result<(), InventoryError> {
    // look up the port’s UUID
    let port_id: Uuid = sqlx::query_scalar!(
        r#"
        SELECT hp.id
          FROM host_ports hp
          JOIN hosts h ON hp.on_host = h.id
         WHERE h.server_name = $1
           AND hp.name = $2
        "#,
        server_name,
        db_interface.name
    )
    .fetch_one(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "Looking up hostport `{}` on `{}`",
            db_interface.name, server_name
        ),
        source: e,
    })?;

    // delete the row
    sqlx::query!(r#"DELETE FROM host_ports WHERE id = $1"#, port_id)
        .execute(pool)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: format!("Deleting hostport `{}`", port_id),
            source: e,
        })?;

    Ok(())
}

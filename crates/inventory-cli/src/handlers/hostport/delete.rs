#[allow(unused_imports)]
use crate::prelude::{HostPort, InterfaceYaml, InventoryError};

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Delete a single [`HostPort`] by host_name + port name.
pub async fn delete_hostport_by_name(
    transaction: &mut Transaction<'_, Postgres>,
    server_name: &str,
    db_interface: &HostPort,
) -> Result<(), InventoryError> {
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
    .fetch_one(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "Looking up hostport `{}` on `{}`",
            db_interface.name, server_name
        ),
        source: e,
    })?;

    sqlx::query!(r#"DELETE FROM host_ports WHERE id = $1"#, port_id)
        .execute(&mut **transaction)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: format!("Deleting hostport `{}`", port_id),
            source: e,
        })?;

    Ok(())
}

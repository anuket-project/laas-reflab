use sqlx::PgPool;
use std::collections::HashMap;

use crate::prelude::{HostPort, InventoryError};

#[derive(Debug, sqlx::FromRow)]
struct HostPortRow {
    server_name: String,

    #[sqlx(flatten)]
    port: HostPort,
}

pub async fn fetch_hostport_map(
    pool: &PgPool,
) -> Result<HashMap<String, Vec<HostPort>>, InventoryError> {
    // fetch all server_name + host_ports from database
    let rows = sqlx::query_as::<_, HostPortRow>(
        r#"
        SELECT
          h.server_name,
          hp.id,
          hp.on_host,
          hp.switchport,
          hp.name,
          hp.speed,
          hp.mac,
          hp.switch,
          hp.bus_addr,
          hp.bmc_vlan_id,
          hp.management_vlan_id,
          hp.is_a
        FROM host_ports hp
        JOIN hosts      h  ON hp.on_host = h.id
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching host ports".to_string(),
        source: e,
    })?;

    // convert to HashMap<String, Vec<HostPort>>
    let mut map: HashMap<String, Vec<HostPort>> = HashMap::new();
    for HostPortRow { server_name, port } in rows {
        map.entry(server_name).or_default().push(port);
    }

    Ok(map)
}

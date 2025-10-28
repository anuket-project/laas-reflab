use dal::FKey;
use sqlx::PgPool;
use std::collections::HashMap;

use mac_address::MacAddress;
use models::inventory::DataValue;

use crate::prelude::{HostPort, InventoryError, SwitchPort};

pub async fn fetch_hostport_map(
    pool: &PgPool,
) -> Result<HashMap<String, Vec<HostPort>>, InventoryError> {
    let rows = sqlx::query!(
        r#"
        SELECT
          h.server_name           AS "server_name!",
          hp.id                   AS "id: uuid::Uuid",
          hp.on_host              AS "on_host: uuid::Uuid",
          hp.switchport           AS "switchport?: uuid::Uuid",
          hp.name                 AS "name!",
          hp.speed                AS "speed?: DataValue",
          hp.mac                  AS "mac: MacAddress",
          hp.switch               AS "switch!",
          hp.bus_addr             AS "bus_addr!",
          hp.bmc_vlan_id          AS "bmc_vlan_id?: i16",
          hp.management_vlan_id   AS "management_vlan_id?: i16"
        FROM host_ports hp
        JOIN hosts      h   ON hp.on_host = h.id
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching host ports".into(),
        source: e,
    })?;

    let mut map: HashMap<String, Vec<HostPort>> = HashMap::new();
    for row in rows {
        let switchport_opt: Option<FKey<SwitchPort>> = row
            .switchport
            .map(|switchport| FKey::from_id(switchport.into()));

        let data_value = row.speed.unwrap_or_default();

        let port = HostPort {
            id: FKey::from_id(row.id.into()),
            on_host: FKey::from_id(row.on_host.into()),
            switchport: switchport_opt,
            name: row.name,
            speed: data_value,
            mac: row.mac.expect("Expected a valid MAC address"),
            switch: row.r#switch,
            bus_addr: row.bus_addr,
            bmc_vlan_id: row.bmc_vlan_id,
            management_vlan_id: row.management_vlan_id,
        };
        map.entry(row.server_name).or_default().push(port);
    }

    Ok(map)
}

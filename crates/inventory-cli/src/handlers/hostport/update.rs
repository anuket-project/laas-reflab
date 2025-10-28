use sqlx::{Postgres, Transaction};

use dal::FKey;
use models::inventory::DataValue;

use crate::prelude::{HostPort, InterfaceYaml, InventoryError};

/// Update all columns of a [`HostPort`] identified by host.server_name + hostport.name
/// Returns the fully updated [`HostPort`].
pub async fn update_hostport_by_name(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &InterfaceYaml,
    server_name: &str,
    _flavor_name: &str,
) -> Result<HostPort, InventoryError> {
    let row = sqlx::query!(
        r#"
        UPDATE host_ports hp
           SET
             switchport          = (SELECT id FROM switchports WHERE name = $3 AND for_switch = (SELECT id FROM switches WHERE name = $6)),
             name                = $2,
             speed               = $4,
             mac                 = $5,
             switch              = $6,
             bus_addr            = $7,
             bmc_vlan_id         = $8,
             management_vlan_id  = $9
          FROM hosts h
          WHERE hp.on_host = h.id
            AND h.server_name = $1
            AND hp.name        = $2
        RETURNING
            hp.id,
            hp.on_host,
            hp.switchport,
            hp.name,
            hp.speed,
            hp.mac,
            hp.switch,
            hp.bus_addr,
            hp.bmc_vlan_id,
            hp.management_vlan_id
        "#,
        server_name,                     // $1: hosts.server_name
        yaml.name,                       // $2: host_ports.name
        yaml.connection.switchport_name, // $3 switchport_name
        serde_json::Value::Null,         // $4: speed (or yaml.connection.speed)
        yaml.mac,                        // $5: mac
        yaml.connection.switch_name,     // $6: switch
        yaml.bus_addr,                   // $7: bus_addr
        yaml.bmc_vlan_id,                // $8: bmc_vlan_id
        yaml.management_vlan_id,         // $9 management_vlan_id
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("Updating hostport `{}` on `{}`", yaml.name, server_name),
        source: e,
    })?;

    let mac = row.mac.ok_or_else(|| {
        InventoryError::NotFound(format!(
            "HostPort `{}` on `{}` has no mac",
            yaml.name, server_name
        ))
    })?;

    let switchport_opt = row
        .switchport
        .map(|switchport| FKey::from_id(switchport.into()));

    // safely handle serde_json::NULL
    let speed = if row.speed.is_null() {
        DataValue::default()
    } else {
        DataValue::from_sqlval(row.speed).map_err(|e| InventoryError::DataValueDeserialization {
            context: format!("hostport '{}' on host '{}'", row.name, server_name),
            column: "speed".to_string(),
            source: e,
        })?
    };

    let hp = HostPort {
        id: FKey::from_id(row.id.into()),
        on_host: FKey::from_id(row.on_host.into()),
        switchport: switchport_opt,
        name: row.name.clone(),
        speed,
        mac,
        switch: row.switch,
        bus_addr: row.bus_addr,
        bmc_vlan_id: row.bmc_vlan_id,
        management_vlan_id: row.management_vlan_id,
    };

    Ok(hp)
}

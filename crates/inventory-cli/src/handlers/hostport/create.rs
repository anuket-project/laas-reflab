use crate::prelude::{HostPort, InterfaceYaml, InventoryError};
use sqlx::PgPool;
use uuid::Uuid;

use dal::FKey;
use models::inventory::DataValue;

/// Create a single [`HostPort`] for one InterfaceYaml
pub async fn create_hostport_from_iface(
    pool: &PgPool,
    yaml: &InterfaceYaml,
    server_name: &str,
    _flavor_name: &str,
) -> Result<HostPort, InventoryError> {
    // perform the insert
    let row = sqlx::query!(
        r#"
        INSERT INTO host_ports (
            id,
            on_host,
            switchport,
            name,
            speed,
            mac,
            switch,
            bus_addr,
            bmc_vlan_id,
            management_vlan_id
        ) VALUES (
            $1,
            (SELECT id FROM hosts WHERE server_name = $2),
            (SELECT id FROM switchports WHERE name = $3 AND for_switch = (SELECT id FROM switches WHERE name = $7)),
            $4,
            $5,
            $6,
            $7,
            $8,
            $9,
            $10
            )
            RETURNING
                id,
                on_host,
                switchport,
                name,
                speed,
                mac,
                switch,
                bus_addr,
                bmc_vlan_id,
                management_vlan_id
        "#,
        Uuid::new_v4(),                     // $1: id
        server_name,                        // $2: server_name
        yaml.connection.switchport_name,    // $3: switchport lookup
        yaml.name,                          // $4: name
        *DataValue::default().to_sqlval().unwrap(),            // $5: speed (default to Unknown)
        yaml.mac,                           // $6: mac
        yaml.connection.switch_name,        // $7: switch
        yaml.bus_addr,                      // $8: bus_addr
        yaml.bmc_vlan_id,                   // $9: bmc_vlan_id
        yaml.management_vlan_id,            // $10: management_vlan_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("Inserting hostport `{}`", yaml.name),
        source: e,
    })?;

    let mac = row.mac.ok_or_else(|| {
        InventoryError::NotFound(format!("HostPort `{}` inserted without MAC", row.name))
    })?;

    let switchport_opt = row
        .switchport
        .map(|switchport| FKey::from_id(switchport.into()));

    let hp = HostPort {
        id: FKey::from_id(row.id.into()),
        on_host: FKey::from_id(row.on_host.into()),
        switchport: switchport_opt,
        name: row.name,
        // TODO: This should not really be able to fail
        speed: Result::map_err(DataValue::from_sqlval(row.speed), |e| {
            InventoryError::Anyhow(e)
        })?,
        mac,
        switch: row.switch,
        bus_addr: row.bus_addr,
        bmc_vlan_id: row.bmc_vlan_id,
        management_vlan_id: row.management_vlan_id,
    };

    Ok(hp)
}

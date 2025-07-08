use sqlx::PgPool;

use dal::FKey;
use models::inventory::DataValue;
use uuid::Uuid;

use crate::prelude::{HostPort, InterfaceYaml, InventoryError};

/// Update all columns of a [`HostPort`] identified by host.server_name + hostport.name
/// Returns the fully updated [`HostPort`].
pub async fn update_hostport_by_name(
    pool: &PgPool,
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
    .fetch_one(pool)
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

    let hp = HostPort {
        id: FKey::from_id(row.id.into()),
        on_host: FKey::from_id(row.on_host.into()),
        switchport: switchport_opt,
        name: row.name,
        speed: DataValue::from_sqlval(row.speed).map_err(InventoryError::Anyhow)?,
        mac,
        switch: row.switch,
        bus_addr: row.bus_addr,
        bmc_vlan_id: row.bmc_vlan_id,
        management_vlan_id: row.management_vlan_id,
    };

    Ok(hp)
}

/// Set all `HostPort.switchports` to NULL
pub async fn clear_switchport_foreignkeys(pool: &PgPool) -> Result<(), InventoryError> {
    println!("Clearing switchport foreign keys in host_ports...");
    let query = sqlx::query!(
        r#"
        UPDATE host_ports
           SET switchport = NULL
         WHERE switchport IS NOT NULL
        "#
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "Clearing switchport foreign keys".to_string(),
        source: e,
    })?;

    println!(
        "Cleared {} switchport foreign keys in host_ports.",
        query.rows_affected()
    );

    Ok(())
}

/// Set a specific `HostPort.switchport` to NULL
#[allow(dead_code)]
pub async fn clear_switchport_for_hostport(
    pool: &PgPool,
    hostport_id: &Uuid,
) -> Result<(), InventoryError> {
    println!(
        "Clearing switchport foreign key for hostport {}...",
        hostport_id
    );
    let query = sqlx::query!(
        r#"
        UPDATE host_ports
           SET switchport = NULL
         WHERE id = $1
        "#,
        hostport_id
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "Clearing switchport foreign key for hostport {}",
            hostport_id
        ),
        source: e,
    })?;

    if query.rows_affected() == 0 {
        return Err(InventoryError::NotFound(format!(
            "No HostPort found with ID {}",
            hostport_id
        )));
    }

    println!(
        "Cleared switchport foreign key for hostport {}.",
        hostport_id
    );
    Ok(())
}

/// Set hostport foreign key
#[allow(dead_code)]
pub async fn set_hostport_foreignkey(
    pool: &PgPool,
    interface_yaml: InterfaceYaml,
    switchport_name: &str,
    switch_name: &str,
) -> Result<(), InventoryError> {
    sqlx::query!(
        r#"
        UPDATE host_ports
           SET switchport = (SELECT id FROM switchports WHERE name = $1 AND for_switch = (SELECT id FROM switches WHERE name = $3))
           WHERE mac = $2
        "#,
        switchport_name, // $1: switchport name
        interface_yaml.mac, // $2: hostport name
        switch_name, // $3: switch name
    ).execute(pool).await.map_err(|e| InventoryError::Sqlx {
        context: format!(
            "Setting switchport foreign key for hostport with mac {}",
            interface_yaml.mac
        ),
        source: e,
    })?;

    Ok(())
}

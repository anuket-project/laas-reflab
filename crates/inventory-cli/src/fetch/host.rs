use eui48::MacAddress;
use sqlx::PgPool;
use std::collections::HashMap;

use dal::{FKey, ID};

use crate::prelude::{Host, InventoryError};

pub async fn fetch_host_map(pool: &PgPool) -> Result<HashMap<String, Host>, InventoryError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, server_name, flavor, serial,
               ipmi_fqdn, iol_id, ipmi_mac, ipmi_user,
               ipmi_pass, fqdn, projects, sda_uefi_device
          FROM hosts WHERE deleted = false
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching hosts".to_string(),
        source: e,
    })?;

    // map of server_name -> Host
    let mut map = HashMap::new();

    for r in rows {
        // convert the row
        let ipmi_mac = MacAddress::from_bytes(&r.ipmi_mac.bytes()).map_err(|e| {
            InventoryError::InvalidMac {
                server_name: r.server_name.clone(),
                raw: r.ipmi_mac.to_string(),
                source: e,
            }
        })?;
        let host = Host {
            id: FKey::from_id(ID::from(r.id)),
            server_name: r.server_name.clone(),
            flavor: FKey::from_id(ID::from(r.flavor)),
            serial: r.serial,
            ipmi_fqdn: r.ipmi_fqdn,
            iol_id: r.iol_id,
            ipmi_mac,
            ipmi_user: r.ipmi_user,
            ipmi_pass: r.ipmi_pass,
            fqdn: r.fqdn,
            projects: serde_json::from_value(r.projects)?,
            sda_uefi_device: r.sda_uefi_device,
        };

        // insert into the map, error if there is a duplicate
        if map.insert(host.server_name.clone(), host).is_some() {
            return Err(InventoryError::DuplicateHost(r.server_name));
        }
    }

    Ok(map)
}
pub async fn fetch_host_by_name(pool: &PgPool, server_name: &str) -> Result<Host, InventoryError> {
    let row = sqlx::query!(
        r#"
        SELECT
            id,
            server_name,
            flavor,
            serial,
            ipmi_fqdn,
            iol_id,
            ipmi_mac,
            ipmi_user,
            ipmi_pass,
            projects,
            fqdn,
            sda_uefi_device
        FROM hosts
        WHERE server_name = $1 AND DELETED = false
        "#,
        server_name,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While fetching host `{}`", server_name),
        source: e,
    })?;

    // convert `macaddr` → `mac_address::MacAddress`
    let ipmi_mac =
        MacAddress::from_bytes(&row.ipmi_mac.bytes()).map_err(|e| InventoryError::InvalidMac {
            server_name: row.server_name.clone(),
            raw: row.ipmi_mac.to_string(),
            source: e,
        })?;

    // convert JSONB → Vec<String>
    let projects: Vec<String> =
        serde_json::from_value(row.projects).map_err(|e| InventoryError::InvalidProjects {
            server_name: row.server_name.clone(),
            source: e,
        })?;

    let host = Host {
        id: FKey::from_id(ID::from(row.id)),
        server_name: row.server_name.clone(),
        flavor: FKey::from_id(ID::from(row.flavor)),
        serial: row.serial,
        ipmi_fqdn: row.ipmi_fqdn,
        iol_id: row.iol_id,
        ipmi_mac,
        ipmi_user: row.ipmi_user,
        ipmi_pass: row.ipmi_pass,
        fqdn: row.fqdn,
        projects,
        sda_uefi_device: row.sda_uefi_device,
    };

    Ok(host)
}

pub async fn delete_host_by_name(server_name: &str, pool: &PgPool) -> Result<(), InventoryError> {
    let host = sqlx::query!(
        r#"
        SELECT id
        FROM hosts
        WHERE server_name = $1
          AND deleted = false
        "#,
        server_name
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While fetching host `{}`", server_name),
        source: e,
    })?
    .ok_or_else(|| {
        InventoryError::NotFound(format!(
            "Host '{}' not found or already deleted",
            server_name
        ))
    })?;

    sqlx::query!(
        r#"
        UPDATE hosts
        SET deleted = true
        WHERE id = $1
        "#,
        host.id
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While soft‐deleting host".into(),
        source: e,
    })?;

    Ok(())
}

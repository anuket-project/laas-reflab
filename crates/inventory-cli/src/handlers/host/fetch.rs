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

    // server_name to `Host`
    let mut map = HashMap::new();

    for r in rows {
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

        if map.insert(host.server_name.clone(), host).is_some() {
            return Err(InventoryError::DuplicateHost(r.server_name));
        }
    }

    Ok(map)
}

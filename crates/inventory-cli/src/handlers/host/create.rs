use crate::prelude::{HostYaml, InventoryError};

use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Insert a new host row into `hosts`.
pub async fn create_host(transaction: &mut Transaction<'_, Postgres>, yaml: &HostYaml) -> Result<(), InventoryError> {
    let id = Uuid::new_v4();

    // make sure the flavor exists
    let flavor_id = sqlx::query_scalar!(
        "SELECT id FROM flavors WHERE name = $1 AND deleted = false",
        yaml.flavor_name
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!(
            "While looking up flavor '{}' for host '{}'",
            yaml.flavor_name, yaml.server_name
        ),
        source: e,
    })?
    .ok_or_else(|| {
        InventoryError::NotFound(format!(
            "Flavor '{}' not found for host '{}'. Make sure the flavor is defined in flavors.yaml",
            yaml.flavor_name, yaml.server_name
        ))
    })?;

    sqlx::query!(
        r#"
            INSERT INTO hosts (
              id,
              server_name,
              fqdn,
              flavor,
              iol_id,
              serial,
              ipmi_fqdn,
              ipmi_mac,
              ipmi_user,
              ipmi_pass,
              projects
            ) VALUES (
              $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
            )
            "#,
        id,                                                               // $1: UUID
        yaml.server_name,                                                 // $2: VARCHAR
        format!("{}.{}", yaml.server_name, yaml.domain),                  // $3: VARCHAR
        flavor_id,                                                        // $4: UUID → flavor fk
        yaml.iol_id,                                                      // $5: VARCHAR
        yaml.serial_number,                                               // $6: VARCHAR
        format!("{}.{}", yaml.ipmi_yaml.hostname, yaml.ipmi_yaml.domain), // $7: VARCHAR
        yaml.ipmi_yaml.mac,                                               // $8: MACADDR
        yaml.ipmi_yaml.user,                                              // $9: VARCHAR
        yaml.ipmi_yaml.pass,                                              // $10: VARCHAR
        json!([yaml.project]),                                            // $11: [JSONB]
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("While inserting host '{}'", yaml.server_name),
        source: e,
    })?;

    Ok(())
}

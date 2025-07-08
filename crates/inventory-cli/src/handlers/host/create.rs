use crate::prelude::{HostYaml, InventoryError};

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Insert a brand-new host row into `hosts`.
pub async fn create_host(pool: &PgPool, yaml: &HostYaml) -> Result<(), InventoryError> {
    let id = Uuid::new_v4();

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
              $1, $2, $3,
              (SELECT id FROM flavors WHERE name = $4),
              $5, $6, $7, $8, $9, $10, $11
            )
            "#,
        id,                                                               // $1: UUID
        yaml.server_name,                                                 // $2: VARCHAR
        format!("{}.{}", yaml.server_name, yaml.domain),                  // $3: VARCHAR
        yaml.flavor_name,                                                 // $4: VARCHAR → flavor fk
        yaml.iol_id,                                                      // $5: VARCHAR
        yaml.serial_number,                                               // $6: VARCHAR
        format!("{}.{}", yaml.ipmi_yaml.hostname, yaml.ipmi_yaml.domain), // $7: VARCHAR
        yaml.ipmi_yaml.mac,                                               // $8: MACADDR
        yaml.ipmi_yaml.user,                                              // $9: VARCHAR
        yaml.ipmi_yaml.pass,                                              // $10: VARCHAR
        json!([yaml.project]),                                            // $11: [JSONB]
    )
    .execute(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While inserting new host".into(),
        source: e,
    })?;

    Ok(())
}

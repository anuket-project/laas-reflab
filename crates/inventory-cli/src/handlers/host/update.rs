use crate::prelude::{HostYaml, InventoryError};

use serde_json::json;
use sqlx::{Postgres, Transaction};

pub async fn update_host(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &HostYaml,
) -> Result<(), InventoryError> {
    sqlx::query!(
        r#"
    UPDATE hosts
    SET
      fqdn        = $2,
      flavor      = (SELECT id FROM flavors WHERE name = $3),
      iol_id      = $4,
      serial      = $5,
      ipmi_fqdn   = $6,
      ipmi_mac    = $7,
      ipmi_user   = $8,
      ipmi_pass   = $9,
      projects    = $10
    WHERE server_name = $1;
    "#,
        yaml.server_name,                                // $1: VARCHAR → String
        format!("{}.{}", yaml.server_name, yaml.domain), // $2: VARCHAR → String
        yaml.flavor_name,                                // $3: VARCHAR → String
        yaml.iol_id,                                     // $4: VARCHAR → String
        yaml.serial_number,                              // $5: VARCHAR → String
        format!("{}.{}", yaml.ipmi_yaml.hostname, yaml.ipmi_yaml.domain), // $6: VARCHAR → String
        yaml.ipmi_yaml.mac,                              // $7: macaddr  → String
        yaml.ipmi_yaml.user,                             // $8: VARCHAR → String
        yaml.ipmi_yaml.pass,                             // $9: VARCHAR → String
        json!([yaml.project]),                           // $10: JSONB []   → String
    )
    .execute(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While updating host record".into(),
        source: e,
    })?;

    Ok(())
}

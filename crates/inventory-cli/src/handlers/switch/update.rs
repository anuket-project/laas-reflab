use sqlx::PgPool;

use crate::prelude::{InventoryError, Switch, SwitchYaml};

use dal::FKey;

/// Update an existing [`Switch`] by its name.
/// Returns the updated [`Switch`].
pub async fn update_switch_by_name(
    pool: &PgPool,
    yaml: &SwitchYaml,
) -> Result<Switch, InventoryError> {
    let os_id = sqlx::query_scalar!(
        r#"
        SELECT id FROM switch_os WHERE os_type = $1
        "#,
        yaml.switch_os
    )
    .fetch_one(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("Fetching switch OS `{}`", yaml.switch_os),
        source: e,
    })?;

    let row = sqlx::query!(
        r#"
        UPDATE switches
           SET
             ip        = $2,
             switch_user = $3,
             switch_pass = $4,
             switch_os = (SELECT id FROM switch_os WHERE name = $5)
         WHERE name = $1
        RETURNING
          id,
          name,
          ip,
          switch_user AS user,
          switch_pass AS pass,
          switch_os
        "#,
        yaml.name,           // $1: existing name
        yaml.ip.to_string(), // $2
        yaml.username,       // $3
        yaml.password,       // $4
        yaml.switch_os,      // $5
    )
    .fetch_one(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("Updating switch `{}`", yaml.name),
        source: e,
    })?;

    Ok(Switch {
        id: FKey::from_id(row.id.into()),
        name: row.name,
        ip: row.ip.to_string(),
        user: row.user,
        pass: row.pass,
        switch_os: Some(FKey::from_id(os_id.into())),
    })
}

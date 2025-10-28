use crate::prelude::{InventoryError, SwitchYaml};
use models::inventory::Switch;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use dal::FKey;

/// Insert a new [`Switch`] from a [`SwitchYaml`].
pub async fn create_switch(
    transaction: &mut Transaction<'_, Postgres>,
    yaml: &SwitchYaml,
) -> Result<Switch, InventoryError> {
    // handle switch_os
    let os_id = if yaml.switch_os.is_empty() {
        None
    } else {
        Some(
            sqlx::query_scalar!(
                r#"
                SELECT id FROM switch_os WHERE os_type = $1
                "#,
                yaml.switch_os
            )
            .fetch_one(&mut **transaction)
            .await
            .map_err(|e| InventoryError::Sqlx {
                context: format!("Fetching switch OS `{}`", yaml.switch_os),
                source: e,
            })?,
        )
    };

    let row = sqlx::query!(
        r#"
        INSERT INTO switches (
          id,
          name,
          ip,
          switch_user,
          switch_pass,
          switch_os
        ) VALUES (
          $1, $2, $3, $4, $5, $6
        )
        RETURNING
          id,
          name,
          ip,
          switch_user AS user,
          switch_pass AS pass,
          switch_os
        "#,
        Uuid::new_v4(),      // $1
        yaml.name,           // $2
        yaml.ip.to_string(), // $3
        yaml.username,       // $4
        yaml.password,       // $5
        os_id                // $6
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: format!("Creating switch `{}`", yaml.name),
        source: e,
    })?;

    Ok(Switch {
        id: FKey::from_id(row.id.into()),
        name: row.name,
        ip: row.ip.to_string(),
        user: row.user,
        pass: row.pass,
        switch_os: os_id.map(|id| FKey::from_id(id.into())),
    })
}

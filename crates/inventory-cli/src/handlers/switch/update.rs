use sqlx::{Postgres, Transaction};

use crate::prelude::{InventoryError, Switch, SwitchYaml};

use dal::FKey;

/// Update an existing [`Switch`] by its name.
/// Returns the updated [`Switch`].
pub async fn update_switch_by_name(
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
        UPDATE switches
           SET
             ip          = $2,
             switch_user = $3,
             switch_pass = $4,
             switch_os   = $5
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
        os_id,               // $5
    )
    .fetch_one(&mut **transaction)
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
        switch_os: os_id.map(|id| FKey::from_id(id.into())),
    })
}

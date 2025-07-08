use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::prelude::{
    InventoryError, Switch, SwitchPort, SwitchYaml, switchport::fetch_switchport_map,
};

pub async fn fetch_switch_map(pool: &PgPool) -> Result<HashMap<String, Switch>, InventoryError> {
    let rows: Vec<Switch> = sqlx::query_as::<_, Switch>(
        r#"
        SELECT
          id,
          name,
          ip,
          switch_user AS user,
          switch_pass AS pass,
          switch_os
        FROM switches
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching switch map".to_string(),
        source: e,
    })?;

    // build a HashMap name -> Switch
    let map = rows
        .into_iter()
        .map(|sw| (sw.name.clone(), sw))
        .collect::<HashMap<_, _>>();

    Ok(map)
}

// TODO : maybe not the best idea to mix the yaml here but I have to get this done.
#[allow(dead_code)]
pub async fn fetch_switch_with_ports_map(
    pool: &PgPool,
    switch_map: HashMap<String, SwitchYaml>,
) -> Result<HashMap<String, HashMap<String, SwitchPort>>, InventoryError> {
    let port_vec_map: HashMap<String, Vec<SwitchPort>> = fetch_switchport_map(pool).await?;

    let mut result: HashMap<String, HashMap<String, SwitchPort>> = HashMap::new();

    for (switch_name, switch) in switch_map {
        let inner = port_vec_map
            .get(&switch_name)
            .map(|ports| ports.iter().cloned().map(|p| (p.name.clone(), p)).collect())
            .unwrap_or_default();

        result.insert(switch.name, inner);
    }

    Ok(result)
}

#[allow(dead_code)]
pub async fn fetch_switch_by_id(
    pool: &PgPool,
    id: &Uuid,
) -> Result<Option<Switch>, InventoryError> {
    let row: Option<Switch> = sqlx::query_as::<_, Switch>(
        r#"
        SELECT
          id,
          name,
          ip,
          switch_user AS user,
          switch_pass AS pass,
          switch_os
        FROM switches
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching switch by name".to_string(),
        source: e,
    })?;

    Ok(row)
}

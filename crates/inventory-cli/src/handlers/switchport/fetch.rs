use sqlx::PgPool;
use std::collections::HashMap;

use crate::prelude::{InventoryError, SwitchPort};

use sqlx::FromRow;

#[derive(Debug, FromRow)]
struct SwitchPortRow {
    switch_name: String,

    #[sqlx(flatten)]
    port: SwitchPort,
}

// retrieve a map of switch names to their switchports
pub async fn fetch_switchport_map(
    pool: &PgPool,
) -> Result<HashMap<String, Vec<SwitchPort>>, InventoryError> {
    let rows: Vec<SwitchPortRow> = sqlx::query_as::<_, SwitchPortRow>(
        r#"
        SELECT
          s.name             AS switch_name,
          sp.id,
          sp.for_switch,
          sp.name
        FROM switchports sp
        JOIN switches      s ON sp.for_switch = s.id
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| InventoryError::Sqlx {
        context: "While fetching switch → switchport map".to_string(),
        source: e,
    })?;

    let mut map: HashMap<String, Vec<SwitchPort>> = HashMap::new();
    for SwitchPortRow { switch_name, port } in rows {
        map.entry(switch_name).or_default().push(port);
    }

    Ok(map)
}

use dal::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::Switch;

#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct SwitchPort {
    pub id: FKey<SwitchPort>,

    pub for_switch: FKey<Switch>,
    pub name: String,
}

impl DBTable for SwitchPort {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "switchports"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_switch: row.try_get("for_switch")?,
            name: row.try_get("name")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("for_switch", Box::new(clone.for_switch)),
            ("name", Box::new(clone.name)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl SwitchPort {
    pub async fn get_or_create_port(
        t: &mut EasyTransaction<'_>,
        on: FKey<Switch>,
        name: String,
    ) -> Result<ExistingRow<SwitchPort>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE for_switch = $1 AND name = $2;");

        let existing = t.query_opt(&q, &[&on, &name.clone()]).await.unwrap();

        match existing {
            Some(r) => Ok(Self::from_row(r)?),
            None => {
                // need to create the port
                let sp = SwitchPort {
                    id: FKey::new_id_dangling(),
                    for_switch: on,
                    name,
                };

                Ok(NewRow::new(sp).insert(t).await?.get(t).await?)
            }
        }
    }
}

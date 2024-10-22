use dal::*;

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Hash, Clone, JsonSchema)]
pub struct Cifile {
    pub id: FKey<Cifile>,
    pub priority: i16,
    pub data: String,
}

impl Cifile {
    pub async fn new(
        t: &mut EasyTransaction<'_>,
        strings: Vec<String>,
    ) -> Result<Vec<FKey<Cifile>>, anyhow::Error> {
        let mut priority: i16 = 1;
        let mut cifiles: Vec<FKey<Cifile>> = Vec::new();
        for data in strings {
            if !data.is_empty() {
                let cif = Cifile {
                    id: FKey::new_id_dangling(),
                    priority,
                    data,
                };

                priority += 1; // Starts priority at 2 as the generated file is highest priority

                match NewRow::new(cif.clone()).insert(t).await {
                    Ok(fk) => cifiles.push(fk),
                    Err(e) => {
                        todo!("Handle failure: {e:?}")
                        // TODO
                    }
                }
            }
        }
        Ok(cifiles)
    }
}

impl DBTable for Cifile {
    fn table_name() -> &'static str {
        "ci_files"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            priority: row.try_get("priority")?,
            data: row.try_get("data")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("priority", Box::new(clone.priority)),
            ("data", Box::new(clone.data)),
        ];

        Ok(c.into_iter().collect())
    }
}

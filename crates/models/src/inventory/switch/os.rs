use dal::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::Version;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwitchOS {
    pub id: FKey<SwitchOS>,
    pub os_type: String,
    pub version: Version,
}

impl DBTable for SwitchOS {
    fn table_name() -> &'static str {
        "switch_os"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            os_type: row.try_get("os_type")?,
            version: row.try_get("version")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("os_type", Box::new(clone.os_type)),
            ("version", Box::new(clone.version)),
        ];

        Ok(c.into_iter().collect())
    }
}

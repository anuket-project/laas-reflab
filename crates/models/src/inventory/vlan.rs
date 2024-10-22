//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use dal::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio_postgres::Row;

use crate::inventory::IPNetwork;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Vlan {
    pub id: FKey<Vlan>,

    pub vlan_id: i16,
    pub public_config: Option<IPNetwork>,
}

impl DBTable for Vlan {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "vlans"
    }

    fn from_row(row: Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let pc: Option<Value> = row.try_get("public_config")?;
        let pc = match pc {
            Some(v) => Some(serde_json::from_value(v)?),
            None => None,
        };
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            vlan_id: row.try_get("vlan_id")?,
            public_config: pc,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let public_config = match clone.public_config {
            None => None,
            Some(v) => Some(serde_json::to_value(v)?),
        };

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("vlan_id", Box::new(self.vlan_id)),
            ("public_config", Box::new(public_config)),
        ];

        Ok(c.into_iter().collect())
    }
}

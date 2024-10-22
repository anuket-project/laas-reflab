use common::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

use dal::*;

use crate::dashboard::network::Network;
use crate::inventory::Vlan;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkAssignmentMap {
    pub id: FKey<Self>,

    pub networks: HashMap<FKey<Network>, FKey<Vlan>>,
}

impl NetworkAssignmentMap {
    pub fn empty() -> Self {
        Self {
            id: FKey::new_id_dangling(),
            networks: HashMap::new(),
        }
    }

    pub fn add_assignment(&mut self, net: FKey<Network>, is: FKey<Vlan>) {
        self.networks.insert(net, is);
    }
}

impl DBTable for NetworkAssignmentMap {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "network_assignments"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let networks = row.try_get("networks")?;
        let networks: HashMap<String, String> = serde_json::from_value(networks)?;
        let networks = networks
            .into_iter()
            .filter_map(|(k, v)| {
                let k = ID::from_str(&k).ok()?;
                let v = ID::from_str(&v).ok()?;

                Some((FKey::from_id(k), FKey::from_id(v)))
            })
            .collect();

        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            networks,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let networks: HashMap<String, String> = clone
            .networks
            .into_iter()
            .map(|(k, v)| (k.into_id().to_string(), v.into_id().to_string()))
            .collect();
        let networks = serde_json::to_value(networks)?;
        let c: [(&str, Box<dyn ToSqlObject>); _] =
            [("id", Box::new(clone.id)), ("networks", Box::new(networks))];

        Ok(c.into_iter().collect())
    }
}

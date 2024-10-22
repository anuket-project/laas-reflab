use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use dal::*;

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct NetworkBlob {
    pub name: String,
    pub public: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct Network {
    pub id: FKey<Network>,
    pub name: String,
    pub public: bool,
}

impl DBTable for Network {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "networks"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            public: row.try_get("public")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("name", Box::new(clone.name)),
            ("public", Box::new(clone.public)),
        ];

        Ok(c.into_iter().collect())
    }
}

pub async fn import_net(net: NetworkBlob, transaction: &mut EasyTransaction<'_>) -> FKey<Network> {
    match Network::select()
        .where_field("name")
        .equals(net.name.clone())
        .run(transaction)
        .await
    {
        Ok(existing_net) => match existing_net.len() {
            0 => {
                tracing::error!("No network found, creating network.");
                let id = FKey::new_id_dangling();

                let net = Network {
                    id,
                    name: net.name,
                    public: net.public,
                };

                NewRow::new(net)
                    .insert(transaction)
                    .await
                    .expect("Expected to insert new network")
            }
            1 => existing_net.first().expect("Expected to find network").id,
            _ => {
                tracing::error!("More than one network found, please modify your template to use a specific network");
                existing_net.first().expect("Expected to find network").id
            }
        },
        Err(_) => {
            let id = FKey::new_id_dangling();

            let net = Network {
                id,
                name: net.name,
                public: net.public,
            };

            NewRow::new(net)
                .insert(transaction)
                .await
                .expect("Expected to insert new network")
        }
    }
}

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

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, Eq, PartialEq, Default)]
pub struct Network {
    pub id: FKey<Network>,
    pub name: String,
    pub public: bool,
}

impl DBTable for Network {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for Network {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Network>>(), // id
                any::<String>(),        // name
                any::<bool>(),          // public
            )
                .prop_map(|(id, name, public)| Network { id, name, public })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_network_model(network in any::<Network>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();


                let new_row = NewRow::new(network.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_network_result = Network::select().where_field("id").equals(network.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_network_result.is_ok(), "Retrieval failed: {:?}", retrieved_network_result.err());

                let first_network = retrieved_network_result.unwrap().into_iter().next();
                prop_assert!(first_network.is_some(), "No host found, empty result");

                let retrieved_network = first_network.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_network, network);

                Ok(())
            })?
        }
    }
}

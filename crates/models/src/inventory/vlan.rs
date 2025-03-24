use dal::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio_postgres::Row;

use crate::inventory::IPNetwork;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Vlan {
    pub id: FKey<Vlan>,
    pub vlan_id: i16,
    pub public_config: Option<IPNetwork>,
}

impl DBTable for Vlan {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for Vlan {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Vlan>>(),                      // id
                any::<i16>(),                             // vlan_id
                proptest::option::of(any::<IPNetwork>()), // public_config
            )
                .prop_map(|(id, vlan_id, public_config)| Vlan {
                    id,
                    vlan_id,
                    public_config,
                })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_vlan_model(vlan in any::<Vlan>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let new_row = NewRow::new(vlan.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_vlan = Vlan::select()
                    .where_field("id")
                    .equals(vlan.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_vlan.is_ok(), "Retrieval failed: {:?}", retrieved_vlan.err());
                let retrieved_vlan = retrieved_vlan.unwrap();

                let first_vlan = retrieved_vlan.first();
                prop_assert!(first_vlan.is_some(), "No Vlan found, empty result");

                let retrieved_vlan = first_vlan.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_vlan, vlan);

                Ok(())
            })?
        }
    }
}

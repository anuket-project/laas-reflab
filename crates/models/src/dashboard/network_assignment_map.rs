use common::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

use dal::*;

use crate::dashboard::network::Network;
use crate::inventory::Vlan;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
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

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::hash_map;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for NetworkAssignmentMap {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<NetworkAssignmentMap>>(),
                hash_map(any::<FKey<Network>>(), any::<FKey<Vlan>>(), 1..10),
            )
                .prop_map(|(id, networks)| NetworkAssignmentMap { id, networks })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_network_map_model(network_map in any::<NetworkAssignmentMap>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();


                let new_row = NewRow::new(network_map.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_network_map_result = NetworkAssignmentMap::select().where_field("id").equals(network_map.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_network_map_result.is_ok(), "Retrieval failed: {:?}", retrieved_network_map_result.err());

                let first_network_map = retrieved_network_map_result.unwrap().into_iter().next();
                prop_assert!(first_network_map.is_some(), "No host found, empty result");

                let retrieved_network_map = first_network_map.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_network_map, network_map);

                Ok(())
            })?
        }
    }
}

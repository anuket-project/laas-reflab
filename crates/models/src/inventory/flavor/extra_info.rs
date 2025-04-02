use dal::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::Flavor;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExtraFlavorInfo {
    pub id: FKey<ExtraFlavorInfo>,
    pub for_flavor: FKey<Flavor>,
    // Format from flavors doc: "trait:<trait_name> = value". Can be used to require or forbid hardware with the 'required' and 'forbidden' values.
    pub extra_trait: String, // Trait the key value pair appies to (e.g. 'quota', 'hw', 'hw_rng', 'pci_passthrough', 'os')
    pub key: String,
    pub value: String,
}

impl DBTable for ExtraFlavorInfo {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "extra_flavor_info"
    }

    fn from_row(
        row: tokio_postgres::Row,
    ) -> Result<ExistingRow<ExtraFlavorInfo>, common::prelude::anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_flavor: row.try_get("for_flavor")?,

            extra_trait: row.try_get("extra_trait")?,
            key: row.try_get("key")?,
            value: row.try_get("value")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            col("id", self.id),
            col("for_flavor", self.for_flavor),
            col("extra_trait", self.extra_trait.clone()),
            col("key", self.key.clone()),
            col("value", self.value.clone()),
        ];

        Ok(c.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::{block_on_runtime, insert_default_model_at};

    impl Arbitrary for ExtraFlavorInfo {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<ExtraFlavorInfo>>(), // id
                any::<FKey<Flavor>>(),          // for_flavor
                any::<String>(),                // extra_trait
                any::<String>(),                // key
                any::<String>(),                // value
            )
                .prop_map(
                    |(id, for_flavor, extra_trait, key, value)| ExtraFlavorInfo {
                        id,
                        for_flavor,
                        extra_trait,
                        key,
                        value,
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_extra_flavor_info_model(extra_flavor_info in any::<ExtraFlavorInfo>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let flavor_insert_result = insert_default_model_at(extra_flavor_info.for_flavor, &mut transaction).await;
                prop_assert!(flavor_insert_result.is_ok(), "Insert failed: {:?}", flavor_insert_result.err());

                let new_row = NewRow::new(extra_flavor_info.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_extra_flavor_info = ExtraFlavorInfo::select()
                    .where_field("id")
                    .equals(extra_flavor_info.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_extra_flavor_info.is_ok(), "Retrieval failed: {:?}", retrieved_extra_flavor_info.err());
                let retrieved_extra_flavor_info = retrieved_extra_flavor_info.unwrap();

                let first_extra_flavor_info = retrieved_extra_flavor_info.first();
                prop_assert!(first_extra_flavor_info.is_some(), "No extra flavor info found, empty result");

                let retrieved_extra_flavor_info = first_extra_flavor_info.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_extra_flavor_info.id, extra_flavor_info.id);
                prop_assert_eq!(retrieved_extra_flavor_info.for_flavor, extra_flavor_info.for_flavor);
                prop_assert_eq!(retrieved_extra_flavor_info.extra_trait, extra_flavor_info.extra_trait);
                prop_assert_eq!(retrieved_extra_flavor_info.key, extra_flavor_info.key);
                prop_assert_eq!(retrieved_extra_flavor_info.value, extra_flavor_info.value);

                Ok(())
            })?
        }
    }
}

use dal::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct SwitchOS {
    pub id: FKey<SwitchOS>,
    pub os_type: String,
}

impl DBTable for SwitchOS {
    fn table_name() -> &'static str {
        "switch_os"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            os_type: row.try_get("os_type")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("os_type", Box::new(clone.os_type)),
        ];

        Ok(c.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for SwitchOS {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<SwitchOS>>(), // id
                any::<String>(),         // os_type
            )
                .prop_map(|(id, os_type)| SwitchOS { id, os_type })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_switch_os_model(switch_os in any::<SwitchOS>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let new_row = NewRow::new(switch_os.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_switch_os = SwitchOS::select()
                    .where_field("id")
                    .equals(switch_os.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_switch_os.is_ok(), "Retrieval failed: {:?}", retrieved_switch_os.err());
                let retrieved_switch_os = retrieved_switch_os.unwrap();

                let first_switch_os = retrieved_switch_os.first();
                prop_assert!(first_switch_os.is_some(), "No SwitchOS found, empty result");

                let retrieved_switch_os = first_switch_os.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_switch_os, switch_os);

                Ok(())
            })?
        }
    }
}

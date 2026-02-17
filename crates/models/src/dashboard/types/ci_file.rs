use dal::*;

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Hash, Clone, JsonSchema, Eq, PartialEq)]
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
                    Ok(fk) => {
                        cifiles.push(fk)
                    }
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

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for Cifile {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Cifile>>(), // id
                any::<i16>(),          // priority
                "[a-zA-Z]{1,20}",      // data
            )
                .prop_map(|(id, priority, data)| Cifile { id, priority, data })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_cifile_model(ci_file in any::<Cifile>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();


                let new_row = NewRow::new(ci_file.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_ci_result = Cifile::select().where_field("id").equals(ci_file.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_ci_result.is_ok(), "Retrieval failed: {:?}", retrieved_ci_result.err());

                let first_ci_file = retrieved_ci_result.unwrap().into_iter().next();
                prop_assert!(first_ci_file.is_some(), "No host found, empty result");

                let retrieved_ci_file = first_ci_file.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_ci_file, ci_file);

                Ok(())
            })?
        }
    }
}

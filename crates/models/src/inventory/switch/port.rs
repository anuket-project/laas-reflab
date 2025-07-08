use dal::*;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;

use crate::inventory::Switch;

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, FromRow)]
pub struct SwitchPort {
    pub id: FKey<SwitchPort>,
    pub for_switch: FKey<Switch>,
    pub name: String,
}

impl DBTable for SwitchPort {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "switchports"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_switch: row.try_get("for_switch")?,
            name: row.try_get("name")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("for_switch", Box::new(clone.for_switch)),
            ("name", Box::new(clone.name)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl SwitchPort {
    pub async fn get_or_create_port(
        t: &mut EasyTransaction<'_>,
        on: FKey<Switch>,
        name: String,
    ) -> Result<ExistingRow<SwitchPort>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE for_switch = $1 AND name = $2;");

        let existing = t.query_opt(&q, &[&on, &name.clone()]).await.unwrap();

        match existing {
            Some(r) => Ok(<SwitchPort as dal::DBTable>::from_row(r)?),
            None => {
                // need to create the port
                let sp = SwitchPort {
                    id: FKey::new_id_dangling(),
                    for_switch: on,
                    name,
                };

                Ok(NewRow::new(sp).insert(t).await?.get(t).await?)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for SwitchPort {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<SwitchPort>>(), // id
                any::<FKey<Switch>>(),     // for_switch
                "[a-zA-Z0-9-]{1,50}",      // name
            )
                .prop_map(|(id, for_switch, name)| SwitchPort {
                    id,
                    for_switch,
                    name,
                })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_switch_port_model(switch_port in any::<SwitchPort>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let new_row = NewRow::new(switch_port.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_switch_port = SwitchPort::select()
                    .where_field("id")
                    .equals(switch_port.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_switch_port.is_ok(), "Retrieval failed: {:?}", retrieved_switch_port.err());
                let retrieved_switch_port = retrieved_switch_port.unwrap();

                let first_switch_port = retrieved_switch_port.first();
                prop_assert!(first_switch_port.is_some(), "No SwitchPort found, empty result");

                let retrieved_switch_port = first_switch_port.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_switch_port.id, switch_port.id);
                prop_assert_eq!(retrieved_switch_port.for_switch, switch_port.for_switch);
                prop_assert_eq!(retrieved_switch_port.name, switch_port.name);

                Ok(())
            })?
        }
    }
}

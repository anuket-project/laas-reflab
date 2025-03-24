use dal::{web::AnyWay, *};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::{DataValue, Flavor};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct InterfaceFlavor {
    pub id: FKey<InterfaceFlavor>,
    pub on_flavor: FKey<Flavor>,
    pub name: String, // Interface name
    pub speed: DataValue,
    pub cardtype: CardType,
}

impl DBTable for InterfaceFlavor {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "interface_flavors"
    }

    fn from_row(
        row: tokio_postgres::Row,
    ) -> Result<ExistingRow<Self>, common::prelude::anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            on_flavor: row.try_get("on_flavor")?,

            name: row.try_get("name")?,
            speed: DataValue::from_sqlval(row.try_get("speed")?)?,
            cardtype: serde_json::from_value(row.try_get("cardtype")?)?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("on_flavor", Box::new(self.on_flavor)),
            ("name", Box::new(self.name.clone())),
            ("speed", self.speed.to_sqlval()?),
            ("cardtype", Box::new(serde_json::to_value(self.cardtype)?)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl InterfaceFlavor {
    pub async fn all_for_flavor(
        transaction: &mut EasyTransaction<'_>,
        flavor: FKey<Flavor>,
    ) -> Result<Vec<ExistingRow<Self>>, anyhow::Error> {
        let tn = Self::table_name();
        let q = format!("SELECT * FROM {tn} WHERE on_flavor = $1;");
        let rows = transaction.query(&q, &[&flavor]).await.anyway()?;
        Self::from_rows(rows)
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema, Eq, PartialEq)]
pub enum CardType {
    PCIeOnboard,
    PCIeModular,

    #[default]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::{block_on_runtime, insert_default_model_at};

    fn cardtype_strategy() -> impl Strategy<Value = CardType> {
        prop_oneof![
            Just(CardType::PCIeOnboard),
            Just(CardType::PCIeModular),
            Just(CardType::Unknown),
        ]
    }

    impl Arbitrary for InterfaceFlavor {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<InterfaceFlavor>>(), // id
                any::<FKey<Flavor>>(),          // on_flavor
                any::<String>(),                // name
                any::<DataValue>(),             // speed
                cardtype_strategy(),            // cardtype
            )
                .prop_map(|(id, on_flavor, name, speed, cardtype)| InterfaceFlavor {
                    id,
                    on_flavor,
                    name,
                    speed,
                    cardtype,
                })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_interface_flavor_model(interface_flavor in any::<InterfaceFlavor>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let flavor_insert_result = insert_default_model_at(interface_flavor.on_flavor, &mut transaction).await;
                prop_assert!(flavor_insert_result.is_ok(), "Insert failed while trying to prepare test: {:?}", flavor_insert_result.err());

                let new_row = NewRow::new(interface_flavor.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_interface_flavor = InterfaceFlavor::select()
                    .where_field("id")
                    .equals(interface_flavor.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_interface_flavor.is_ok(), "Retrieval failed: {:?}", retrieved_interface_flavor.err());
                let retrieved_interface_flavor = retrieved_interface_flavor.unwrap();

                let first_interface_flavor = retrieved_interface_flavor.first();
                prop_assert!(first_interface_flavor.is_some(), "No interface flavor found, empty result");

                let retrieved_interface_flavor = first_interface_flavor.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_interface_flavor, interface_flavor);

                Ok(())
            })?
        }
    }
}

use dal::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

use crate::inventory::{Arch, DataValue};

mod extra_info;
mod import;
mod interface;

pub use extra_info::ExtraFlavorInfo;
pub use import::ImportFlavor;
pub use interface::{CardType, InterfaceFlavor};

// Flavor io used to create an instance
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Default)]
pub struct Flavor {
    pub id: FKey<Flavor>,

    pub arch: Arch,
    pub name: String,
    pub public: bool,
    pub cpu_count: usize,
    pub ram: DataValue,
    pub root_size: DataValue,
    pub disk_size: DataValue,
    pub swap_size: DataValue,
    pub brand: String,
    pub model: String,
}

impl DBTable for Flavor {
    fn table_name() -> &'static str {
        "flavors"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("arch", Box::new(self.arch.to_string())),
            ("name", Box::new(self.name.clone())),
            ("public", Box::new(self.public)),
            (
                "cpu_count",
                Box::new(serde_json::to_value(self.cpu_count as i64)?),
            ),
            ("ram", self.ram.to_sqlval()?),
            ("root_size", self.root_size.to_sqlval()?),
            ("disk_size", self.disk_size.to_sqlval()?),
            ("swap_size", self.swap_size.to_sqlval()?),
            ("brand", Box::new(self.brand.clone())),
            ("model", Box::new(self.model.clone())),
        ];
        Ok(c.into_iter().collect())
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            arch: Arch::from_str(row.try_get("arch")?)?,
            name: row.try_get("name")?,
            public: row.try_get("public")?,
            cpu_count: serde_json::from_value::<i64>(row.try_get("cpu_count")?)? as usize,
            ram: DataValue::from_sqlval(row.try_get("ram")?)?,
            root_size: DataValue::from_sqlval(row.try_get("root_size")?)?,
            disk_size: DataValue::from_sqlval(row.try_get("disk_size")?)?,
            swap_size: DataValue::from_sqlval(row.try_get("swap_size")?)?,
            brand: row.try_get("brand")?,
            model: row.try_get("model")?,
        }))
    }
}

impl Flavor {
    pub async fn ports(
        &self,
        transaction: &mut EasyTransaction<'_>,
    ) -> Result<Vec<ExistingRow<InterfaceFlavor>>, anyhow::Error> {
        InterfaceFlavor::all_for_flavor(transaction, self.id).await
    }
}

impl Named for Flavor {
    fn name_columnnames() -> Vec<String> {
        vec!["name".to_owned()]
    }

    fn name_parts(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
}

impl Lookup for Flavor {}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for Flavor {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Flavor>>(), // id
                any::<Arch>(),         // arch
                any::<String>(),       // name
                any::<bool>(),         // public
                (1..=128usize),        // cpu_count
                any::<DataValue>(),    // ram
                any::<DataValue>(),    // root_size
                any::<DataValue>(),    // disk_size
                any::<DataValue>(),    // swap_size
                any::<String>(),       // brand
                any::<String>(),       // model
            )
                .prop_map(
                    |(
                        id,
                        arch,
                        name,
                        public,
                        cpu_count,
                        ram,
                        root_size,
                        disk_size,
                        swap_size,
                        brand,
                        model,
                    )| Flavor {
                        id,
                        arch,
                        name,
                        public,
                        cpu_count,
                        ram,
                        root_size,
                        disk_size,
                        swap_size,
                        brand,
                        model,
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_flavor_model(flavor in any::<Flavor>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let new_row = NewRow::new(flavor.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());


                let retrieved_flavor = Flavor::select()
                    .where_field("id")
                    .equals(flavor.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_flavor.is_ok(), "Retrieval failed: {:?}", retrieved_flavor.err());

                let first_flavor = retrieved_flavor.unwrap().into_iter().next();
                prop_assert!(first_flavor.is_some(), "No flavor found");

                let retrieved = first_flavor.unwrap().clone().into_inner();
                prop_assert_eq!(&retrieved, &flavor);

                Ok(())
            })?
        }
    }
}

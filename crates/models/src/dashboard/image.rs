use common::prelude::reqwest::StatusCode;
use dal::{web::*, *};

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};
use strum_macros::{Display, EnumString};

use crate::{inventory::types::arch::Arch, inventory::Flavor};

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Debug,
    Hash,
    Copy,
    EnumString,
    Display,
    Eq,
    PartialEq,
    Default,
    JsonSchema,
)]
pub enum Distro {
    #[default]
    #[strum(serialize = "Ubuntu")]
    Ubuntu,
    #[strum(serialize = "Fedora")]
    Fedora,
    #[strum(serialize = "EVE")]
    // Needed because the JSON parser defaults to "Eve"
    #[serde(rename = "EVE")]
    Eve,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Image {
    pub id: FKey<Image>, // id of image used for booking
    pub owner: String,
    pub name: String, // name of image
    pub deleted: bool,
    pub cobbler_name: String,
    pub public: bool,
    pub flavors: Vec<FKey<Flavor>>, // vector of compatible flavor IDs
    pub distro: Distro,
    pub version: String,
    pub arch: Arch,
}

impl Named for Image {
    fn name_columnnames() -> Vec<std::string::String> {
        vec!["name".to_owned()]
    }

    fn name_parts(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
}

impl DBTable for Image {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "images"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            owner: row.try_get("owner")?,
            name: row.try_get("name")?,
            deleted: row.try_get("deleted")?,
            cobbler_name: row.try_get("cobbler_name")?,
            public: row.try_get("public")?,
            flavors: row.try_get("flavors")?,
            distro: Distro::from_str(row.try_get("distro")?).unwrap(),
            version: row.try_get("version")?,
            arch: Arch::from_str(row.try_get("arch")?).unwrap(),
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("owner", Box::new(clone.owner)),
            ("name", Box::new(clone.name)),
            ("deleted", Box::new(clone.deleted)),
            ("cobbler_name", Box::new(clone.cobbler_name)),
            ("public", Box::new(clone.public)),
            ("flavors", Box::new(clone.flavors)),
            ("distro", Box::new(clone.distro.to_string())),
            ("version", Box::new(clone.version)),
            ("arch", Box::new(clone.arch.to_string())),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Image {
    pub async fn get_by_name(
        t: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<ExistingRow<Image>, anyhow::Error> {
        let table_name = Self::table_name();
        let query = format!("SELECT * FROM {table_name} WHERE name = $1;");
        let qr = t.query_opt(&query, &[&name]).await?;
        let qr = qr.ok_or(anyhow::Error::msg("Image did not exist for query"))?;

        let results = Image::from_row(qr)
            .log_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database corruption did not allow instantiating an image from a row",
                true,
            )
            .map(|i| i.into_inner())
            .unwrap(); // TODO: get rid of unwrap

        Ok(ExistingRow::from_existing(results))
    }

    pub async fn images_for_flavor(
        t: &mut EasyTransaction<'_>,
        flavor: FKey<Flavor>,
        owner: Option<String>,
    ) -> Result<Vec<Image>, anyhow::Error> {
        if owner.is_some() {
            let table_name = Self::table_name();
            let query = format!("SELECT * FROM {table_name} WHERE (owner = $1 OR public = $2) AND ($3 = ANY(flavors));");
            let qr = t.query(&query, &[&owner, &true, &flavor.into_id()]).await?;

            let results: Vec<Image> = qr
                .into_iter()
                .filter_map(|row| {
                    Image::from_row(row)
                        .log_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "database corruption did not allow instantiating an image from a row",
                            true,
                        )
                        .map(|er| er.into_inner())
                        .ok()
                })
                .collect();

            Ok(results)
        } else {
            let table_name = Self::table_name();
            let query =
                format!("SELECT * FROM {table_name} WHERE (public = $1) AND ($2 = ANY(flavors));");
            let qr = t.query(&query, &[&true, &flavor.into_id()]).await?;

            let results: Vec<Image> = qr
                .into_iter()
                .filter_map(|row| {
                    Image::from_row(row)
                        .log_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "database corruption did not allow instantiating an image from a row",
                            true,
                        )
                        .map(|er| er.into_inner())
                        .ok()
                })
                .collect();

            Ok(results)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::inventory::Arch;

    use super::*;
    use prop::collection::vec;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for Distro {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(Distro::Ubuntu),
                Just(Distro::Fedora),
                Just(Distro::Eve),
            ]
            .boxed()
        }
    }

    impl Arbitrary for Image {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Image>>(),             // id
                "[a-zA-Z]{1,20}",                 // owner
                "[a-zA-Z]{1,20}",                 // name
                any::<bool>(),                    // deleted
                "[a-zA-Z]{1,20}",                 // cobbler_name
                any::<bool>(),                    // public
                vec(any::<FKey<Flavor>>(), 0..3), // flavors
                any::<Distro>(),
                "[a-zA-Z]{1,20}",
                any::<Arch>(),
            )
                .prop_map(
                    |(
                        id,
                        owner,
                        name,
                        deleted,
                        cobbler_name,
                        public,
                        flavors,
                        distro,
                        version,
                        arch,
                    )| Image {
                        id,
                        owner,
                        name,
                        deleted,
                        cobbler_name,
                        public,
                        flavors,
                        distro,
                        version,
                        arch,
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_image_model(image in any::<Image>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();


                let new_row = NewRow::new(image.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_image_result = Image::select().where_field("id").equals(image.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_image_result.is_ok(), "Retrieval failed: {:?}", retrieved_image_result.err());

                let first_image = retrieved_image_result.unwrap().into_iter().next();
                prop_assert!(first_image.is_some(), "No host found, empty result");

                let retrieved_image = first_image.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_image, image);

                Ok(())
            })?
        }
    }
}

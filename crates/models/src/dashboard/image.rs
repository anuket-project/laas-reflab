use common::prelude::reqwest::StatusCode;
use dal::{web::*, *};
use sqlx::PgPool;

use http::Uri;

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashMap, error::Error};
use strum_macros::{Display, EnumString};
use tokio_postgres::types::{private::BytesMut, FromSql, ToSql, Type};
use uuid::Uuid;

use crate::{inventory::types::arch::Arch, inventory::Flavor};

// TODO: put this somewhere it actually belongs
pub mod uri_vec_serde {
    use super::*;

    pub fn serialize<S>(uris: &[Uri], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let strings: Vec<String> = uris.iter().map(|uri| uri.to_string()).collect();
        strings.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Uri>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let strings: Vec<String> = Vec::deserialize(deserializer)?;
        strings
            .into_iter()
            .map(|s| s.parse().map_err(serde::de::Error::custom))
            .collect()
    }
}

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
    sqlx::Type,
    JsonSchema,
)]
#[sqlx(type_name = "distro")]
pub enum Distro {
    #[default]
    #[strum(serialize = "Ubuntu")]
    Ubuntu,
    #[strum(serialize = "Fedora")]
    Fedora,
    #[strum(serialize = "Alma")]
    Alma,
    #[strum(serialize = "EVE")]
    #[serde(rename = "EVE")]
    #[sqlx(rename = "EVE")]
    Eve,
}

// This is another example of something we only need while partially depending on the `dal`
impl FromSql<'_> for Distro {
    fn from_sql(_ty: &Type, raw: &[u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        match s {
            "Ubuntu" => Ok(Distro::Ubuntu),
            "Fedora" => Ok(Distro::Fedora),
            "Alma" => Ok(Distro::Alma),
            "EVE" => Ok(Distro::Eve),
            other => Err(format!("Invalid Distro enum variant: {}", other).into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "distro"
    }
}

// This is another example of something we only need while partially depending on the `dal`
// TODO: Delete when [`dal`] is deprecated
impl ToSql for Distro {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn Error + Sync + Send>> {
        let s = match self {
            Distro::Ubuntu => "Ubuntu",
            Distro::Fedora => "Fedora",
            Distro::Alma => "Alma",
            Distro::Eve => "EVE",
        };
        out.extend_from_slice(s.as_bytes());
        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "distro"
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn Error + Sync + Send>> {
        if !<Self as ToSql>::accepts(ty) {
            return Err(format!("cannot convert to type {}", ty.name()).into());
        }
        self.to_sql(ty, out)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, sqlx::FromRow)]
pub struct Image {
    pub id: FKey<Image>, // id of image used for booking
    pub name: String,    // name of image
    pub cobbler_name: String, // name used for Cobbler integration
    pub deleted: bool,
    pub flavors: Vec<FKey<Flavor>>, // vector of compatible flavor IDs
    pub distro: Distro,
    pub version: String,
    pub arch: Arch,

    #[serde(with = "http_serde::uri")]
    pub http_unattended_install_config_path: Uri,

    #[serde(with = "http_serde::uri")]
    pub http_iso_path: Uri,

    #[serde(with = "http_serde::uri")]
    pub tftp_kernel_path: Uri,

    #[serde(with = "uri_vec_serde")]
    pub tftp_initrd_paths: Vec<Uri>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, sqlx::FromRow)]
pub struct ImageKernelArg {
    pub id: Uuid,
    pub for_image: Uuid,
    pub _key: String,
    pub _value: Option<String>,
}

impl ImageKernelArg {
    pub fn render_to_kernel_arg(&self) -> String {
        match &self._value {
            Some(v) => format!("{}={}", self._key, v),
            None => self._key.clone(),
        }
    }

    pub async fn compile_kernel_args_for_image(
        image_name: &str,
        pool: &PgPool,
    ) -> Result<Vec<String>, sqlx::Error> {
        let kernel_args: Vec<ImageKernelArg> = sqlx::query_as!(
            ImageKernelArg,
            r#"
            SELECT *
            FROM image_kernel_args
            WHERE for_image = (SELECT id FROM images WHERE name = $1)
            ORDER BY _key ASC;
            "#,
            image_name
        )
        .fetch_all(pool)
        .await?;

        Ok(kernel_args
            .into_iter()
            .map(|arg| arg.render_to_kernel_arg())
            .collect())
    }
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
            name: row.try_get("name")?,
            cobbler_name: row.try_get("cobbler_name")?,
            deleted: row.try_get("deleted")?,
            flavors: row.try_get("flavors")?,
            distro: row.try_get::<_, Distro>("distro")?,
            version: row.try_get("version")?,
            arch: row.try_get::<_, Arch>("arch")?,
            http_unattended_install_config_path: row
                .try_get::<_, String>("http_unattended_install_config_path")?
                .parse()?,
            http_iso_path: row.try_get::<_, String>("http_iso_path")?.parse()?,
            tftp_kernel_path: row.try_get::<_, String>("tftp_kernel_path")?.parse()?,
            tftp_initrd_paths: row
                .try_get::<_, Vec<String>>("tftp_initrd_paths")?
                .into_iter()
                .map(|s| s.parse())
                .collect::<Result<Vec<Uri>, _>>()?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let tftp_initrd_paths_strings: Vec<String> = clone
            .tftp_initrd_paths
            .into_iter()
            .map(|uri| uri.to_string())
            .collect();

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("name", Box::new(clone.name)),
            ("cobbler_name", Box::new(clone.cobbler_name)),
            ("deleted", Box::new(clone.deleted)),
            ("flavors", Box::new(clone.flavors)),
            ("distro", Box::new(clone.distro)),
            ("version", Box::new(clone.version)),
            ("arch", Box::new(clone.arch)),
            (
                "http_unattended_install_config_path",
                Box::new(clone.http_unattended_install_config_path.to_string()),
            ),
            ("http_iso_path", Box::new(clone.http_iso_path.to_string())),
            (
                "tftp_kernel_path",
                Box::new(clone.tftp_kernel_path.to_string()),
            ),
            ("tftp_initrd_paths", Box::new(tftp_initrd_paths_strings)),
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
        _owner: Option<String>,
    ) -> Result<Vec<Image>, anyhow::Error> {
        // 'owner' and 'public' columns were removed in migration 015
        let table_name = Self::table_name();
        // TODO: rewrite to not use a raw query.
        let query = format!("SELECT * FROM {table_name} WHERE $1 = ANY(flavors);");
        let qr = t.query(&query, &[&flavor.into_id()]).await?;

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
                Just(Distro::Alma),
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
                any::<FKey<Image>>(),                // id
                "[a-zA-Z]{1,20}",                    // name
                "[a-zA-Z]{1,20}",                    // cobbler_name
                any::<bool>(),                       // deleted
                vec(any::<FKey<Flavor>>(), 0..3),    // flavors
                any::<Distro>(),                     // distro
                "[a-zA-Z]{1,20}",                    // version
                any::<Arch>(),                       // arch
                testing_utils::arb_uri(),            // http_unattended_install_config_path
                testing_utils::arb_uri(),            // http_iso_path
                testing_utils::arb_uri(),            // tftp_kernel_path
                vec(testing_utils::arb_uri(), 0..3), // tftp_initrd_paths
            )
                .prop_map(
                    |(
                        id,
                        name,
                        cobbler_name,
                        deleted,
                        flavors,
                        distro,
                        version,
                        arch,
                        http_unattended_install_config_path,
                        http_iso_path,
                        tftp_kernel_path,
                        tftp_initrd_paths,
                    )| Image {
                        id,
                        name,
                        cobbler_name,
                        deleted,
                        flavors,
                        distro,
                        version,
                        arch,
                        http_unattended_install_config_path,
                        http_iso_path,
                        tftp_kernel_path,
                        tftp_initrd_paths,
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

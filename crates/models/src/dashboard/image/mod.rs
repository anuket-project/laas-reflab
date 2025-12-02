mod kernel_arg;
pub mod serde;

pub use kernel_arg::ImageKernelArg;
pub use serde::{option_uri_serde, uri_vec_serde};

use common::prelude::reqwest::StatusCode;
use common::prelude::*;
use config::settings;
use dal::{web::*, *};
use http::Uri;
use std::collections::HashMap;

use crate::{
    dashboard::types::Distro,
    inventory::{types::arch::Arch, Flavor},
};

#[derive(::serde::Serialize, ::serde::Deserialize, Debug, Clone, Eq, PartialEq, sqlx::FromRow)]
pub struct Image {
    pub id: FKey<Image>,      // id of image used for booking
    pub name: String,         // name of image
    pub cobbler_name: String, // name used for Cobbler integration
    pub deleted: bool,
    pub flavors: Vec<FKey<Flavor>>, // vector of compatible flavor IDs
    pub distro: Distro,
    pub version: String,
    pub arch: Arch,

    #[serde(with = "option_uri_serde")]
    http_unattended_install_config_path: Option<Uri>,

    #[serde(with = "option_uri_serde")]
    http_iso_path: Option<Uri>,

    #[serde(with = "http_serde::uri")]
    tftp_kernel_path: Uri,

    #[serde(with = "uri_vec_serde")]
    tftp_initrd_paths: Vec<Uri>,
}

impl Image {
    /// Creates a new Image
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: FKey<Image>,
        name: String,
        cobbler_name: String,
        distro: Distro,
        version: String,
        arch: Arch,
        tftp_kernel_path: Uri,
        tftp_initrd_paths: Vec<Uri>,
    ) -> Self {
        Self {
            id,
            name,
            cobbler_name,
            deleted: false,
            flavors: Vec::new(),
            distro,
            version,
            arch,
            http_unattended_install_config_path: None,
            http_iso_path: None,
            tftp_kernel_path,
            tftp_initrd_paths,
        }
    }

    /// Sets deleted flag
    pub fn set_deleted(&mut self, deleted: bool) {
        self.deleted = deleted;
    }

    /// Sets flavors list
    pub fn set_flavors(&mut self, flavors: Vec<FKey<Flavor>>) {
        self.flavors = flavors;
    }

    /// Sets HTTP unattended install config path
    pub fn set_http_unattended_install_config_path(&mut self, path: Option<Uri>) {
        self.http_unattended_install_config_path = path;
    }

    /// Sets HTTP ISO path
    pub fn set_http_iso_path(&mut self, path: Option<Uri>) {
        self.http_iso_path = path;
    }

    /// Sets TFTP kernel path
    pub fn set_tftp_kernel_path(&mut self, path: Uri) {
        self.tftp_kernel_path = path;
    }

    /// Sets TFTP initrd paths
    pub fn set_tftp_initrd_paths(&mut self, paths: Vec<Uri>) {
        self.tftp_initrd_paths = paths;
    }

    /// Helper to construct a full HTTP URL
    fn construct_url(base_url: &str, path: &Uri) -> Uri {
        let base = base_url.trim_end_matches('/');
        let path_str = path.to_string();
        let path_clean = path_str.trim_start_matches('/');
        let full_url = format!("{}/{}", base, path_clean);
        full_url
            .parse()
            .expect("Failed to parse crafted URL as Uri")
    }

    /// Returns the full HTTP URL for unattended install config path
    pub fn http_unattended_install_config_url(&self) -> Option<Uri> {
        let base_url = &settings().pxe.http_base_url;
        self.http_unattended_install_config_path
            .as_ref()
            .map(|uri| Self::construct_url(base_url, uri))
    }

    /// Returns the full HTTP URL for ISO path
    pub fn http_iso_url(&self) -> Option<Uri> {
        let base_url = &settings().pxe.http_base_url;
        self.http_iso_path
            .as_ref()
            .map(|uri| Self::construct_url(base_url, uri))
    }

    /// Returns the full URL for kernel path
    pub fn tftp_kernel_url(&self) -> Uri {
        let base_url = &settings().pxe.http_base_url;
        Self::construct_url(base_url, &self.tftp_kernel_path)
    }

    /// Returns the full URLs for all initrd paths
    pub fn tftp_initrd_urls(&self) -> Vec<Uri> {
        let base_url = &settings().pxe.http_base_url;
        self.tftp_initrd_paths
            .iter()
            .map(|uri| Self::construct_url(base_url, uri))
            .collect()
    }

    /// Returns the relative path for the unattended install config
    pub fn http_unattended_install_config_path(&self) -> Option<&Uri> {
        self.http_unattended_install_config_path.as_ref()
    }

    /// Returns the relative path for the ISO
    pub fn http_iso_path(&self) -> Option<&Uri> {
        self.http_iso_path.as_ref()
    }

    /// Returns the relative path for the kernel
    pub fn tftp_kernel_path(&self) -> &Uri {
        &self.tftp_kernel_path
    }

    /// Returns the relative initrd paths
    pub fn tftp_initrd_paths(&self) -> &[Uri] {
        &self.tftp_initrd_paths
    }

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
                .try_get::<_, Option<String>>("http_unattended_install_config_path")?
                .map(|s| s.parse())
                .transpose()?,
            http_iso_path: row
                .try_get::<_, Option<String>>("http_iso_path")?
                .map(|s| s.parse())
                .transpose()?,
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
                Box::new(
                    clone
                        .http_unattended_install_config_path
                        .map(|u| u.to_string()),
                ),
            ),
            (
                "http_iso_path",
                Box::new(clone.http_iso_path.map(|u| u.to_string())),
            ),
            (
                "tftp_kernel_path",
                Box::new(clone.tftp_kernel_path.to_string()),
            ),
            ("tftp_initrd_paths", Box::new(tftp_initrd_paths_strings)),
        ];

        Ok(c.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
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
                any::<FKey<Image>>(),                           // id
                "[a-zA-Z]{1,20}",                               // name
                "[a-zA-Z]{1,20}",                               // cobbler_name
                any::<bool>(),                                  // deleted
                vec(any::<FKey<Flavor>>(), 0..3),               // flavors
                any::<Distro>(),                                // distro
                "[a-zA-Z]{1,20}",                               // version
                any::<Arch>(),                                  // arch
                proptest::option::of(testing_utils::arb_uri()), // http_unattended_install_config_path
                proptest::option::of(testing_utils::arb_uri()), // http_iso_path
                testing_utils::arb_uri(),                       // tftp_kernel_path
                vec(testing_utils::arb_uri(), 0..3),            // tftp_initrd_paths
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

use dal::*;
use serde::{Deserialize, Serialize};
use sqlx;
use sqlx::PgPool;
use std::collections::HashMap;

use crate::{
    dashboard::{image::Distro, Image},
    inventory::{Arch, StorageType},
};

mod extra_info;
mod interface;

pub use extra_info::ExtraFlavorInfo;
pub use interface::{CardType, InterfaceFlavor};

// Flavor io used to create an instance
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Default, sqlx::FromRow)]
pub struct Flavor {
    pub id: FKey<Flavor>,
    pub name: String,
    pub description: Option<String>,
    pub arch: Arch,
    pub cpu_count: Option<i32>,
    pub cpu_frequency_mhz: Option<i32>,
    pub cpu_model: Option<String>,
    pub ram_bytes: Option<i64>,
    pub root_size_bytes: Option<i64>,
    pub disk_size_bytes: Option<i64>,
    pub storage_type: Option<StorageType>,
    pub network_speed_mbps: Option<i32>,
    pub network_interfaces: Option<i32>,
    pub brand: Option<String>,
    pub model: Option<String>,
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
            ("name", Box::new(self.name.clone())),
            ("description", Box::new(self.description.clone())),
            ("arch", Box::new(self.arch)),
            ("cpu_count", Box::new(self.cpu_count)),
            ("cpu_frequency_mhz", Box::new(self.cpu_frequency_mhz)),
            ("cpu_model", Box::new(self.cpu_model.clone())),
            ("ram_bytes", Box::new(self.ram_bytes)),
            ("root_size_bytes", Box::new(self.root_size_bytes)),
            ("disk_size_bytes", Box::new(self.disk_size_bytes)),
            ("storage_type", Box::new(self.storage_type)),
            ("network_speed_mbps", Box::new(self.network_speed_mbps)),
            ("network_interfaces", Box::new(self.network_interfaces)),
            ("brand", Box::new(self.brand.clone())),
            ("model", Box::new(self.model.clone())),
        ];
        Ok(c.into_iter().collect())
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            arch: row.try_get::<_, Arch>("arch")?,
            cpu_count: row.try_get("cpu_count")?,
            cpu_frequency_mhz: row.try_get("cpu_frequency_mhz")?,
            cpu_model: row.try_get("cpu_model")?,
            ram_bytes: row.try_get("ram_bytes")?,
            root_size_bytes: row.try_get("root_size_bytes")?,
            disk_size_bytes: row.try_get("disk_size_bytes")?,
            storage_type: row.try_get("storage_type")?,
            network_speed_mbps: row.try_get("network_speed_mbps")?,
            network_interfaces: row.try_get("network_interfaces")?,
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

    pub async fn get_images(&self, pool: &PgPool) -> Vec<Image> {
        let image_records = sqlx::query!(
            r#"SELECT id, name, deleted, flavors, distro as "distro: Distro", cobbler_name, version, arch as "arch: Arch", http_unattended_install_config_path, http_iso_path, tftp_kernel_path, tftp_initrd_paths FROM images where $1=ANY(flavors)"#,
            self.id().into_uuid()
        )
        .fetch_all(pool)
        .await
        .unwrap();

        if image_records.is_empty() {
            return vec![];
        }

        let mut ret_image_vec: Vec<Image> = vec![];
        for image_record in image_records {
            let mut flavors: Vec<FKey<Flavor>> = vec![];
            for flavor_uuid in image_record.flavors {
                flavors.push(FKey::from_id(ID::from(flavor_uuid)));
            }

            let tftp_initrd_paths: Vec<http::Uri> = image_record
                .tftp_initrd_paths
                .into_iter()
                .filter_map(|s| s.parse().ok())
                .collect();

            let image: Image = Image {
                id: FKey::from_id(ID::from(image_record.id)),
                name: image_record.name,
                deleted: image_record.deleted,
                flavors,
                distro: image_record.distro,
                version: image_record.version,
                arch: image_record.arch,
                cobbler_name: image_record.cobbler_name,
                http_unattended_install_config_path: image_record
                    .http_unattended_install_config_path
                    .parse()
                    .unwrap_or_else(|_| "/".parse().unwrap()),
                http_iso_path: image_record
                    .http_iso_path
                    .parse()
                    .unwrap_or_else(|_| "/".parse().unwrap()),
                tftp_kernel_path: image_record
                    .tftp_kernel_path
                    .parse()
                    .unwrap_or_else(|_| "/".parse().unwrap()),
                tftp_initrd_paths,
            };

            ret_image_vec.push(image);
        }

        ret_image_vec
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
            // WARNING: proptest only works with tuples of 16 elements or less, this must be broken
            // into two parts.
            let first = (
                any::<FKey<Flavor>>(),   // id
                "[a-zA-Z]{1,20}",        // name
                any::<Option<String>>(), // description
                any::<Arch>(),           // arch
                any::<Option<i32>>(),    // cpu_count
                any::<Option<i32>>(),    // cpu_frequency_mhz
                any::<Option<String>>(), // cpu_model
                any::<Option<i64>>(),    // ram_bytes
                any::<Option<i64>>(),    // root_size_bytes
                any::<Option<i64>>(),    // disk_size_bytes
            );

            let second = (
                prop_oneof![Just(None), any::<StorageType>().prop_map(Some)].boxed(), // storage_type
                any::<Option<i32>>(),    // network_speed_mbps
                any::<Option<i32>>(),    // network_interfaces
                any::<Option<String>>(), // brand
                any::<Option<String>>(), // model
            );

            (first, second)
                .prop_map(
                    |(
                        (
                            id,
                            name,
                            description,
                            arch,
                            cpu_count,
                            cpu_frequency_mhz,
                            cpu_model,
                            ram_bytes,
                            root_size_bytes,
                            disk_size_bytes,
                        ),
                        (storage_type, network_speed_mbps, network_interfaces, brand, model),
                    )| Flavor {
                        id,
                        name,
                        description,
                        arch,
                        cpu_count,
                        cpu_frequency_mhz,
                        cpu_model,
                        ram_bytes,
                        root_size_bytes,
                        disk_size_bytes,
                        storage_type,
                        network_speed_mbps,
                        network_interfaces,
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

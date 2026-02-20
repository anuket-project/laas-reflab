use std::io::Error;

use dal::*;

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::dashboard::image::Image;
use crate::dashboard::types::ci_file::Cifile;
use crate::dashboard::types::BondGroupConfig;
use crate::inventory::Flavor;

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, Eq, PartialEq, Default)]
pub struct HostConfig {
    pub hostname: String, // Hostname that the user would like
    pub flavor: FKey<Flavor>,
    pub image: FKey<Image>, // Name of image used to match image id during provisioning
    /// DO NOT USE, call get_ci_file()
    pub cifile: Vec<FKey<Cifile>>, // To-Do: change this into an Option<String> so we can remove extra impl and warnings
    pub connections: Vec<BondGroupConfig>,
}

impl HostConfig {
    pub async fn get_ci_file(self) -> Result<Option<String>, Error> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();
        if self.cifile.len() > 1 {
            panic!("There is more than 1 ci file, there should only ever be 1 ci file")
        } else if self.cifile.is_empty() {
            Ok(Option::None)
        } else {
            Ok(Option::Some(
                self.cifile
                    .first()
                    .unwrap()
                    .get(&mut transaction)
                    .await
                    .unwrap()
                    .into_inner()
                    .data,
            ))
        }
    }

    pub async fn new(
        hostname: String,
        flavor: FKey<Flavor>,
        image: FKey<Image>,
        cifile_content: Option<String>,
        connections: Vec<BondGroupConfig>,
    ) -> Self {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let ci_file_vec: Vec<FKey<Cifile>> = match cifile_content {
            Some(content) => {
                info!("Got CI file with content: {}", content);
                Cifile::new(&mut transaction, vec![content]).await.unwrap()
            }
            None => {
                info!("Ci file has no content");
                vec![]
            }
        };

        transaction.commit().await.unwrap();

        HostConfig {
            hostname,
            flavor,
            image,
            cifile: ci_file_vec,
            connections,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::*;

    impl Arbitrary for HostConfig {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                "[a-zA-Z]{1,20}",                     // hostname
                any::<FKey<Flavor>>(),                // flavor
                any::<FKey<Image>>(),                 // image
                vec(any::<FKey<Cifile>>(), 0..10),    // cifile
                vec(any::<BondGroupConfig>(), 0..10), // connections
            )
                .prop_map(
                    |(hostname, flavor, image, cifile, connections)| HostConfig {
                        hostname,
                        flavor,
                        image,
                        cifile,
                        connections,
                    },
                )
                .boxed()
        }
    }
}

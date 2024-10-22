use dal::*;

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dashboard::types::{BondGroupConfig, ImportBondGroupConfig};
use crate::dashboard::{ci_file::Cifile, image::Image};
use crate::inventory::Flavor;

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct HostConfig {
    pub hostname: String, // Hostname that the user would like

    pub flavor: FKey<Flavor>,
    pub image: FKey<Image>, // Name of image used to match image id during provisioning
    pub cifile: Vec<FKey<Cifile>>, // A vector of C-I Files. order is determined by order of the Vec

    pub connections: Vec<BondGroupConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct ImportHostConfig {
    pub hostname: String,
    pub image: String,
    pub flavor: String,
    pub cifile: Vec<Cifile>,
    pub connections: Vec<ImportBondGroupConfig>,
}

impl ImportHostConfig {
    pub async fn to_host_config(&self, transaction: &mut EasyTransaction<'_>) -> HostConfig {
        let clone = self.clone();

        let image = Image::lookup(transaction, vec![clone.image.clone()])
            .await
            .unwrap_or_else(|_| panic!("Expected to find image named {}", clone.image))
            .id;

        let mut cifile: Vec<FKey<Cifile>> = Vec::new();
        for cf in clone.cifile {
            cifile.push(cf.id);
        }

        let flavor = Flavor::lookup(transaction, vec![clone.flavor.clone()])
            .await
            .unwrap_or_else(|_| panic!("Expected to find flavor named {}", clone.flavor))
            .id;

        let mut connections = Vec::new();

        for conn in clone.connections {
            connections.push(conn.to_bgc(transaction).await);
        }

        HostConfig {
            hostname: clone.hostname,
            flavor,
            image,
            cifile,
            connections,
        }
    }

    pub async fn from_host_config(
        transaction: &mut EasyTransaction<'_>,
        host_config: &HostConfig,
    ) -> ImportHostConfig {
        let clone = host_config.clone();

        let image = clone
            .image
            .get(transaction)
            .await
            .expect("Expected to find image");

        let mut cifile: Vec<Cifile> = Vec::new();
        for cf in clone.cifile {
            cifile.push(
                cf.get(transaction)
                    .await
                    .expect("Expected to find cifile")
                    .into_inner(),
            );
        }

        let flavor = clone
            .flavor
            .get(transaction)
            .await
            .expect("Expected to find flavor")
            .name
            .clone();

        let mut connections = Vec::new();

        for conn in clone.connections {
            connections.push(ImportBondGroupConfig::from_bgc(conn, transaction).await);
        }

        ImportHostConfig {
            hostname: clone.hostname,
            image: image.name.clone(),
            flavor,
            cifile,
            connections,
        }
    }
}

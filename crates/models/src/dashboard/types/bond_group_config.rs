use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::dashboard::types::vlan_connection_config::{
    ImportVlanConnectionConfig, VlanConnectionConfig,
};

use dal::*;

#[derive(Serialize, Deserialize, Debug, Clone, Default, JsonSchema)]
pub struct BondGroupConfig {
    pub connects_to: HashSet<VlanConnectionConfig>,
    pub member_interfaces: HashSet<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, JsonSchema)]
pub struct ImportBondGroupConfig {
    pub connects_to: HashSet<ImportVlanConnectionConfig>,
    pub member_interfaces: HashSet<String>,
}

impl ImportBondGroupConfig {
    pub async fn to_bgc(&self, transaction: &mut EasyTransaction<'_>) -> BondGroupConfig {
        let mut connections = HashSet::new();

        for conf in self.connects_to.clone() {
            connections.insert(conf.to_vcc(transaction).await);
        }

        BondGroupConfig {
            connects_to: connections,
            member_interfaces: self.member_interfaces.clone(),
        }
    }

    pub async fn from_bgc(
        bgc: BondGroupConfig,
        transaction: &mut EasyTransaction<'_>,
    ) -> ImportBondGroupConfig {
        let mut connections = HashSet::new();

        for conf in bgc.connects_to.clone() {
            connections.insert(ImportVlanConnectionConfig::from_vcc(conf, transaction).await);
        }

        ImportBondGroupConfig {
            connects_to: connections,
            member_interfaces: bgc.member_interfaces.clone(),
        }
    }
}

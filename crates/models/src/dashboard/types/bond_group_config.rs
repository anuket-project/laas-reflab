use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::dashboard::types::vlan_connection_config::{
    ImportVlanConnectionConfig, VlanConnectionConfig,
};

use dal::*;

#[derive(Serialize, Deserialize, Debug, Clone, Default, JsonSchema, Eq, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::hash_set;
    use proptest::prelude::*;

    impl Arbitrary for BondGroupConfig {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                hash_set(any::<VlanConnectionConfig>(), 1..10),
                hash_set(any::<String>(), 1..10),
            )
                .prop_map(|(connects_to, member_interfaces)| BondGroupConfig {
                    connects_to,
                    member_interfaces,
                })
                .boxed()
        }
    }
}

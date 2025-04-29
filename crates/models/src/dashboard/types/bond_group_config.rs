use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::dashboard::types::vlan_connection_config::VlanConnectionConfig;

use dal::*;

#[derive(Serialize, Deserialize, Debug, Clone, Default, JsonSchema, Eq, PartialEq)]
pub struct BondGroupConfig {
    pub connects_to: HashSet<VlanConnectionConfig>,
    pub member_interfaces: HashSet<String>,
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

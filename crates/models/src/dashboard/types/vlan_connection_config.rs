use crate::dashboard::Network;
use dal::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, JsonSchema)]
pub struct VlanConnectionConfig {
    pub network: FKey<Network>,
    pub tagged: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, JsonSchema)]
pub struct ImportVlanConnectionConfig {
    pub network: String, // network name
    pub tagged: bool,
}

impl ImportVlanConnectionConfig {
    pub async fn to_vcc(&self, transaction: &mut EasyTransaction<'_>) -> VlanConnectionConfig {
        VlanConnectionConfig {
            network: Network::select()
                .where_field("name")
                .equals(self.network.clone())
                .run(transaction)
                .await
                .expect("Expected to find network")
                .first()
                .expect("Expected to find network")
                .id,
            tagged: self.tagged,
        }
    }

    pub async fn from_vcc(
        vcc: VlanConnectionConfig,
        transaction: &mut EasyTransaction<'_>,
    ) -> ImportVlanConnectionConfig {
        ImportVlanConnectionConfig {
            network: vcc
                .network
                .get(transaction)
                .await
                .expect("Expected to find network")
                .name
                .clone(),
            tagged: vcc.tagged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for VlanConnectionConfig {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (any::<FKey<Network>>(), any::<bool>())
                .prop_map(|(network, tagged)| VlanConnectionConfig { network, tagged })
                .boxed()
        }
    }
}

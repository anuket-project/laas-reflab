use crate::dashboard::Network;
use dal::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, JsonSchema)]
pub struct VlanConnectionConfig {
    pub network: FKey<Network>,
    pub tagged: bool,
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

use dal::*;

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dashboard::types::BondGroupConfig;
use crate::dashboard::{ci_file::Cifile, image::Image};
use crate::inventory::Flavor;

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, Eq, PartialEq, Default)]
pub struct HostConfig {
    pub hostname: String, // Hostname that the user would like
    pub flavor: FKey<Flavor>,
    pub image: FKey<Image>, // Name of image used to match image id during provisioning
    pub cifile: Vec<FKey<Cifile>>, // A vector of C-I Files. order is determined by order of the Vec
    pub connections: Vec<BondGroupConfig>,
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

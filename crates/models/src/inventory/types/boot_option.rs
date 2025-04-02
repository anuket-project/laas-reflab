use serde::{Deserialize, Serialize};

use strum_macros::Display;

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, Display)]
pub enum BootTo {
    #[strum(serialize = "Network")]
    Network,
    #[strum(serialize = "Disk")]
    #[default]
    Disk,
    #[strum(serialize = "Specific Disk")]
    SpecificDisk,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for BootTo {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(BootTo::Network),
                Just(BootTo::Disk),
                Just(BootTo::SpecificDisk),
            ]
            .boxed()
        }
    }
}

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Copy, PartialEq, Eq)]
pub enum StatusSentiment {
    Succeeded,
    InProgress,
    Degraded,
    Failed,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for StatusSentiment {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(StatusSentiment::Succeeded),
                Just(StatusSentiment::InProgress),
                Just(StatusSentiment::Degraded),
                Just(StatusSentiment::Failed),
                Just(StatusSentiment::Unknown),
            ]
            .boxed()
        }
    }
}

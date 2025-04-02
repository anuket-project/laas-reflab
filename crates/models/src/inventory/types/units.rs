use common::prelude::*;
use dal::web::AnyWay;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema, PartialEq, Eq)]
pub struct DataValue {
    pub value: u64,
    pub unit: DataUnit,
}

// TODO: This should really be refactored to have two seperate enums for network speed and storage
// units. Also Unknown as a default enum variant when it doesn't represent a relevant value is an
// actual antipattern (also lets be real when are we going to have anything other than GB of RAM)
#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema, PartialEq, Eq)]
pub enum DataUnit {
    #[default]
    Unknown,

    Bytes,
    KiloBytes,
    MegaBytes,
    GigaBytes,
    TeraBytes,

    Bits,

    BitsPerSecond,
    KiloBitsPerSecond,
    MegaBitsPerSecond,
    GigaBitsPerSecond,
}

impl DataValue {
    /// if is_bytes is false, the passed value is instead
    pub fn from_decimal(s: &str, value_type: DataUnit) -> Option<Self> {
        parse_size::parse_size(s)
            .map(|v| DataValue {
                value: v,
                unit: value_type,
            })
            .ok()
    }

    pub fn to_sqlval(&self) -> Result<Box<serde_json::Value>, anyhow::Error> {
        serde_json::to_value(self).map(Box::new).anyway()
    }

    pub fn from_sqlval(v: serde_json::Value) -> Result<Self, anyhow::Error> {
        serde_json::from_value(v).anyway()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for DataUnit {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(DataUnit::Unknown),
                Just(DataUnit::Bytes),
                Just(DataUnit::KiloBytes),
                Just(DataUnit::MegaBytes),
                Just(DataUnit::GigaBytes),
                Just(DataUnit::TeraBytes),
                Just(DataUnit::Bits),
                Just(DataUnit::BitsPerSecond),
                Just(DataUnit::KiloBitsPerSecond),
                Just(DataUnit::MegaBitsPerSecond),
                Just(DataUnit::GigaBitsPerSecond),
            ]
            .boxed()
        }
    }

    impl Arbitrary for DataValue {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (any::<u64>(), any::<DataUnit>())
                .prop_map(|(value, unit)| DataValue { value, unit })
                .boxed()
        }
    }
}

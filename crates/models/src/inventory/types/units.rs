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

mod sqlx_impl {
    use serde_json::Value as JsonValue;
    use sqlx::{
        encode::IsNull,
        postgres::{PgTypeInfo, PgValueRef},
        Database, Decode, Encode, Postgres, Type,
    };
    use std::error::Error;

    use super::DataValue;

    impl Type<Postgres> for DataValue {
        fn type_info() -> PgTypeInfo {
            // delegate to serde_json::Value’s JSONB type
            <JsonValue as Type<Postgres>>::type_info()
        }
    }

    impl<'r> Decode<'r, Postgres> for DataValue {
        fn decode(value: PgValueRef<'r>) -> Result<Self, Box<dyn Error + Send + Sync>> {
            // first decode into serde_json::Value
            let raw: JsonValue = Decode::decode(value)?;
            // if the JSON is null, return the default DataValue
            match raw {
                JsonValue::Null => Ok(DataValue::default()),
                other => serde_json::from_value(other).map_err(|e| Box::new(e) as _),
            }
        }
    }

    impl<'q> Encode<'q, Postgres> for DataValue {
        fn encode_by_ref(
            &self,
            buf: &mut <Postgres as Database>::ArgumentBuffer<'q>,
        ) -> Result<IsNull, Box<dyn Error + Send + Sync>> {
            let raw = serde_json::to_value(self)?;
            <JsonValue as Encode<'q, Postgres>>::encode(raw, buf)
        }

        fn produces(&self) -> Option<<Postgres as Database>::TypeInfo> {
            Some(<JsonValue as Type<Postgres>>::type_info())
        }

        fn size_hint(&self) -> usize {
            match serde_json::to_value(self) {
                Ok(raw) => <JsonValue as Encode<'q, Postgres>>::size_hint(&raw),
                Err(_) => 0,
            }
        }
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

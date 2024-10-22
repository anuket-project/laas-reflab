use common::prelude::*;
use dal::web::AnyWay;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema, PartialEq, Eq)]
pub struct DataValue {
    pub value: u64,
    pub unit: DataUnit,
}

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

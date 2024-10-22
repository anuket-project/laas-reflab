use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Copy)]
pub enum StatusSentiment {
    Succeeded,
    InProgress,
    Degraded,
    Failed,
    Unknown,
}

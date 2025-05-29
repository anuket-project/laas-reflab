use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionYaml {
    pub switch_name: String,
    pub switchport_name: String,
}

use serde::{Deserialize, Serialize};

/// Represents a YAML level connection between an interface (hostport) and a switchport.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Hash)]
pub struct ConnectionYaml {
    pub switch_name: String,
    pub switchport_name: String,
}

use super::BondGroup;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Hash)]
pub struct NetworkConfig {
    pub bondgroups: Vec<BondGroup>,
    pub persist: bool,
}

impl std::fmt::Debug for NetworkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NetworkConfig with persist {} and <some> bondgroups",
            self.persist
        )
    }
}

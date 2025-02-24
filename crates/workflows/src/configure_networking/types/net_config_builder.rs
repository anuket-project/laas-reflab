use super::{BondGroup, NetworkConfig};

#[derive(Debug, Clone)]
pub struct NetworkConfigBuilder {
    based_on: NetworkConfig,
}

impl Default for NetworkConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkConfigBuilder {
    pub fn new() -> Self {
        Self {
            based_on: NetworkConfig {
                bondgroups: vec![],
                persist: true,
            },
        }
    }

    pub fn persist(self, persist: bool) -> Self {
        Self {
            based_on: NetworkConfig {
                bondgroups: self.based_on.bondgroups,
                persist,
            },
        }
    }

    pub fn bond(mut self, b: BondGroup) -> Self {
        self.based_on.bondgroups.push(b);

        self
    }

    pub fn build(self) -> NetworkConfig {
        self.based_on
    }
}

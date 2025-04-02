use crate::{
    dashboard::Cifile,
    inventory::{Flavor, Vlan},
};
use dal::*;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProvEvent {
    pub event: String,
    pub details: String,
}

impl ProvEvent {
    pub fn new<A, B>(event: A, details: B) -> Self
    where
        A: Into<String>,
        B: Into<String>,
    {
        Self {
            event: event.into(),
            details: details.into(),
        }
    }
}

impl fmt::Display for ProvEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -- {}", self.event, self.details)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct NetworkProvData {
    pub network_name: String,
    pub hostname: String,
    pub public: bool,
    pub tagged: bool,
    pub iface: String,
    pub vlan_id: FKey<Vlan>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstanceProvData {
    pub hostname: String,
    pub flavor: FKey<Flavor>,
    pub image: String,
    pub cifile: Vec<Cifile>,
    pub ipmi_create: bool,
    pub networks: Vec<NetworkProvData>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for ProvEvent {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (any::<String>(), any::<String>())
                .prop_map(|(event, details)| ProvEvent { event, details })
                .boxed()
        }
    }
}

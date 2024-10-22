use dal::*;
use serde::{Deserialize, Serialize};

use crate::{
    allocator::ResourceHandle,
    inventory::{Arch, DataValue, Flavor, Host, Lab, Vlan},
};
#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub enum ResourceClass {
    None,
    Host,
    PrivateVlan,
    PublicVlan,
    VPNAccess,
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub enum ResourceRequestInner {
    VlanByCharacteristics {
        public: bool,
        serves_dhcp: bool,
        lab: FKey<Lab>,
    },

    SpecificVlan {
        vlan: FKey<Vlan>,
        lab: FKey<Lab>,
    },

    HostByFlavor {
        flavor: FKey<Flavor>,
        lab: FKey<Lab>,
    },

    HostByCharacteristics {
        arch: Option<Arch>,
        minimum_ram: Option<DataValue>,
        maximum_ram: Option<DataValue>,
        minimum_cores: Option<DataValue>,
        maximum_cores: Option<DataValue>,
        lab: FKey<Lab>,
    },

    SpecificHost {
        host: FKey<Host>,
        lab: FKey<Lab>,
    },

    VPNAccess {
        for_project: String,
        for_user: String,
        lab: FKey<Lab>,
    },

    /// Deallocates this resource only so long
    /// as the handle is owned by/allocated for
    /// the aggregate in `for_aggregate`
    DeallocateHost {
        resource: ResourceHandle,
    },

    /// Deallocates all resources relating to the given `for_aggregate`
    DeallocateAll {},
}

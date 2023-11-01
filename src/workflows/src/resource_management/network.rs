//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::macaddr;

use macaddr::MacAddr6;
use models::{
    dal::{DBTable, EasyTransaction, FKey},
    inventory,
    inventory::Vlan,
};
use serde::{Deserialize, Serialize};
use tascii::prelude::*;

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

pub struct NetworkConfigBuilder {
    based_on: NetworkConfig,
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

impl NetworkConfig {}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct BondGroup {
    pub member_host_ports: Vec<FKey<inventory::HostPort>>,
    pub vlans: Vec<VlanConnection>,
}

impl BondGroup {
    pub fn with_vlans<Iter, Item>(mut self, vlans: Iter)
    where
        VlanConnection: From<Item>,
        Iter: IntoIterator<Item = Item>,
    {
        for it in vlans.into_iter() {
            let v: VlanConnection = it.into();

            self.vlans.push(v);
        }
    }

    pub fn with_ports<Iter>(mut self, ports: Iter)
    where Iter: IntoIterator<Item = FKey<inventory::HostPort>> {
        for it in ports.into_iter() {
            self.member_host_ports.push(it);
        }
    }

    pub fn with_vlan<Item>(mut self, vc: Item) -> Self
    where VlanConnection: From<Item> {
        self.vlans.push(vc.into());
        self
    }

    pub fn with_port(mut self, machine_port: FKey<inventory::HostPort>) -> Self {
        self.member_host_ports.push(machine_port);
        self
    }

    pub fn new() -> Self {
        Self {
            member_host_ports: vec![],
            vlans: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, Serialize, Deserialize)]
pub struct VlanConnection {
    pub vlan: FKey<models::inventory::Vlan>,
    pub tagged: bool,
}

impl VlanConnection {
    pub async fn from_pair(t: &mut EasyTransaction<'_>, vlan_id: u16, tagged: bool) -> Self {
        Self {
            vlan: Vlan::select()
                .where_field("vlan_id")
                .equals(vlan_id as i16)
                .run(t)
                .await
                .expect("need at least the ipmi vlan, hardcode requirement")
                .get(0)
                .unwrap()
                .id,
            tagged,
        }
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct SwitchPort {
    mac_address: Option<MacAddr6>,
    port_name: String, // ex: Ethernet1/23
    switch: SwitchIdentifier,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct SwitchIdentifier {
    fqdn: String,
}

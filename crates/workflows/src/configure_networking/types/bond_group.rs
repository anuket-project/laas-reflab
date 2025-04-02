use dal::FKey;

use super::VlanConnection;
use models::inventory::HostPort;
use std::collections::HashMap;
use tascii::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct BondGroup {
    pub member_host_ports: Vec<FKey<HostPort>>,
    pub vlans: Vec<VlanConnection>,
}

impl Default for BondGroup {
    fn default() -> Self {
        Self::new()
    }
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
    where
        Iter: IntoIterator<Item = FKey<HostPort>>,
    {
        for it in ports.into_iter() {
            self.member_host_ports.push(it);
        }
    }

    pub fn with_vlan<Item>(mut self, vc: Item) -> Self
    where
        VlanConnection: From<Item>,
    {
        self.vlans.push(vc.into());
        self
    }

    pub fn with_port(mut self, machine_port: FKey<HostPort>) -> Self {
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

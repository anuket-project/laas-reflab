use serde::{Deserialize, Serialize};

mod host;
mod interface;
mod ipmi;
mod parse;

pub(crate) use host::{HostInfo, HostYaml};
pub(crate) use interface::{ConnectionYaml, InterfaceYaml};
pub(crate) use parse::load_inventory_hosts;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum Report {}

// TODO: make this make a little more sense with hosts as a vec in a yaml list
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryYaml {
    pub host: HostYaml,
}

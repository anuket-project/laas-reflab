use serde::{Deserialize, Serialize};

mod host;
mod interface;
mod ipmi;
mod parse;
mod switch;

pub(crate) use host::{HostInfo, HostYaml};
pub(crate) use interface::{
    InterfaceYaml, generate_created_interface_reports, generate_interface_reports,
};
pub(crate) use ipmi::IpmiYaml;
pub(crate) use parse::load_inventory;
pub(crate) use switch::{SwitchDatabaseInfo, SwitchYaml};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryYaml {
    /// All switches in this inventory
    #[serde(default)]
    pub switches: Vec<SwitchYaml>,

    /// All hosts in this inventory
    #[serde(default)]
    pub hosts: Vec<HostYaml>,
}

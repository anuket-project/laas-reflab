use serde::{Deserialize, Serialize};

mod flavor;
mod host;
mod image;
mod interface;
mod ipmi;
mod lab;
mod parse;
mod switch;

pub(crate) use flavor::FlavorYaml;
pub(crate) use host::{HostInfo, HostYaml};
pub(crate) use image::{ImageYaml, KernelArg};
pub(crate) use interface::{
    InterfaceYaml, generate_created_interface_reports, generate_interface_reports,
};
pub(crate) use ipmi::IpmiYaml;
pub(crate) use lab::LabYaml;
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

    // All images in this inventory
    #[serde(default)]
    pub images: Vec<ImageYaml>,

    // All flavors in this inventory
    #[serde(default)]
    pub flavors: Vec<FlavorYaml>,

    // All labs in this inventory
    #[serde(default)]
    pub labs: Vec<LabYaml>,
}

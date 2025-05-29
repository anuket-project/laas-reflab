use mac_address::MacAddress;
use serde::{Deserialize, Serialize};

pub use connection::ConnectionYaml;

pub(crate) mod connection;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterfaceYaml {
    pub name: String,
    pub mac: MacAddress,
    pub bus_addr: String,
    pub connection: ConnectionYaml,
}

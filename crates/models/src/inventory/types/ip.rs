use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct IPInfo<IP: Serialize + std::fmt::Debug + Clone> {
    pub subnet: IP,
    pub netmask: u8,
    pub gateway: Option<IP>,
    pub provides_dhcp: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IPNetwork {
    pub v4: Option<IPInfo<Ipv4Addr>>,
    pub v6: Option<IPInfo<Ipv6Addr>>,
}

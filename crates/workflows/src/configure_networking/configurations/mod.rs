mod empty;
mod management;
mod production;

pub use empty::empty_network_config;
pub use management::{mgmt_network_config, mgmt_network_config_with_public};
pub use production::prod_network_config;

mod flavor;
mod host;
mod lab;
mod switch;
pub(crate) mod types;
mod vlan;

pub use flavor::{CardType, ExtraFlavorInfo, Flavor, InterfaceFlavor};
pub use host::{Host, HostPort};
pub use lab::Lab;
pub use switch::{Switch, SwitchOS, SwitchPort};
pub use types::{Arch, BootTo, DataUnit, DataValue, IPInfo, IPNetwork, StorageType};
pub use vlan::Vlan;

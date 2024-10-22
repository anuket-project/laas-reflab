pub mod aggregate;
pub mod ci_file;
pub mod image;
pub mod instance;
pub mod network;
pub mod network_assignment_map;
pub mod provision_log_event;
pub mod template;
pub mod types;

pub use aggregate::{Aggregate, AggregateConfiguration, BookingMetadata, LifeCycleState};
pub use ci_file::Cifile;
pub use image::Image;
pub use instance::Instance;
pub use network::{import_net, Network, NetworkBlob};
pub use network_assignment_map::NetworkAssignmentMap;
pub use provision_log_event::ProvisionLogEvent;
pub use template::Template;
pub use types::*;

// #[derive(Debug, Display, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
// pub enum InfraType {
// Switch,
// Server,
// }

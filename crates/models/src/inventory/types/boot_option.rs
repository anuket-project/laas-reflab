use serde::{Deserialize, Serialize};

use strum_macros::Display;

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, Display)]
pub enum BootTo {
    #[strum(serialize = "Network")]
    Network,
    #[strum(serialize = "Disk")]
    #[default]
    Disk,
    #[strum(serialize = "Specific Disk")]
    SpecificDisk,
}

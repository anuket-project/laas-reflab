use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Copy, EnumString, Display)]
pub enum Arch {
    #[strum(serialize = "x86")]
    X86,
    #[strum(serialize = "x86_64")]
    X86_64,
    #[strum(serialize = "aarch64")]
    Aarch64,
}

impl Arch {
    pub fn from_string_fuzzy(s: &str) -> Option<Arch> {
        if s.contains("x86_64") {
            Some(Arch::X86_64)
        } else if s.contains("x86") {
            Some(Arch::X86)
        } else if s.contains("aarch64") {
            Some(Arch::Aarch64)
        } else {
            None
        }
    }
}

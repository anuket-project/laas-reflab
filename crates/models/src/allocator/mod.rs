use std::sync::Mutex;

pub mod allocation;
pub mod resource_handle;
pub mod types;
pub mod vpn_token;

pub use allocation::{Allocation, AllocationOperation, AllocationReason, AllocationStatus};
pub use resource_handle::{ResourceHandle, ResourceHandleInner};
pub use types::{ResourceClass, ResourceRequestInner};
pub use vpn_token::VPNToken;

/// This struct is intentionally not constructable outside this module,
/// it provides one (and only one!) AT to the blessed allocator
pub struct AllocatorToken {
    #[allow(dead_code)]
    private: (),
}

lazy_static::lazy_static! {
    static ref TOKEN: Mutex<Option<AllocatorToken>> = Mutex::new(Some(AllocatorToken { private: () }));
}

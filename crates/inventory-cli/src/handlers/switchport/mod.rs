mod create;
mod delete;
mod fetch;
mod update;

pub use create::create_switchport;
pub use delete::delete_switchport;
pub use fetch::fetch_switchport_map;
#[allow(unused_imports)]
pub use update::update_switchport;

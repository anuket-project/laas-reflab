pub mod create;
pub mod delete;
pub mod fetch;
pub mod update;

pub use create::create_flavor;
pub use delete::delete_flavor_by_name;
pub use fetch::{fetch_flavor_map, fetch_flavor_name_by_id};
pub use update::update_flavor;

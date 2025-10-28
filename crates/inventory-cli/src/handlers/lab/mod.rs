pub mod create;
pub mod delete;
pub mod fetch;
pub mod update;

pub use create::create_lab;
pub use delete::delete_lab_by_name;
pub use fetch::fetch_lab_map;
pub use update::update_lab;

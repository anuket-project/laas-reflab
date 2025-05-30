use sqlx::PgPool;

mod flavor;
mod host;
mod hostport;
mod switchport;

pub(crate) use flavor::fetch_flavor_name;
pub(crate) use host::{delete_host_by_name, fetch_host_by_name, fetch_host_map};
pub(crate) use hostport::fetch_hostport_map;
pub(crate) use switchport::fetch_switchport_uuid_from_switchport_names;

use crate::error::InventoryError;

pub(crate) async fn get_db_pool() -> Result<PgPool, InventoryError> {
    let url = std::env::var("DATABASE_URL")?;
    PgPool::connect(&url)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: "While attempting to connect to database".to_string(),
            source: e,
        })
}

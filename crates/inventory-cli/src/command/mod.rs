use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::prelude::{
    HostInfo, InventoryError, MultipleErrors, Report, fetch_flavor_name, fetch_host_map,
    fetch_hostport_map, get_db_pool, load_inventory_hosts,
};
use futures::future::join_all;
use sqlx::PgPool;

mod import;
mod validate;

pub use import::import_inventory;
pub use validate::validate_inventory;

pub async fn generate_reports(dir: &str) -> Result<Vec<Report>, InventoryError> {
    // TODO: tracing

    // parse all hosts on disk in inventory directory
    let path = Path::new(dir);
    let yamls = load_inventory_hosts(path)
        .map_err(|errs| InventoryError::InventoryErrors(MultipleErrors(errs)))?;

    let pool: PgPool = get_db_pool().await?;

    let host_map = fetch_host_map(&pool).await?;
    let port_map = fetch_hostport_map(&pool).await?;

    // fetch flavor names for all hosts in the DB
    let flavor_futs = host_map.iter().map(|(server_name, host)| {
        let pool = &pool;
        let flavor_id = host.flavor.into_id().into_uuid();
        async move {
            fetch_flavor_name(pool, &flavor_id)
                .await
                .map(|flavor_name| (server_name.clone(), flavor_name))
        }
    });
    let flavor_pairs = join_all(flavor_futs)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    // HashMap<String, String> of server_name → flavor_name
    let flavor_map: HashMap<_, _> = flavor_pairs.into_iter().collect();

    // walk through yamls
    let mut reports = Vec::with_capacity(yamls.len());
    // track which DB hosts that are seen, so they can be marked as Removed
    let mut seen_db_hosts = HashSet::new();

    for yaml in yamls {
        // fetch db side by server_name
        let host_info: Option<HostInfo> = host_map.get(&yaml.server_name).map(|db_host| {
            seen_db_hosts.insert(db_host.server_name.clone());
            let ports = port_map
                .get(&db_host.server_name)
                .cloned()
                .unwrap_or_default();
            let flavor_name = flavor_map
                .get(&db_host.server_name)
                .expect("flavor must exist")
                .clone();
            (db_host.clone(), ports, flavor_name)
        });

        // diff and collect the Report
        let report = yaml.report_diff(host_info)?;
        reports.push(report);
    }

    // anything in DB we never saw in yaml inventory is now removed
    for server_name in host_map.keys() {
        if !seen_db_hosts.contains(server_name) {
            reports.push(Report::new_removed(server_name.clone()));
        }
    }

    reports.sort_by_key(Report::report_sort_order);

    Ok(reports)
}

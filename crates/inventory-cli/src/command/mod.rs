use futures::future::join_all;
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::prelude::{
    HostReport, InterfaceReport, InventoryError, MultipleErrors, Report, Reportable, Switch,
    SwitchPort, SwitchReport, flavor, get_db_pool, host, hostport, load_inventory, switch,
    switchport,
};

mod import;
mod validate;

pub use import::import_inventory;
pub use validate::validate_inventory;

/// Load YAML inventory, fetch DB state, diff, and return sorted reports.
pub async fn generate_reports(dir: &str) -> Result<Vec<Report>, InventoryError> {
    let path = Path::new(dir);
    let inventory = load_inventory(path)
        .map_err(|errs| InventoryError::InventoryErrors(MultipleErrors(errs)))?;
    println!("Loaded inventory from: {}", path.display());
    println!(
        "Found {} switches, {} hosts.",
        inventory.switches.len(),
        inventory.hosts.len()
    );

    let pool: PgPool = get_db_pool().await?;
    println!("Fetching current DB state...");

    let switch_map = switch::fetch_switch_map(&pool).await?;
    let switchport_map = switchport::fetch_switchport_map(&pool).await?;
    let host_map = host::fetch_host_map(&pool).await?;
    let port_map = hostport::fetch_hostport_map(&pool).await?;

    println!(
        "Fetched {} switches, {} hosts, {} ports.",
        switch_map.len(),
        host_map.len(),
        port_map.values().map(Vec::len).sum::<usize>()
    );
    println!("Generating diff reports...");

    let mut reports: Vec<Report> = Vec::new();
    let mut seen_switches = HashSet::new();

    for yaml in inventory.switches {
        // build the info type for this yaml switch
        let db_info: Option<(Switch, Vec<SwitchPort>)> =
            switch_map.get(&yaml.name).cloned().map(|sw| {
                let ports = switchport_map.get(&yaml.name).cloned().unwrap_or_default();
                seen_switches.insert(sw.name.clone());
                (sw, ports)
            });

        let sw_report = yaml.generate_switch_report(db_info)?;
        reports.push(Report::SwitchReport(sw_report));
    }

    // removed switches
    for (name, sw) in switch_map.iter() {
        if !seen_switches.contains(name) {
            reports.push(Report::SwitchReport(SwitchReport::new_removed(
                sw.clone(),
                switchport_map.get(name).unwrap_or(&Vec::new()).clone(),
            )));
        }
    }

    // preload flavor names
    let flavor_map: HashMap<String, String> = join_all(host_map.iter().map(|(srv, host)| {
        let pool = pool.clone();
        let srv = srv.clone();
        let f_id = host.flavor.into_id().into_uuid();
        async move {
            flavor::fetch_flavor_name(&pool, &f_id)
                .await
                .map(|name| (srv, name))
        }
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .collect();

    // host reports
    let mut seen_hosts = HashSet::new();

    for yaml in inventory.hosts {
        let host_info = host_map.get(&yaml.server_name).map(|db_host| {
            seen_hosts.insert(db_host.server_name.clone());
            let ports = port_map
                .get(&db_host.server_name)
                .cloned()
                .unwrap_or_default();
            let flavor = flavor_map.get(&db_host.server_name).cloned().unwrap();
            (db_host.clone(), ports, flavor)
        });

        // only pass host_info and switchport_tuples to match signature
        let h_report = yaml
            .generate_host_report(host_info, &switchport_map)
            .await?;
        reports.push(Report::HostReport(h_report));
    }

    // removed hosts
    for (name, db_host) in host_map.iter() {
        if !seen_hosts.contains(name) {
            let iface_reports: Vec<InterfaceReport> = port_map
                .get(name)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|p| InterfaceReport::Removed {
                    server_name: name.clone(),
                    db_interface: p.clone(),
                })
                .collect();
            reports.push(Report::HostReport(HostReport::new_removed(
                db_host.clone(),
                iface_reports,
            )));
        }
    }

    // sort and return
    reports.sort_by_key(|r| r.sort_order());

    Ok(reports)
}

pub fn print_reports(reports: &[Report]) {
    for report in reports {
        print!("{}", report);
    }
}

use colored::Colorize;
use futures::future::join_all;
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::prelude::{
    HostReport, InterfaceReport, InventoryError, MultipleErrors, Report, Reportable, Switch,
    SwitchPort, SwitchReport, flavor, get_db_pool, host, hostport, image, lab, load_inventory,
    switch, switchport,
};

mod import;
mod validate;

pub use import::import_inventory;
pub use validate::validate_inventory;

/// Load YAML inventory, fetch DB state, generate diffs, and return sorted reports.
///
/// # Arguments
///
/// * `dir` - Path to the directory containing inventory YAML files
/// * `verbose` - Debug output
///
/// # Returns
///
/// A vector of [`Report`] objects describing all changes, sorted by type and priority
///
/// # Errors
///
/// Returns an error if:
/// - YAML files cannot be loaded or parsed
/// - Database connection fails
/// - Database queries fail
/// - Invalid references are found (e.g., non-existent flavor)
pub async fn generate_reports(dir: &str, verbose: bool) -> Result<Vec<Report>, InventoryError> {
    let path = Path::new(dir);
    let inventory = load_inventory(path)
        .map_err(|errs| InventoryError::InventoryErrors(MultipleErrors(errs)))?;

    if verbose {
        println!(
            "Loaded inventory from: {}",
            path.display().to_string().yellow()
        );
        println!(
            "Found {} switches, {} hosts, {} images, {} flavors, {} labs.",
            inventory.switches.len(),
            inventory.hosts.len(),
            inventory.images.len(),
            inventory.flavors.len(),
            inventory.labs.len()
        );
    }

    let pool: PgPool = get_db_pool().await?;

    if verbose {
        println!("Fetching current DB state...");
    }

    let lab_map = lab::fetch_lab_map(&pool).await?;
    let flavor_map = flavor::fetch_flavor_map(&pool).await?;
    let image_map = image::fetch_image_map(&pool).await?;
    let kernel_args_map = image::fetch_kernel_args_map(&pool).await?;
    let switch_map = switch::fetch_switch_map(&pool).await?;
    let switchport_map = switchport::fetch_switchport_map(&pool).await?;
    let host_map = host::fetch_host_map(&pool).await?;
    let port_map = hostport::fetch_hostport_map(&pool).await?;

    if verbose {
        println!(
            "Fetched {} labs, {} flavors, {} images, {} switches, {} hosts, {} ports.",
            lab_map.len(),
            flavor_map.len(),
            image_map.len(),
            switch_map.len(),
            host_map.len(),
            port_map.values().map(Vec::len).sum::<usize>()
        );
        println!("Generating diff reports...");
    }

    let mut reports: Vec<Report> = Vec::new();

    // created, modified, unchanged labs
    let mut seen_labs = HashSet::new();
    for yaml in inventory.labs {
        let db_lab = lab_map.get(&yaml.name).cloned();
        if let Some(ref l) = db_lab {
            seen_labs.insert(l.name.clone());
        }
        let report = yaml.generate_lab_report(db_lab)?;
        reports.push(Report::LabReport(report));
    }

    // removed labs
    for (name, db_lab) in lab_map.iter() {
        if !seen_labs.contains(name) {
            reports.push(Report::LabReport(
                crate::report::LabReport::new_removed(db_lab.clone()),
            ));
        }
    }

    // created, modified, unchanged flavors
    let mut seen_flavors = HashSet::new();
    for yaml in inventory.flavors {
        let db_flavor = flavor_map.get(&yaml.name).cloned();
        if let Some(ref f) = db_flavor {
            seen_flavors.insert(f.name.clone());
        }
        let report = yaml.generate_flavor_report(db_flavor)?;
        reports.push(Report::FlavorReport(report));
    }

    // removed flavors
    for (name, db_flavor) in flavor_map.iter() {
        if !seen_flavors.contains(name) {
            reports.push(Report::FlavorReport(
                crate::report::FlavorReport::new_removed(db_flavor.clone()),
            ));
        }
    }

    // created, modified, removed images
    let mut seen_images = HashSet::new();
    for yaml in inventory.images {
        let db_image = image_map.get(&yaml.name).cloned();
        let db_kernel_args = kernel_args_map.get(&yaml.name).cloned();
        if let Some(ref img) = db_image {
            seen_images.insert(img.name.clone());
        }
        let report = yaml
            .generate_image_report(db_image, db_kernel_args, &flavor_map)
            .await?;
        reports.push(Report::ImageReport(report));
    }

    // removed images
    for (name, db_image) in image_map.iter() {
        if !seen_images.contains(name) {
            // NOTE: CASCADE will delete all kernel_args
            let kernel_arg_reports = vec![];
            reports.push(Report::ImageReport(
                crate::report::ImageReport::new_removed(db_image.clone(), kernel_arg_reports),
            ));
        }
    }

    let mut seen_switches = HashSet::new();

    // created, modified, unchanged switches
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
            flavor::fetch_flavor_name_by_id(&pool, &f_id)
                .await
                .map(|name| (srv, name))
        }
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .collect();

    // created, modified, unchanged hosts
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

    reports.sort_by_key(|r| r.sort_order());

    Ok(reports)
}

/// Print a formatted summary of all inventory changes
pub fn print_reports(reports: &[Report]) {
    let (created, modified, removed, unchanged) =
        reports.iter().fold((0, 0, 0, 0), |(c, m, r, u), report| {
            if report.is_created() {
                (c + 1, m, r, u)
            } else if report.is_modified() {
                (c, m + 1, r, u)
            } else if report.is_removed() {
                (c, m, r + 1, u)
            } else {
                (c, m, r, u + 1)
            }
        });

    let total_changes = created + modified + removed;

    println!("\n{}", "Inventory Diff Summary".bold());
    println!("{}", "-".repeat(40));

    if total_changes == 0 {
        println!("{}", "No changes detected".dimmed());
        if unchanged > 0 {
            println!("{} items unchanged", unchanged);
        }
        println!();
        return;
    }

    if created > 0 {
        println!(
            "  {} {}",
            "+".green().bold(),
            format!("{} to create", created).green()
        );
    }
    if modified > 0 {
        println!(
            "  {} {}",
            "~".yellow().bold(),
            format!("{} to modify", modified).yellow()
        );
    }
    if removed > 0 {
        println!(
            "  {} {}",
            "-".red().bold(),
            format!("{} to remove", removed).red()
        );
    }
    if unchanged > 0 {
        println!("  {}", format!("{} unchanged", unchanged).dimmed());
    }
    println!();

    let (labs, flavors, images, switches, hosts): (Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>) =
        reports.iter().fold(
            (vec![], vec![], vec![], vec![], vec![]),
            |(mut l, mut f, mut i, mut s, mut h), report| {
                match report {
                    Report::LabReport(r) => l.push(r),
                    Report::FlavorReport(r) => f.push(r),
                    Report::ImageReport(r) => i.push(r),
                    Report::SwitchReport(r) => s.push(r),
                    Report::HostReport(r) => h.push(r),
                }
                (l, f, i, s, h)
            },
        );

    if labs.iter().any(|r| !r.is_unchanged()) {
        print_section_header("LABS");
        for lab in labs {
            if !lab.is_unchanged() {
                println!("{}", lab);
            }
        }
    }

    if flavors.iter().any(|r| !r.is_unchanged()) {
        print_section_header("FLAVORS");
        for flavor in flavors {
            if !flavor.is_unchanged() {
                println!("{}", flavor);
            }
        }
    }

    if images.iter().any(|r| !r.is_unchanged()) {
        print_section_header("IMAGES");
        for image in images {
            if !image.is_unchanged() {
                println!("{}", image);
            }
        }
    }

    if switches.iter().any(|r| !r.is_unchanged()) {
        print_section_header("SWITCHES");
        for switch in switches {
            if !switch.is_unchanged() {
                println!("{}", switch);
            }
        }
    }

    if hosts.iter().any(|r| !r.is_unchanged()) {
        print_section_header("HOSTS");
        for host in hosts {
            if !host.is_unchanged() {
                println!("{}", host);
            }
        }
    }
}

fn print_section_header(title: &str) {
    use colored::Colorize;
    println!("{}", title.cyan().bold());
    println!("{}", "─".repeat(title.len()).cyan());
}

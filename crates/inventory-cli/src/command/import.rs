use colored::Colorize;
use std::io::{self, Write};
use std::{collections::HashMap, path::Path};

use crate::prelude::{
    InventoryError, MultipleErrors, Report, confirm_and_proceed, delete_host_by_name,
    fetch_host_by_name, generate_reports, get_db_pool, load_inventory_hosts, print_reports,
};
use sqlx::PgPool;

pub async fn import_inventory(
    dir: &str,
    auto_yes: bool,
    detailed: bool,
    ignore_errors: bool,
) -> Result<(), InventoryError> {
    // TODO: refactor to avoid fetching two yaml inventories (one for generate reports and one for
    // comparing server_names and running CRUD fn's)
    let path = Path::new(dir);
    let yamls = load_inventory_hosts(path)
        .map_err(|errs| InventoryError::InventoryErrors(MultipleErrors(errs)))?;
    let yaml_map: HashMap<_, _> = yamls
        .into_iter()
        .map(|y| (y.server_name.clone(), y))
        .collect();

    let reports = generate_reports(dir).await?;

    let pool: PgPool = get_db_pool().await?;

    let summary = print_reports(&reports, detailed);

    if confirm_and_proceed(summary, auto_yes) {
        for report in reports {
            let keep_going = match report {
                Report::Created { server_name, .. } => {
                    let yaml = &yaml_map[&server_name];
                    println!("Creating host {}...", server_name);
                    match yaml.create_host_record(&pool).await {
                        Ok(_) => true,
                        Err(e) => handle_import_error(e, ignore_errors),
                    }
                }
                Report::Modified { server_name, .. } => {
                    let yaml = &yaml_map[&server_name];
                    let db_host = fetch_host_by_name(&pool, &server_name).await?;
                    println!("Updating host {}...", server_name);
                    match yaml.update_host_record(&db_host, &pool).await {
                        Ok(_) => true,
                        Err(e) => handle_import_error(e, ignore_errors),
                    }
                }
                Report::Removed { server_name, .. } => {
                    println!("Deleting host {}...", server_name);
                    match delete_host_by_name(&server_name, &pool).await {
                        Ok(_) => true,
                        Err(e) => handle_import_error(e, ignore_errors),
                    }
                }
                Report::Unchanged { .. } => true,
            };

            if !keep_going {
                return Err(InventoryError::Aborted);
            }
        }
    } else {
        return Err(InventoryError::Aborted);
    }

    println!("{}", "Inventory imported successfully.".green().bold());

    Ok(())
}

/// Returns `true` if we should continue to the next host,
/// or `false` if we should abort import entirely.
fn handle_import_error(err: InventoryError, ignore_errors: bool) -> bool {
    eprintln!("{}", format!("{}", err).red());

    if ignore_errors {
        return true;
    }

    print!("{}", "Continue with next host? [y/N]: ".cyan().bold());
    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

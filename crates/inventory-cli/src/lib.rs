//! # Inventory CLI
//!
//! A command-line interface for managing a Lab as a Service (LaaS) inventory.
//!
//! ## Usage
//!
//! ```bash
//! # Validate inventory without making changes
//! inventory-cli validate --path ./inventory
//!
//! # Import inventory into the database
//! inventory-cli import --path ./inventory
//! ```

mod command;
mod error;
mod handlers;
mod modified;
mod report;
mod schema;
mod utils;

pub mod prelude;

use crate::prelude::InventoryError;

use clap::{Parser, Subcommand};
use colored::Colorize;
use sqlx::PgPool;

#[derive(Parser)]
#[clap(name = "LaaS Inventory CLI", version = "0.1.0")]
pub struct Cli {
    #[clap(subcommand)]
    pub command: InventoryCommand,
}

#[derive(Subcommand, Debug)]
pub enum InventoryCommand {
    /// Check inventory against the database
    Validate {
        /// Path to inventory folder
        #[clap(short, long, default_value = ".")]
        path: String,
        /// Show debug information
        #[clap(short, long, default_value = "false")]
        verbose: bool,
    },
    /// Import inventory into the database
    Import {
        /// Path to inventory folder
        #[clap(short, long, default_value = ".")]
        path: String,
        /// Automatically confirm the import
        #[clap(short = 'y', long = "yes")]
        yes: bool,
        /// Show debug information
        #[clap(short, long, default_value = "false")]
        verbose: bool,
    },
}

/// Get a database connection pool from the DATABASE_URL environment variable
///
/// # Errors
/// - DATABASE_URL environment variable is not set
/// - Connection to the database fails
/// - There are pending database migrations
pub(crate) async fn get_db_pool() -> Result<PgPool, InventoryError> {
    let url = std::env::var("DATABASE_URL")?;
    let pool = PgPool::connect(&url)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: "While attempting to connect to database".to_string(),
            source: e,
        })?;

    // check for pending migrations
    check_migrations(&pool).await?;

    Ok(pool)
}

/// Checks if there are any pending migrations
async fn check_migrations(pool: &PgPool) -> Result<(), InventoryError> {
    let migrator = sqlx::migrate!("../../migrations");

    let migrations = migrator.migrations;
    let applied = sqlx::query!("SELECT version FROM _sqlx_migrations")
        .fetch_all(pool)
        .await;

    match applied {
        Ok(applied_migrations) => {
            let applied_versions: std::collections::HashSet<i64> =
                applied_migrations.iter().map(|m| m.version).collect();

            let pending: Vec<_> = migrations
                .iter()
                .filter(|m| !applied_versions.contains(&m.version))
                .collect();

            if !pending.is_empty() {
                return Err(InventoryError::PendingMigrations {
                    count: pending.len(),
                });
            }
        }
        Err(_) => {
            return Err(InventoryError::DatabaseNotInitialized);
        }
    }

    Ok(())
}

pub fn match_and_print(result: Result<(), InventoryError>) {
    match result {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            eprintln!(
                "{}{}",
                "Error encountered: ".red().bold(),
                e.to_string().red()
            );
            std::process::exit(1);
        }
    }
}

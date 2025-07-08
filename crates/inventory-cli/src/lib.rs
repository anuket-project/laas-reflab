mod command;
mod error;
mod handlers;
mod modified;
pub mod prelude;
mod report;
mod schema;
mod utils;

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
        /// Show detailed output (eg. Unchanged hosts)
        #[clap(short, long, default_value = "false")]
        detailed: bool,
    },
    /// Import inventory into the database
    Import {
        /// Path to inventory folder
        #[clap(short, long, default_value = ".")]
        path: String,
        /// Automatically confirm the import
        #[clap(short = 'y', long = "yes")]
        yes: bool,
        /// Show detailed output (eg. Unchanged hosts)
        #[clap(short, long, default_value = "false")]
        detailed: bool,
        /// Ignore import per host import errors  WARNING: This is dangerous!
        #[clap(short, long, default_value = "false")]
        ignore_errors: bool,
    },
}

pub(crate) async fn get_db_pool() -> Result<PgPool, InventoryError> {
    let url = std::env::var("DATABASE_URL")?;
    PgPool::connect(&url)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: "While attempting to connect to database".to_string(),
            source: e,
        })
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

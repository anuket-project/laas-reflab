mod command;
mod error;
mod fetch;
mod modified;
pub mod prelude;
mod report;
mod schema;
mod utils;

use crate::prelude::InventoryError;
use clap::{Parser, Subcommand};
use colored::Colorize;

#[derive(Parser)]
#[clap(name = "LaaS Inventory CLI", version = "0.1.0")]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
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

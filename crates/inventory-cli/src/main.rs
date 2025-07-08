use clap::Parser;
use inventory_cli::prelude::{import_inventory, validate_inventory};
use inventory_cli::{Cli, InventoryCommand, match_and_print};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        InventoryCommand::Validate { path, detailed } => {
            match_and_print(validate_inventory(&path, detailed).await)
        }
        InventoryCommand::Import {
            path,
            yes,
            detailed,
            ignore_errors,
        } => match_and_print(import_inventory(&path, yes, detailed, ignore_errors).await),
    }
}

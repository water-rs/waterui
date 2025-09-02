//! `WaterUI` CLI - Create and manage `WaterUI` projects

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod cargo_config;
mod commands;
mod config;
mod template;

#[derive(Parser)]
#[command(name = "waterui")]
#[command(author = "Water-rs Team")]
#[command(version = "0.1.0")]
#[command(about = "WaterUI CLI - Create and manage WaterUI projects", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new `WaterUI` project
    New {
        /// Name of the project
        name: String,
        
        /// Path where to create the project (defaults to current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    
    /// Initialize a `WaterUI` project in the current directory
    Init,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::New { name, path } => {
            commands::new::execute(&name, path)?;
        }
        Commands::Init => {
            commands::init::execute()?;
        }
    }
    
    Ok(())
}
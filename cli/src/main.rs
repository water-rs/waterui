use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "water")]
#[command(about = "CLI of WaterUI", long_about = None)]
#[command(version, author)]
struct Cli {
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    New {
        // Setting up all backends by default
    },
    Run {
        // Hot reload enabled by default
    },
    Build {},
    I18n {},

    Plugin {},

    Backend {},
}

fn main() {
    let _cli = Cli::parse();
}

pub fn add_swiftui_backend() {
    println!("Adding SwiftUI backend...");
}

pub fn add_android_backend() {
    println!("Adding Android backend...");
}

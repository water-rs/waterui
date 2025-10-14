mod apple;
mod android;
mod clean;
mod config;
mod create;
mod devices;
mod doctor;
mod package;
mod run;
mod util;

use clap::{Parser, Subcommand};
use color_eyre::{config::HookBuilder, eyre::Result};
use tracing_subscriber::{FmtSubscriber, filter::LevelFilter, fmt::format::FmtSpan};

//pub const WATERUI_VERSION: &str = env!("WATERUI_VERSION");
//pub const WATERUI_SWIFT_BACKEND_VERSION: &str = env!("WATERUI_SWIFT_BACKEND_VERSION");

pub const WATERUI_VERSION: &str = "";
pub const WATERUI_SWIFT_BACKEND_VERSION: &str = "";

#[derive(Parser)]
#[command(name = "water")]
#[command(about = "CLI of WaterUI", long_about = None)]
#[command(version, author)]
struct Cli {
    /// Increase output verbosity (-v, -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactively create a new WaterUI application project
    Create(create::CreateArgs),
    /// Build and run a WaterUI application with SwiftUI hot reload support
    Run(run::RunArgs),
    /// Check for potential problems with the development environment
    Doctor(doctor::DoctorArgs),
    /// Remove build artifacts and platform caches
    Clean(clean::CleanArgs),
    /// List available simulators and connected devices
    Devices(devices::DevicesArgs),
    /// Build distributable artifacts without launching them
    Package(package::PackageArgs),
}

fn main() {
    if let Err(err) = run_cli() {
        util::print_error(err, None);
    }
}

fn run_cli() -> Result<()> {
    let cli = Cli::parse();

    HookBuilder::default()
        .display_env_section(false)
        .issue_url("https://github.com/water-rs/waterui/issues/new")
        .panic_section("It looks like WaterUI CLI encountered a bug")
        .install()?;

    let level = match cli.verbose {
        0 => LevelFilter::INFO,
        1 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_span_events(FmtSpan::NONE)
        .without_time()
        .with_target(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    match cli.command {
        Commands::Create(args) => create::run(args),
        Commands::Run(args) => run::run(args),
        Commands::Doctor(args) => doctor::run(args),
        Commands::Clean(args) => clean::run(args),
        Commands::Devices(args) => devices::run(args),
        Commands::Package(args) => package::run(args),
    }
}

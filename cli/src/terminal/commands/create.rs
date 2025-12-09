//! `water create` command implementation.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use color_eyre::eyre::Result;
use dialoguer::{Input, MultiSelect, theme::ColorfulTheme};
use heck::ToKebabCase;

use crate::shell;
use crate::{header, line, success};
use waterui_cli::project::{CreateOptions, Project};

/// Arguments for the create command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Project display name (e.g., "Water Example" creates folder "water-example").
    name: Option<String>,

    /// Bundle identifier (defaults to com.example.<name>).
    #[arg(long)]
    bundle_id: Option<String>,

    /// Platforms to scaffold (ios, android, macos).
    #[arg(long, value_delimiter = ',')]
    platform: Option<Vec<String>>,

    /// Path to local `WaterUI` repository (for development).
    #[arg(long, conflicts_with = "dev")]
    waterui_path: Option<PathBuf>,

    /// Use current directory as `WaterUI` repository path (shorthand for --waterui-path .).
    #[arg(long, conflicts_with = "waterui_path")]
    dev: bool,

    /// Create a playground project (auto-managed backends, no manual backend files).
    #[arg(long)]
    playground: bool,
}

/// Platform options for scaffolding.
#[derive(Debug, Clone, Copy)]
enum Platform {
    Ios,
    Android,
    MacOs,
}

impl Platform {
    const ALL: [Self; 3] = [Self::Ios, Self::Android, Self::MacOs];

    const fn label(self) -> &'static str {
        match self {
            Self::Ios => "iOS",
            Self::Android => "Android",
            Self::MacOs => "macOS",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ios" => Some(Self::Ios),
            "android" => Some(Self::Android),
            "macos" => Some(Self::MacOs),
            _ => None,
        }
    }
}

/// Run the create command.
pub async fn run(args: Args) -> Result<()> {
    let interactive = shell::is_interactive();

    // Gather config - use CLI args if provided, otherwise prompt
    let name = match args.name.clone() {
        Some(n) => n,
        None if interactive => prompt_name()?,
        None => return Err(color_eyre::eyre::eyre!("Project name is required")),
    };

    // Resolve waterui_path (--dev uses current directory)
    let waterui_path = if args.dev {
        Some(std::env::current_dir()?)
    } else {
        args.waterui_path.clone()
    };

    let bundle_id = match args.bundle_id.clone() {
        Some(id) => id,
        None if interactive => prompt_bundle_id(&name)?,
        None => default_bundle_id(&name),
    };

    let platforms = match &args.platform {
        Some(plats) => parse_platforms(plats),
        None if interactive => prompt_platforms()?,
        None => vec![Platform::Ios, Platform::Android],
    };

    // Compute project path
    let folder_name = name.to_kebab_case();
    let project_path = std::env::current_dir()?.join(&folder_name);

    header!("Creating WaterUI project: {}", name);

    // Create project using library API
    let spinner = shell::spinner("Creating project files...");
    let mut project = Project::create(
        &project_path,
        CreateOptions {
            name: name.clone(),
            bundle_identifier: bundle_id,
            playground: args.playground,
            waterui_path,
            author: whoami::username(),
        },
    )
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Created Cargo.toml and src/lib.rs");

    // Initialize backends (skip for playground projects)
    if !args.playground {
        let has_apple = platforms.iter().any(|p| matches!(p, Platform::Ios | Platform::MacOs));
        let has_android = platforms.iter().any(|p| matches!(p, Platform::Android));

        if has_apple {
            let spinner = shell::spinner("Scaffolding Apple backend...");
            project.init_apple_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created Apple backend in apple/");
        }

        if has_android {
            let spinner = shell::spinner("Scaffolding Android backend...");
            project.init_android_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created Android backend in android/");
        }
    }

    // Final message
    line!();
    success!("Project created at {}", project_path.display());
    line!();
    line!("Next steps:");
    line!("  cd {folder_name}");
    line!("  water run --platform ios");

    Ok(())
}

fn prompt_name() -> Result<String> {
    Ok(Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Project name")
        .interact_text()?)
}

fn default_bundle_id(app_name: &str) -> String {
    format!("com.example.{}", app_name.to_lowercase().replace(' ', ""))
}

fn prompt_bundle_id(app_name: &str) -> Result<String> {
    let default = default_bundle_id(app_name);
    Ok(Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Bundle identifier")
        .default(default)
        .interact_text()?)
}

fn parse_platforms(plats: &[String]) -> Vec<Platform> {
    plats.iter().filter_map(|s| Platform::from_str(s)).collect()
}

fn prompt_platforms() -> Result<Vec<Platform>> {
    let items: Vec<&str> = Platform::ALL.iter().map(|p| p.label()).collect();
    let defaults = vec![true, true, false]; // iOS and Android selected by default

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select platforms")
        .items(&items)
        .defaults(&defaults)
        .interact()?;

    Ok(selections.into_iter().map(|i| Platform::ALL[i]).collect())
}

//! `water create` command implementation.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use color_eyre::eyre::Result;
use dialoguer::{Input, MultiSelect, theme::ColorfulTheme};
use heck::{ToKebabCase, ToSnakeCase};

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

    /// Backends to scaffold (apple, android, gtk).
    #[arg(long, value_delimiter = ',')]
    backends: Option<Vec<String>>,

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

/// Backend options for scaffolding.
#[derive(Debug, Clone, Copy)]
enum Backend {
    Apple,
    Android,
    Gtk,
}

impl Backend {
    const ALL: [Self; 3] = [Self::Apple, Self::Android, Self::Gtk];

    const fn label(self) -> &'static str {
        match self {
            Self::Apple => "Apple (iOS/macOS)",
            Self::Android => "Android",
            Self::Gtk => "GTK (Linux/macOS/Windows)",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "apple" | "ios" | "macos" => Some(Self::Apple),
            "android" => Some(Self::Android),
            "gtk" | "gtk4" | "linux" => Some(Self::Gtk),
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

    // Resolve waterui_path (--dev prompts for local path)
    let waterui_path = if args.dev {
        let user_input = if interactive {
            prompt_waterui_path()?
        } else {
            ".".to_string()
        };

        // Convert user input to a path relative to the new project directory
        // If user inputs ".", it becomes "../" in the new project
        let input_path = std::path::Path::new(&user_input);
        let relative_to_new_project = if input_path.is_relative() {
            // For relative paths, prepend "../" since we're going one level deeper
            std::path::PathBuf::from("..").join(input_path)
        } else {
            // For absolute paths, use as-is
            input_path.to_path_buf()
        };

        Some(relative_to_new_project)
    } else {
        args.waterui_path.clone()
    };

    let bundle_id = match args.bundle_id.clone() {
        Some(id) => id,
        None if interactive => prompt_bundle_id(&name)?,
        None => default_bundle_id(&name),
    };

    let backends = match &args.backends {
        Some(b) => parse_backends(b),
        None if interactive => prompt_backends()?,
        None => vec![Backend::Apple, Backend::Android],
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
        let has_apple = backends.iter().any(|b| matches!(b, Backend::Apple));
        let has_android = backends.iter().any(|b| matches!(b, Backend::Android));
        let has_gtk = backends.iter().any(|b| matches!(b, Backend::Gtk));

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

        if has_gtk {
            let spinner = shell::spinner("Scaffolding GTK backend...");
            project.init_gtk_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created GTK backend in gtk/");
        }
    }

    // Final message
    line!();
    success!("Project created at {}", project_path.display());
    line!();
    line!("Next steps:");
    line!("  cd {folder_name}");
    if backends.iter().any(|b| matches!(b, Backend::Apple)) {
        line!("  water run --platform ios");
    } else if backends.iter().any(|b| matches!(b, Backend::Android)) {
        line!("  water run --platform android");
    } else if backends.iter().any(|b| matches!(b, Backend::Gtk)) {
        line!("  water run --platform gtk");
    }

    Ok(())
}

fn prompt_name() -> Result<String> {
    Ok(Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Project name")
        .interact_text()?)
}

fn prompt_waterui_path() -> Result<String> {
    Ok(Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Local WaterUI path")
        .default(".".to_string())
        .interact_text()?)
}

fn default_bundle_id(app_name: &str) -> String {
    format!("com.example.{}", app_name.to_snake_case())
}

fn prompt_bundle_id(app_name: &str) -> Result<String> {
    let default = default_bundle_id(app_name);
    Ok(Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Bundle identifier")
        .default(default)
        .interact_text()?)
}

fn parse_backends(backends: &[String]) -> Vec<Backend> {
    backends
        .iter()
        .filter_map(|s| Backend::from_str(s))
        .collect()
}

fn prompt_backends() -> Result<Vec<Backend>> {
    let items: Vec<&str> = Backend::ALL.iter().map(|b| b.label()).collect();
    let defaults = vec![true, true, false]; // Apple and Android selected by default

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select backends")
        .items(&items)
        .defaults(&defaults)
        .interact()?;

    Ok(selections.into_iter().map(|i| Backend::ALL[i]).collect())
}

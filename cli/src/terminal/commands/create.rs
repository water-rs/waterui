//! `water create` command implementation.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use color_eyre::eyre::{Result, bail};
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

    /// Backends to scaffold (apple, android, gtk4).
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
    Gtk4,
}

impl Backend {
    const ALL: [Self; 3] = [Self::Apple, Self::Android, Self::Gtk4];

    const fn label(self) -> &'static str {
        match self {
            Self::Apple => "Apple (iOS/macOS)",
            Self::Android => "Android",
            Self::Gtk4 => "GTK4 (Linux/macOS/Windows)",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "apple" | "ios" | "macos" => Some(Self::Apple),
            "android" => Some(Self::Android),
            "gtk" | "gtk4" | "linux" => Some(Self::Gtk4),
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
        Some(b) => parse_backends(b)?,
        None if interactive => prompt_backends()?,
        None => vec![Backend::Apple, Backend::Android],
    };

    if !args.playground && backends.is_empty() {
        bail!(
            "At least one backend is required. Choose from: apple, android, gtk4 (or use --playground)."
        );
    }

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
    success!("Created Cargo.toml, src/lib.rs, and assets/raw + assets/images");

    // Initialize backends (skip for playground projects)
    if !args.playground {
        let has_apple = backends.iter().any(|b| matches!(b, Backend::Apple));
        let has_android = backends.iter().any(|b| matches!(b, Backend::Android));
        let has_gtk4 = backends.iter().any(|b| matches!(b, Backend::Gtk4));

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

        if has_gtk4 {
            let spinner = shell::spinner("Scaffolding GTK4 backend...");
            project.init_gtk4_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created GTK4 backend in gtk4/");
        }
    }

    // Final message
    line!();
    success!("Project created at {}", project_path.display());
    line!();
    line!("Next steps:");
    line!("  cd {folder_name}");
    if let Some(command) = next_run_command(&backends) {
        line!("  {command}");
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

fn parse_backends(backends: &[String]) -> Result<Vec<Backend>> {
    let mut parsed = Vec::with_capacity(backends.len());
    let mut invalid = Vec::new();

    for backend in backends {
        if let Some(parsed_backend) = Backend::from_str(backend) {
            parsed.push(parsed_backend);
        } else {
            invalid.push(backend.clone());
        }
    }

    if invalid.is_empty() {
        Ok(parsed)
    } else {
        bail!(
            "Unknown backend(s): {}. Valid values: apple, android, gtk4",
            invalid.join(", ")
        )
    }
}

fn next_run_command(backends: &[Backend]) -> Option<&'static str> {
    if backends.iter().any(|b| matches!(b, Backend::Apple)) {
        return Some("water run --platform ios");
    }

    if backends.iter().any(|b| matches!(b, Backend::Android)) {
        return Some("water run --platform android");
    }

    if backends.iter().any(|b| matches!(b, Backend::Gtk4)) {
        #[cfg(target_os = "macos")]
        return Some("water run --platform macos --backend gtk4");

        #[cfg(target_os = "linux")]
        return Some("water run --platform linux");

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return None;
    }

    None
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

#[cfg(test)]
mod tests {
    use super::{Backend, next_run_command, parse_backends};

    #[test]
    fn parse_backends_rejects_unknown_values() {
        let err = parse_backends(&["apple".to_string(), "androd".to_string()])
            .expect_err("invalid backend should fail");
        let msg = err.to_string();
        assert!(msg.contains("Unknown backend(s): androd"));
        assert!(msg.contains("apple, android, gtk4"));
    }

    #[test]
    fn parse_backends_accepts_aliases() {
        let parsed = parse_backends(&[
            "ios".to_string(),
            "android".to_string(),
            "linux".to_string(),
        ])
        .expect("known aliases should parse");
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn next_run_command_prefers_apple_then_android_then_gtk4() {
        assert_eq!(
            next_run_command(&[Backend::Apple, Backend::Gtk4]),
            Some("water run --platform ios")
        );
        assert_eq!(
            next_run_command(&[Backend::Android, Backend::Gtk4]),
            Some("water run --platform android")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn next_run_command_gtk4_is_valid_on_macos() {
        assert_eq!(
            next_run_command(&[Backend::Gtk4]),
            Some("water run --platform macos --backend gtk4")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn next_run_command_gtk4_is_valid_on_linux() {
        assert_eq!(
            next_run_command(&[Backend::Gtk4]),
            Some("water run --platform linux")
        );
    }
}

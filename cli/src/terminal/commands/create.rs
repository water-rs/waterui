//! `water create` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use dialoguer::{Input, MultiSelect, theme::ColorfulTheme};
use heck::{ToKebabCase, ToSnakeCase};

use crate::shell;
use crate::{header, line, success};
use waterui_cli::project::{CreateOptions, PackageType, Project};

/// Arguments for the create command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Project display name (e.g., "Water Example" creates folder "water-example").
    name: Option<String>,

    /// Bundle identifier (defaults to com.example.<name>).
    #[arg(long)]
    bundle_id: Option<String>,

    /// Backends to scaffold (apple, android, gtk4, hydrolysis).
    #[arg(long, value_delimiter = ',')]
    backends: Option<Vec<String>>,

    /// Path to local `WaterUI` repository (for development).
    #[arg(long, conflicts_with = "dev")]
    waterui_path: Option<PathBuf>,

    /// Use current directory as `WaterUI` repository path (shorthand for --waterui-path .).
    #[arg(long, conflicts_with = "waterui_path")]
    dev: bool,

    /// Project mode (`app` or `playground`).
    #[arg(long, value_enum, default_value_t = ProjectMode::App)]
    mode: ProjectMode,
}

/// Backend options for scaffolding.
#[derive(Debug, Clone, Copy)]
enum Backend {
    Apple,
    Android,
    Gtk4,
    Hydrolysis,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum ProjectMode {
    #[default]
    App,
    Playground,
}

impl ProjectMode {
    const fn package_type(self) -> PackageType {
        match self {
            Self::App => PackageType::App,
            Self::Playground => PackageType::Playground,
        }
    }
}

impl Backend {
    const ALL: [Self; 4] = [Self::Apple, Self::Android, Self::Gtk4, Self::Hydrolysis];

    const fn label(self) -> &'static str {
        match self {
            Self::Apple => "Apple (iOS/macOS)",
            Self::Android => "Android",
            Self::Gtk4 => "GTK4 (Linux/macOS/Windows)",
            Self::Hydrolysis => "Hydrolysis (Linux/macOS)",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "apple" | "ios" | "macos" => Some(Self::Apple),
            "android" => Some(Self::Android),
            "gtk" | "gtk4" | "linux" => Some(Self::Gtk4),
            "hydrolysis" => Some(Self::Hydrolysis),
            _ => None,
        }
    }
}

/// Run the create command.
pub async fn run(args: Args) -> Result<()> {
    let interactive = shell::is_interactive();
    let package_type = args.mode.package_type();

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

    if package_type == PackageType::Playground && args.backends.is_some() {
        bail!("Playground mode does not support --backends; backend projects are auto-managed.");
    }

    let backends = if package_type == PackageType::Playground {
        Vec::new()
    } else {
        match &args.backends {
            Some(values) => parse_backends(values)?,
            None if interactive => prompt_backends()?,
            None => vec![Backend::Apple, Backend::Android],
        }
    };

    if package_type == PackageType::App && backends.is_empty() {
        bail!("At least one backend is required. Choose from: apple, android, gtk4.");
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
            package_type,
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
    if package_type == PackageType::App {
        let has_apple = backends.iter().any(|b| matches!(b, Backend::Apple));
        let has_android = backends.iter().any(|b| matches!(b, Backend::Android));
        let has_gtk4 = backends.iter().any(|b| matches!(b, Backend::Gtk4));
        let has_hydrolysis = backends.iter().any(|b| matches!(b, Backend::Hydrolysis));

        if has_apple {
            let spinner = shell::spinner("Scaffolding Apple backend...");
            project.init_apple_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created Apple backend");
        }

        if has_android {
            let spinner = shell::spinner("Scaffolding Android backend...");
            project.init_android_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created Android backend");
        }

        if has_gtk4 {
            let spinner = shell::spinner("Scaffolding GTK4 backend...");
            project.init_gtk4_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created GTK4 backend");
        }

        if has_hydrolysis {
            let spinner = shell::spinner("Scaffolding hydrolysis backend...");
            project.init_hydrolysis_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Created hydrolysis backend");
        }
    }

    // Final message
    line!();
    success!("Project created at {}", project_path.display());
    line!();
    line!("Next steps:");
    line!("  cd {folder_name}");
    if let Some(command) = next_run_command(package_type, &backends) {
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
            "Unknown backend(s): {}. Valid values: apple, android, gtk4, hydrolysis",
            invalid.join(", ")
        )
    }
}

fn next_run_command(package_type: PackageType, backends: &[Backend]) -> Option<&'static str> {
    if package_type == PackageType::Playground {
        #[cfg(target_os = "macos")]
        return Some("water run --platform macos");

        #[cfg(target_os = "linux")]
        return Some("water run --platform linux");

        #[cfg(target_os = "windows")]
        return Some("water run --platform windows");

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        return None;
    }

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

    if backends.iter().any(|b| matches!(b, Backend::Hydrolysis)) {
        #[cfg(target_os = "macos")]
        return Some("water run --platform macos --backend hydrolysis");

        #[cfg(target_os = "linux")]
        return Some("water run --platform linux --backend hydrolysis");

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
    use super::{Backend, PackageType, next_run_command, parse_backends};

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
            next_run_command(PackageType::App, &[Backend::Apple, Backend::Gtk4]),
            Some("water run --platform ios")
        );
        assert_eq!(
            next_run_command(PackageType::App, &[Backend::Android, Backend::Gtk4]),
            Some("water run --platform android")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn next_run_command_gtk4_is_valid_on_macos() {
        assert_eq!(
            next_run_command(PackageType::App, &[Backend::Gtk4]),
            Some("water run --platform macos --backend gtk4")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn next_run_command_gtk4_is_valid_on_linux() {
        assert_eq!(
            next_run_command(PackageType::App, &[Backend::Gtk4]),
            Some("water run --platform linux")
        );
    }
}

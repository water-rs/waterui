//! `water create` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail, eyre};
use dialoguer::{Input, MultiSelect, theme::ColorfulTheme};
use heck::{ToKebabCase, ToSnakeCase};

use crate::shell;
use crate::{header, line, success};
use waterui_cli::project::{CreateOptions, PackageType, Project};
use waterui_cli::project_types::BundleIdentifier;

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

struct CreatePlan {
    name: String,
    bundle_id: String,
    backends: Vec<Backend>,
    package_type: PackageType,
    waterui_path: Option<PathBuf>,
    folder_name: String,
    project_path: PathBuf,
}

/// Backend options for scaffolding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::Gtk4 => "GTK4 (Linux)",
            Self::Hydrolysis => "Hydrolysis (Linux/macOS/Windows)",
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
    let plan = resolve_create_plan(&args)?;
    header!("Creating WaterUI project: {}", plan.name);
    let mut project = create_project(&plan).await?;
    initialize_requested_backends(&mut project, &plan).await?;
    print_create_summary(&plan);
    Ok(())
}

fn resolve_create_plan(args: &Args) -> Result<CreatePlan> {
    let interactive = shell::is_interactive();
    let package_type = args.mode.package_type();
    let name = resolve_project_name(args, interactive)?;
    let waterui_path = resolve_waterui_path(args, interactive)?;
    let bundle_id = resolve_bundle_id(args, interactive, &name)?;
    let backends = resolve_backends(args, interactive, package_type)?;

    if package_type == PackageType::App {
        validate_backends_on_host(&backends)?;
    }

    let folder_name = name.to_kebab_case();
    let project_path = std::env::current_dir()?.join(&folder_name);

    Ok(CreatePlan {
        name,
        bundle_id,
        backends,
        package_type,
        waterui_path,
        folder_name,
        project_path,
    })
}

fn resolve_project_name(args: &Args, interactive: bool) -> Result<String> {
    match args.name.clone() {
        Some(name) => Ok(name),
        None if interactive => prompt_name(),
        None => Err(eyre!("Project name is required")),
    }
}

fn resolve_waterui_path(args: &Args, interactive: bool) -> Result<Option<PathBuf>> {
    if !args.dev {
        return Ok(args.waterui_path.clone());
    }

    let user_input = if interactive {
        prompt_waterui_path()?
    } else {
        ".".to_string()
    };
    let input_path = std::path::Path::new(&user_input);
    let relative_to_new_project = if input_path.is_relative() {
        PathBuf::from("..").join(input_path)
    } else {
        input_path.to_path_buf()
    };

    Ok(Some(relative_to_new_project))
}

fn resolve_bundle_id(args: &Args, interactive: bool, name: &str) -> Result<String> {
    match args.bundle_id.clone() {
        Some(bundle_id) => Ok(bundle_id),
        None if interactive => prompt_bundle_id(name),
        None => Ok(default_bundle_id(name)),
    }
}

fn resolve_backends(
    args: &Args,
    interactive: bool,
    package_type: PackageType,
) -> Result<Vec<Backend>> {
    if package_type == PackageType::Playground {
        if args.backends.is_some() {
            bail!(
                "Playground mode does not support --backends; backend projects are auto-managed."
            );
        }
        return Ok(Vec::new());
    }

    let backends = match &args.backends {
        Some(values) => parse_backends(values)?,
        None if interactive => prompt_backends()?,
        None => vec![Backend::Apple, Backend::Android],
    };

    if backends.is_empty() {
        bail!("At least one backend is required. Choose from: apple, android, gtk4, hydrolysis.");
    }

    Ok(backends)
}

async fn create_project(plan: &CreatePlan) -> Result<Project> {
    let spinner = shell::spinner("Creating project files...");
    let project = Project::create(
        &plan.project_path,
        CreateOptions {
            name: plan.name.clone(),
            bundle_identifier: BundleIdentifier::try_from(plan.bundle_id.as_str())
                .map_err(|error| eyre!(error))?,
            package_type: plan.package_type,
            waterui_path: plan.waterui_path.clone(),
            author: whoami::username(),
        },
    )
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Created Cargo.toml and src/lib.rs");
    Ok(project)
}

async fn initialize_requested_backends(project: &mut Project, plan: &CreatePlan) -> Result<()> {
    if plan.package_type != PackageType::App {
        return Ok(());
    }

    initialize_backend_if_requested(project, &plan.backends, Backend::Apple).await?;
    initialize_backend_if_requested(project, &plan.backends, Backend::Android).await?;
    initialize_backend_if_requested(project, &plan.backends, Backend::Gtk4).await?;
    initialize_backend_if_requested(project, &plan.backends, Backend::Hydrolysis).await
}

async fn initialize_backend_if_requested(
    project: &mut Project,
    backends: &[Backend],
    backend: Backend,
) -> Result<()> {
    if !backends.contains(&backend) {
        return Ok(());
    }

    let (spinner_message, success_message) = match backend {
        Backend::Apple => ("Scaffolding Apple backend...", "Created Apple backend"),
        Backend::Android => ("Scaffolding Android backend...", "Created Android backend"),
        Backend::Gtk4 => ("Scaffolding GTK4 backend...", "Created GTK4 backend"),
        Backend::Hydrolysis => (
            "Scaffolding hydrolysis backend...",
            "Created hydrolysis backend",
        ),
    };

    let spinner = shell::spinner(spinner_message);
    match backend {
        Backend::Apple => project.init_apple_backend().await?,
        Backend::Android => project.init_android_backend().await?,
        Backend::Gtk4 => project.init_gtk4_backend().await?,
        Backend::Hydrolysis => project.init_hydrolysis_backend().await?,
    }
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("{success_message}");
    Ok(())
}

fn print_create_summary(plan: &CreatePlan) {
    line!();
    success!("Project created at {}", plan.project_path.display());
    line!();
    line!("Next steps:");
    line!("  cd {}", plan.folder_name);
    if let Some(command) = next_run_command(plan.package_type, &plan.backends) {
        line!("  {command}");
    }
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
        #[cfg(target_os = "linux")]
        return Some("water run --platform linux");

        #[cfg(not(target_os = "linux"))]
        return None;
    }

    if backends.iter().any(|b| matches!(b, Backend::Hydrolysis)) {
        #[cfg(target_os = "macos")]
        return Some("water run --platform macos --backend hydrolysis");

        #[cfg(target_os = "linux")]
        return Some("water run --platform linux --backend hydrolysis");

        #[cfg(target_os = "windows")]
        return Some("water run --platform windows --backend hydrolysis");

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        return None;
    }

    None
}

fn prompt_backends() -> Result<Vec<Backend>> {
    let items: Vec<&str> = Backend::ALL.iter().map(|b| b.label()).collect();
    let defaults = vec![true, true, false, false]; // Apple and Android selected by default

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select backends")
        .items(&items)
        .defaults(&defaults)
        .interact()?;

    Ok(selections.into_iter().map(|i| Backend::ALL[i]).collect())
}

fn validate_backends_on_host(backends: &[Backend]) -> Result<()> {
    let wants_gtk4 = backends
        .iter()
        .any(|backend| matches!(backend, Backend::Gtk4));
    if wants_gtk4 && !cfg!(target_os = "linux") {
        bail!("GTK4 backend is only supported on Linux hosts");
    }

    let wants_hydrolysis = backends
        .iter()
        .any(|backend| matches!(backend, Backend::Hydrolysis));
    if wants_hydrolysis
        && !cfg!(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows"
        ))
    {
        bail!("Hydrolysis backend is only supported on macOS, Linux, or Windows hosts");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Backend, PackageType, next_run_command, parse_backends, validate_backends_on_host,
    };

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

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn gtk4_backend_is_rejected_on_non_linux_hosts() {
        assert!(validate_backends_on_host(&[Backend::Gtk4]).is_err());
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

    #[cfg(target_os = "linux")]
    #[test]
    fn next_run_command_gtk4_is_valid_on_linux() {
        assert_eq!(
            next_run_command(PackageType::App, &[Backend::Gtk4]),
            Some("water run --platform linux")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn next_run_command_hydrolysis_is_valid_on_windows() {
        assert_eq!(
            next_run_command(PackageType::App, &[Backend::Hydrolysis]),
            Some("water run --platform windows --backend hydrolysis")
        );
    }
}

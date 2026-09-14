//! `water create` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use dialoguer::{Input, MultiSelect, theme::ColorfulTheme};
use eyre::{Result, bail, eyre};
use heck::{ToKebabCase, ToSnakeCase};

use crate::shell::Shell;
use crate::{header, line, success};
use waterui_cli::framework::FrameworkChannel;
use waterui_cli::project::{CreateOptions, PackageType, Project, WebScaffold};
use waterui_cli::project_types::BundleIdentifier;
use waterui_cli::web::PackageManager;

/// Arguments for the create command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Project display name (e.g., "Water Example" creates folder "water-example").
    name: Option<String>,

    /// Bundle identifier (defaults to `dev.waterui.<name>`).
    #[arg(long)]
    bundle_id: Option<String>,

    /// Backends to scaffold (apple, android, gtk4, hydrolysis, esp32).
    #[arg(long, value_delimiter = ',')]
    backends: Option<Vec<String>>,

    /// Path to local `WaterUI` repository (for development).
    #[arg(long)]
    waterui_path: Option<PathBuf>,

    /// Framework channel: dev, nightly or stable (default).
    #[arg(long, conflicts_with_all = ["waterui_path", "framework_manifest"])]
    channel: Option<FrameworkChannel>,

    /// Pin the framework to a certified `framework.json` on disk, exactly as a
    /// manifest downloaded from its release resolves — reproducible scaffolding
    /// from a certified manifest (used by release preflight).
    #[arg(long, conflicts_with_all = ["waterui_path", "channel"])]
    framework_manifest: Option<PathBuf>,

    /// Project mode (`app` or `playground`).
    #[arg(long, value_enum, default_value_t = ProjectMode::App)]
    mode: ProjectMode,

    /// Project template: `app` for the standard Rust shell, `web` to add a
    /// `web/` Vite frontend mounted through `include_web!`.
    #[arg(long, value_enum, default_value_t = CreateTemplate::App)]
    template: CreateTemplate,

    /// JavaScript package manager declared in `[web]` (default bun).
    #[arg(long, value_enum)]
    package_manager: Option<PackageManager>,

    /// Vite template for the scaffolded frontend (e.g. `vanilla-ts`); skips
    /// `create vite`'s interactive framework picker.
    #[arg(long)]
    vite_template: Option<String>,
}

struct CreatePlan {
    name: String,
    bundle_id: String,
    backends: Vec<Backend>,
    package_type: PackageType,
    waterui_path: Option<PathBuf>,
    channel: Option<FrameworkChannel>,
    framework_manifest: Option<PathBuf>,
    folder_name: String,
    project_path: PathBuf,
    template: CreateTemplate,
    package_manager: PackageManager,
    vite_template: Option<String>,
}

/// Backend options for scaffolding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Apple,
    Android,
    Gtk4,
    Hydrolysis,
    Esp32,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum ProjectMode {
    #[default]
    App,
    Playground,
}

/// The starting shape `create` scaffolds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
enum CreateTemplate {
    /// The standard Rust shell.
    #[default]
    App,
    /// A Rust shell whose root view is `include_web!("web")` plus a `web/`
    /// Vite frontend.
    Web,
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
    const ALL: [Self; 5] = [
        Self::Apple,
        Self::Android,
        Self::Gtk4,
        Self::Hydrolysis,
        Self::Esp32,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Apple => "Apple (iOS/macOS)",
            Self::Android => "Android",
            Self::Gtk4 => "GTK4 (Linux)",
            Self::Hydrolysis => "Hydrolysis (Linux/macOS/Windows)",
            Self::Esp32 => "ESP32 (Dew firmware)",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "apple" | "ios" | "macos" => Some(Self::Apple),
            "android" => Some(Self::Android),
            "gtk" | "gtk4" | "linux" => Some(Self::Gtk4),
            "hydrolysis" => Some(Self::Hydrolysis),
            "esp32" | "esp32s3" | "dew" => Some(Self::Esp32),
            _ => None,
        }
    }
}

/// Run the create command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let plan = resolve_create_plan(shell, &args)?;
    if plan.template == CreateTemplate::Web {
        // The declared manager must exist before anything touches disk.
        super::web::ensure_installed(plan.package_manager).await?;
    }
    header!(shell, "Creating WaterUI project: {}", plan.name);
    let mut project = create_project(shell, &plan).await?;
    if plan.template == CreateTemplate::Web {
        super::web::create_vite(
            shell,
            project.root(),
            "web",
            plan.package_manager,
            plan.vite_template.as_deref(),
        )
        .await?;
        super::web::brand_overlay(shell, &project.root().join("web"), &plan.name)?;
        super::web::install_dependencies(shell, plan.package_manager, &project.root().join("web"))
            .await?;
    }
    initialize_requested_backends(shell, &mut project, &plan).await?;
    print_create_summary(shell, &plan);
    Ok(())
}

fn resolve_create_plan(shell: &Shell, args: &Args) -> Result<CreatePlan> {
    let interactive = shell.is_interactive();
    let package_type = args.mode.package_type();
    let name = resolve_project_name(args, interactive)?;
    let folder_name = name.to_kebab_case();
    let project_path = std::env::current_dir()?.join(&folder_name);
    let waterui_path = args.waterui_path.clone();
    let bundle_id = resolve_bundle_id(args, interactive, &name)?;
    let backends = resolve_backends(args, interactive, package_type)?;

    if package_type == PackageType::App {
        validate_backends_on_host(&backends)?;
    }

    if args.template == CreateTemplate::App
        && (args.package_manager.is_some() || args.vite_template.is_some())
    {
        bail!("--package-manager and --vite-template require --template web");
    }
    let package_manager = resolve_package_manager(args, interactive)?;

    Ok(CreatePlan {
        name,
        bundle_id,
        backends,
        package_type,
        waterui_path,
        channel: args.channel,
        framework_manifest: args.framework_manifest.clone(),
        folder_name,
        project_path,
        template: args.template,
        package_manager,
        vite_template: args.vite_template.clone(),
    })
}

/// The declared manager: the flag, else a prompt when the web template is
/// being created interactively, else `bun`.
fn resolve_package_manager(args: &Args, interactive: bool) -> Result<PackageManager> {
    if let Some(package_manager) = args.package_manager {
        return Ok(package_manager);
    }
    if args.template == CreateTemplate::Web && interactive {
        return super::web::prompt_package_manager(PackageManager::Bun);
    }
    Ok(PackageManager::Bun)
}

fn resolve_project_name(args: &Args, interactive: bool) -> Result<String> {
    match args.name.clone() {
        Some(name) => Ok(name),
        None if interactive => prompt_name(),
        None => Err(eyre!("Project name is required")),
    }
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
        bail!(
            "At least one backend is required. Choose from: apple, android, gtk4, hydrolysis, esp32."
        );
    }

    Ok(backends)
}

async fn create_project(shell: &Shell, plan: &CreatePlan) -> Result<Project> {
    let spinner = shell.spinner("Creating project files...");
    let project = Project::create(
        &plan.project_path,
        CreateOptions {
            name: plan.name.clone(),
            bundle_identifier: BundleIdentifier::try_from(plan.bundle_id.as_str())
                .map_err(|error| eyre!(error))?,
            package_type: plan.package_type,
            waterui_path: plan.waterui_path.clone(),
            channel: plan.channel,
            framework_manifest: plan.framework_manifest.clone(),
            framework: None,
            author: whoami::username()
                .map_err(|error| eyre!("Failed to determine project author: {error}"))?,
            web: (plan.template == CreateTemplate::Web).then(|| WebScaffold {
                package_manager: plan.package_manager,
                include_arg: "web".to_string(),
            }),
        },
    )
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Created Cargo.toml and src/lib.rs");
    Ok(project)
}

async fn initialize_requested_backends(
    shell: &Shell,
    project: &mut Project,
    plan: &CreatePlan,
) -> Result<()> {
    if plan.package_type != PackageType::App {
        return Ok(());
    }

    initialize_backend_if_requested(shell, project, &plan.backends, Backend::Apple).await?;
    initialize_backend_if_requested(shell, project, &plan.backends, Backend::Android).await?;
    initialize_backend_if_requested(shell, project, &plan.backends, Backend::Gtk4).await?;
    initialize_backend_if_requested(shell, project, &plan.backends, Backend::Hydrolysis).await?;
    initialize_backend_if_requested(shell, project, &plan.backends, Backend::Esp32).await
}

async fn initialize_backend_if_requested(
    shell: &Shell,
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
        Backend::Esp32 => ("Scaffolding ESP32 backend...", "Created ESP32 backend"),
    };

    let spinner = shell.spinner(spinner_message);
    match backend {
        Backend::Apple => project.init_apple_backend().await?,
        Backend::Android => project.init_android_backend().await?,
        Backend::Gtk4 => project.init_gtk4_backend().await?,
        Backend::Hydrolysis => project.init_hydrolysis_backend().await?,
        Backend::Esp32 => project.init_esp32_backend().await?,
    }
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "{success_message}");
    Ok(())
}

fn print_create_summary(shell: &Shell, plan: &CreatePlan) {
    line!(shell);
    success!(shell, "Project created at {}", plan.project_path.display());
    line!(shell);
    line!(shell, "Next steps:");
    line!(shell, "  cd {}", plan.folder_name);
    if let Some(command) = next_run_command(plan.package_type, &plan.backends) {
        line!(shell, "  {command}");
    }
}

fn prompt_name() -> Result<String> {
    Ok(Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Project name")
        .interact_text()?)
}

fn default_bundle_id(app_name: &str) -> String {
    format!("dev.waterui.{}", app_name.to_snake_case())
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
            "Unknown backend(s): {}. Valid values: apple, android, gtk4, hydrolysis, esp32",
            invalid.join(", ")
        );
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

    if backends.iter().any(|b| matches!(b, Backend::Esp32)) {
        return Some("water run --platform esp32s3");
    }

    None
}

fn prompt_backends() -> Result<Vec<Backend>> {
    let items: Vec<&str> = Backend::ALL.iter().map(|b| b.label()).collect();
    let defaults = vec![true, true, false, false, false]; // Apple and Android selected by default

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
        Args, Backend, FrameworkChannel, PackageType, next_run_command, parse_backends,
        resolve_create_plan,
    };
    use crate::shell::Shell;
    use clap::Parser;
    use std::path::PathBuf;
    // Only the non-Linux host test exercises this.
    #[cfg(not(target_os = "linux"))]
    use super::validate_backends_on_host;

    /// Scaffolds a `WaterUI` project plus a `create vite` frontend into `root`,
    /// applies the brand overlay, and installs dependencies — the same steps
    /// `run()` performs for `--template web`.
    async fn scaffold_web_project(root: &std::path::Path, name: &str, vite_template: &str) {
        let waterui_checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli/ has a parent")
            .to_path_buf();
        let shell = Shell::new(false);
        let project = waterui_cli::project::Project::create(
            root,
            waterui_cli::project::CreateOptions {
                name: name.to_string(),
                bundle_identifier: waterui_cli::project_types::BundleIdentifier::try_from(
                    "dev.waterui.webapp",
                )
                .expect("bundle identifier"),
                package_type: PackageType::App,
                waterui_path: Some(waterui_checkout),
                channel: None,
                framework_manifest: None,
                framework: None,
                author: "water test".to_string(),
                web: Some(waterui_cli::project::WebScaffold {
                    package_manager: super::PackageManager::Bun,
                    include_arg: "web".to_string(),
                }),
            },
        )
        .await
        .expect("project scaffold");

        crate::commands::web::create_vite(
            &shell,
            project.root(),
            "web",
            super::PackageManager::Bun,
            Some(vite_template),
        )
        .await
        .expect("vite scaffold");
        crate::commands::web::brand_overlay(&shell, &project.root().join("web"), name)
            .expect("brand overlay");
        crate::commands::web::install_dependencies(
            &shell,
            super::PackageManager::Bun,
            &project.root().join("web"),
        )
        .await
        .expect("dependency install");
    }

    /// Every text file under `dir`, skipping `node_modules` and `.git`.
    fn web_project_files(dir: &std::path::Path) -> Vec<(PathBuf, String)> {
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_entry(|entry| {
                entry.file_name() != "node_modules" && entry.file_name() != ".git"
            })
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                std::fs::read_to_string(entry.path())
                    .ok()
                    .map(|contents| (entry.into_path(), contents))
            })
            .collect()
    }

    /// End-to-end `--template web`: scaffolds a project plus a Vite frontend
    /// into a tempdir, verifies the brand overlay landed, `bun run build`s
    /// the frontend, and `cargo check`s the result against this checkout.
    ///
    /// Gated on `bun` being on PATH; skipped in environments without it.
    #[test]
    fn create_template_web_scaffolds_and_checks() {
        if which::which("bun").is_err() {
            eprintln!("skipping: bun is not installed");
            return;
        }
        smol::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let project_path = temp.path().join("web-app");
            scaffold_web_project(&project_path, "WaterUI App", "vanilla-ts").await;

            assert!(project_path.join("web/package.json").exists());
            let water_toml =
                std::fs::read_to_string(project_path.join("Water.toml")).expect("Water.toml");
            assert!(
                water_toml.contains("[web]") && water_toml.contains("package_manager = \"bun\""),
                "Water.toml declares the manager:\n{water_toml}"
            );
            let lib_rs =
                std::fs::read_to_string(project_path.join("src/lib.rs")).expect("src/lib.rs");
            assert!(
                lib_rs.contains("include_web!(\"web\")"),
                "the root view mounts the frontend:\n{lib_rs}"
            );
            assert!(
                lib_rs.contains("#[js_api]") && lib_rs.contains(".serve(Api)"),
                "the root view serves the bridge API:\n{lib_rs}"
            );

            // The overlay landed: the WaterUI mark replaced the starter's
            // favicon (`vite.svg` on Vite 7, `favicon.svg` on Vite 8), the
            // title is the app name, and no starter marketing copy survives.
            assert!(project_path.join("web/public/waterui.svg").is_file());
            assert!(!project_path.join("web/public/vite.svg").exists());
            assert!(!project_path.join("web/public/favicon.svg").exists());
            assert!(!project_path.join("web/src/counter.ts").exists());
            let index_html =
                std::fs::read_to_string(project_path.join("web/index.html")).expect("index.html");
            assert!(index_html.contains("WaterUI"), "{index_html}");
            for (path, contents) in web_project_files(&project_path.join("web")) {
                assert!(
                    !contents.contains("Explore Vite"),
                    "{} still contains Vite marketing copy",
                    path.display()
                );
            }

            // The branded starter type-checks and bundles.
            let status = smol::process::Command::new("bun")
                .args(["run", "build"])
                .current_dir(project_path.join("web"))
                .status()
                .await
                .expect("bun run build runs");
            assert!(status.success(), "the branded frontend must build");

            let status = smol::process::Command::new("cargo")
                .arg("check")
                .current_dir(&project_path)
                .status()
                .await
                .expect("cargo check runs");
            assert!(status.success(), "the generated project must check");
        });
    }

    /// The React overlay compiles: `react-ts` scaffolds get a branded
    /// `App.tsx` that `tsc -b && vite build` accepts.
    ///
    /// Gated on `bun` being on PATH; skipped in environments without it.
    #[test]
    fn create_template_web_react_frontend_builds() {
        if which::which("bun").is_err() {
            eprintln!("skipping: bun is not installed");
            return;
        }
        smol::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let project_path = temp.path().join("web-app");
            scaffold_web_project(&project_path, "WaterUI App", "react-ts").await;

            let app_tsx = project_path.join("web/src/App.tsx");
            assert!(app_tsx.is_file(), "react-ts scaffolds src/App.tsx");
            let contents = std::fs::read_to_string(&app_tsx).expect("App.tsx");
            assert!(contents.contains("WaterUI + React"), "{contents}");
            assert!(project_path.join("web/public/waterui.svg").is_file());

            let status = smol::process::Command::new("bun")
                .args(["run", "build"])
                .current_dir(project_path.join("web"))
                .status()
                .await
                .expect("bun run build runs");
            assert!(status.success(), "the branded React frontend must build");
        });
    }

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

    #[derive(Parser)]
    struct CreateCommand {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn framework_selection_never_implicitly_uses_a_local_checkout() {
        let shell = Shell::new(true);
        for channel in ["dev", "nightly", "stable"] {
            let command = CreateCommand::try_parse_from([
                "water",
                "Example",
                "--mode",
                "playground",
                "--channel",
                channel,
            ])
            .expect("channel arguments");
            let plan = resolve_create_plan(&shell, &command.args).expect("create plan");
            assert_eq!(
                plan.channel,
                Some(channel.parse::<FrameworkChannel>().expect("channel"))
            );
            assert_eq!(plan.waterui_path, None);
        }
        let command = CreateCommand::try_parse_from([
            "water",
            "Example",
            "--mode",
            "playground",
            "--waterui-path",
            "..",
        ])
        .expect("local source arguments");
        let plan = resolve_create_plan(&shell, &command.args).expect("local create plan");
        assert_eq!(plan.waterui_path, Some(PathBuf::from("..")));
        assert_eq!(plan.channel, None);
        assert!(
            CreateCommand::try_parse_from([
                "water",
                "Example",
                "--channel",
                "nightly",
                "--waterui-path",
                "..",
            ])
            .is_err()
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

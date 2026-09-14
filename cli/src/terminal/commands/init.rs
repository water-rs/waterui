//! `water init` command implementation: turn the current directory into a
//! `WaterUI` project, adopting or creating a web frontend per the fixed
//! decision tree in `project_model::web::plan_init`.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail, eyre};
use dialoguer::{Input, Select, theme::ColorfulTheme};
use heck::ToSnakeCase;

use crate::shell::Shell;
use crate::{header, line, success};
use waterui_cli::framework::FrameworkChannel;
use waterui_cli::project::{CreateOptions, PackageType, Project, WebScaffold};
use waterui_cli::project_types::BundleIdentifier;
use waterui_cli::web::{
    self, ExistingFrontendMode, InitAction, InitAnswers, PackageManager, WebSource, plan_init,
};

/// Arguments for the init command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Project display name (defaults to the directory name).
    name: Option<String>,

    /// Bundle identifier (defaults to `dev.waterui.<name>`).
    #[arg(long)]
    bundle_id: Option<String>,

    /// Path to local `WaterUI` repository (for development).
    #[arg(long)]
    waterui_path: Option<PathBuf>,

    /// Framework channel: dev, nightly or stable (default).
    #[arg(long, conflicts_with_all = ["waterui_path", "framework_manifest"])]
    channel: Option<FrameworkChannel>,

    /// Pin the framework to a certified `framework.json` on disk, exactly as a
    /// manifest downloaded from its release resolves.
    #[arg(long, conflicts_with_all = ["waterui_path", "channel"])]
    framework_manifest: Option<PathBuf>,

    /// Frontend source: `new` scaffolds a Vite project into `web/`; any other
    /// value is a path to an existing web project.
    #[arg(long)]
    web: Option<String>,

    /// How an existing `--web <path>` project joins: copy it into `web/` or
    /// reference it in place.
    #[arg(long, value_enum, requires = "web")]
    web_mode: Option<WebMode>,

    /// JavaScript package manager declared in `[web]` (default: inferred from
    /// the frontend's lockfile, else bun).
    #[arg(long, value_enum)]
    package_manager: Option<PackageManager>,

    /// Vite template for a scaffolded frontend (e.g. `vanilla-ts`); skips
    /// `create vite`'s interactive framework picker.
    #[arg(long)]
    vite_template: Option<String>,
}

/// `--web-mode` values.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum WebMode {
    Copy,
    Reference,
}

impl From<WebMode> for ExistingFrontendMode {
    fn from(mode: WebMode) -> Self {
        match mode {
            WebMode::Copy => Self::Copy,
            WebMode::Reference => Self::Reference,
        }
    }
}

/// Run the init command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let entries = top_level_entries(&project_root)?;
    let answers = resolve_answers(shell, &args, &entries)?;
    let actions = plan_init(&project_root, &entries, &answers)?;
    let package_manager = answers
        .package_manager
        .expect("resolve_answers always settles the package manager");

    super::web::ensure_installed(package_manager).await?;

    let name = match &args.name {
        Some(name) => name.clone(),
        None => project_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| eyre!("cannot derive a project name; pass one as an argument"))?,
    };

    header!(
        shell,
        "Initializing WaterUI project in {}",
        project_root.display()
    );
    let include_arg = execute_plan(
        shell,
        &project_root,
        &actions,
        package_manager,
        &args,
        &name,
    )
    .await?;
    scaffold_shell(
        shell,
        &project_root,
        &args,
        package_manager,
        &include_arg,
        name,
    )
    .await?;
    success!(shell, "WaterUI project initialized");
    line!(shell);
    line!(shell, "Next steps:");
    line!(shell, "  water run --platform <platform>");
    Ok(())
}

fn top_level_entries(root: &std::path::Path) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        entries.push(entry.file_name().to_string_lossy().into_owned());
    }
    entries.sort();
    Ok(entries)
}

/// Fill every unset answer: flags first, prompts when interactive, defaults
/// otherwise. Non-interactive runs never block on a prompt.
fn resolve_answers(shell: &Shell, args: &Args, entries: &[String]) -> Result<InitAnswers> {
    let interactive = shell.is_interactive();
    let has_frontend = entries.iter().any(|entry| entry == "package.json");

    if !has_frontend && args.web.as_deref() == Some("new") && args.web_mode.is_some() {
        bail!("--web-mode only applies to an existing project path");
    }

    let web = match (has_frontend, args.web.as_deref()) {
        (true, _) => None,
        (false, Some("new")) => Some(WebSource::New),
        (false, Some(path)) => {
            let source = PathBuf::from(path);
            if !source.join("package.json").exists() {
                bail!("{} has no package.json — it is not a web project", path);
            }
            Some(WebSource::Existing(source))
        }
        (false, None) if interactive => Some(prompt_web_source()?),
        (false, None) => Some(WebSource::New),
    };

    let web_mode = match (&web, args.web_mode) {
        (Some(WebSource::Existing(_)), Some(mode)) => Some(mode.into()),
        (Some(WebSource::Existing(_)), None) if interactive => Some(prompt_web_mode()?),
        (Some(WebSource::Existing(_)), None) => Some(ExistingFrontendMode::Copy),
        _ => None,
    };

    let package_manager = match args.package_manager {
        Some(pm) => Some(pm),
        None if interactive => {
            let initial = web::lockfile_package_manager(entries).unwrap_or_default();
            Some(super::web::prompt_package_manager(initial)?)
        }
        None => Some(web::lockfile_package_manager(entries).unwrap_or_default()),
    };

    Ok(InitAnswers {
        web,
        web_mode,
        package_manager,
    })
}

fn prompt_web_source() -> Result<WebSource> {
    let options = [
        "Create a new Vite app in web/",
        "Use an existing web project",
    ];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("This directory has no web frontend")
        .items(options)
        .default(0)
        .interact()?;
    if selection == 0 {
        return Ok(WebSource::New);
    }
    let path = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("Path to the existing project")
        .interact_text()?;
    let source = PathBuf::from(&path);
    if !source.join("package.json").exists() {
        bail!("{path} has no package.json — it is not a web project");
    }
    Ok(WebSource::Existing(source))
}

fn prompt_web_mode() -> Result<ExistingFrontendMode> {
    let options = [
        "Copy the project into web/",
        "Reference it in place (include_web! points at it)",
    ];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How should the project join?")
        .items(options)
        .default(0)
        .interact()?;
    Ok(match selection {
        0 => ExistingFrontendMode::Copy,
        _ => ExistingFrontendMode::Reference,
    })
}

/// Execute the planned actions in order; returns the `include_web!` argument
/// the shell's root view must use.
async fn execute_plan(
    shell: &Shell,
    root: &std::path::Path,
    actions: &[InitAction],
    package_manager: PackageManager,
    args: &Args,
    display_name: &str,
) -> Result<String> {
    let mut include_arg = None;
    for action in actions {
        match action {
            InitAction::MoveFrontendToWeb { entries } => {
                super::web::move_entries_to_web(root, entries).await?;
                success!(shell, "Moved the frontend into web/");
            }
            InitAction::ScaffoldVite => {
                super::web::create_vite(
                    shell,
                    root,
                    "web",
                    package_manager,
                    args.vite_template.as_deref(),
                )
                .await?;
                super::web::brand_overlay(shell, &root.join("web"), display_name)?;
            }
            InitAction::CopyFrontend { source } => {
                let dest = root.join("web");
                super::web::copy_frontend(source, &dest).await?;
                success!(shell, "Copied {} into web/", source.display());
            }
            InitAction::InstallDependencies => {
                super::web::install_dependencies(shell, package_manager, &root.join("web")).await?;
            }
            InitAction::ScaffoldShell { web_arg } => {
                include_arg = Some(web_arg.clone());
            }
        }
    }
    include_arg.ok_or_else(|| eyre!("the init plan produced no shell scaffold step"))
}

/// The Rust shell: the root view is `include_web!(<include_arg>)`.
async fn scaffold_shell(
    shell: &Shell,
    root: &std::path::Path,
    args: &Args,
    package_manager: PackageManager,
    include_arg: &str,
    name: String,
) -> Result<()> {
    let bundle_id = args
        .bundle_id
        .clone()
        .unwrap_or_else(|| format!("dev.waterui.{}", name.to_snake_case()));

    let spinner = shell.spinner("Scaffolding the Rust shell...");
    let project = Project::init(
        root,
        CreateOptions {
            name,
            bundle_identifier: BundleIdentifier::try_from(bundle_id.as_str())
                .map_err(|error| eyre!(error))?,
            package_type: PackageType::App,
            waterui_path: args.waterui_path.clone(),
            channel: args.channel,
            framework_manifest: args.framework_manifest.clone(),
            framework: None,
            author: whoami::username()
                .map_err(|error| eyre!("Failed to determine project author: {error}"))?,
            web: Some(WebScaffold {
                package_manager,
                include_arg: include_arg.to_string(),
            }),
        },
    )
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Created Cargo.toml, src/lib.rs, and Water.toml");
    drop(project);
    Ok(())
}

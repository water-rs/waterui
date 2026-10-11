//! `water create` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use dialoguer::{Input, theme::ColorfulTheme};
use eyre::{Result, bail, eyre};
use heck::ToKebabCase;

use crate::shell::Shell;
use crate::{header, line, success};
use waterui_cli::framework::FrameworkChannel;
use waterui_cli::project::{CreateOptions, Project, ProjectDraft, WebScaffold};
use waterui_cli::project_types::{BundleIdentifier, default_bundle_identifier};
use waterui_cli::toolchain::Host;
use waterui_cli::web::PackageManager;

/// Arguments for the create command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Project display name (e.g., "Water Example" creates folder "water-example").
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
    /// manifest downloaded from its release resolves — reproducible scaffolding
    /// from a certified manifest (used by release preflight).
    #[arg(long, conflicts_with_all = ["waterui_path", "channel"])]
    framework_manifest: Option<PathBuf>,

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
    waterui_path: Option<PathBuf>,
    channel: Option<FrameworkChannel>,
    framework_manifest: Option<PathBuf>,
    folder_name: String,
    project_path: PathBuf,
    template: CreateTemplate,
    package_manager: PackageManager,
    vite_template: Option<String>,
}

/// The starting shape `create` scaffolds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
enum CreateTemplate {
    /// The standard Rust shell.
    #[default]
    App,
    /// A Rust shell whose root view is `include_web!(resources, "web")` plus a `web/`
    /// Vite frontend.
    Web,
}

/// Run the create command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let host = Host::current();
    let plan = resolve_create_plan(shell, &host, &args)?;
    if plan.template == CreateTemplate::Web {
        // The declared manager must exist before anything touches disk.
        super::web::ensure_installed(&host, plan.package_manager).await?;
    }
    header!(shell, "Creating WaterUI project: {}", plan.name);
    let draft = create_project(shell, &host, &plan).await?;
    if plan.template == CreateTemplate::Web
        && let Err(error) = scaffold_web_frontend(shell, &plan, draft.project()).await
    {
        return Err(draft.discard(error).await);
    }
    let spinner = shell.spinner("Scaffolding generated crates and fetching declared fonts...");
    let fetched = draft.finish().await;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    for (name, path) in fetched? {
        success!(shell, "Fetched font '{name}' to {}", path.display());
    }
    print_create_summary(shell, &plan);
    Ok(())
}

fn resolve_create_plan(shell: &Shell, host: &Host, args: &Args) -> Result<CreatePlan> {
    let interactive = shell.is_interactive();
    let name = resolve_project_name(args, interactive)?;
    let folder_name = name.to_kebab_case();
    let project_path = host.cwd().join(&folder_name);
    let waterui_path = args.waterui_path.clone();
    let bundle_id = resolve_bundle_id(args, interactive, &name)?;

    if args.template == CreateTemplate::App
        && (args.package_manager.is_some() || args.vite_template.is_some())
    {
        bail!("--package-manager and --vite-template require --template web");
    }
    let package_manager = resolve_package_manager(args, interactive)?;

    Ok(CreatePlan {
        name,
        bundle_id,
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
        None => default_bundle_id(name).map_err(|error| eyre!(error)),
    }
}

async fn create_project(shell: &Shell, host: &Host, plan: &CreatePlan) -> Result<ProjectDraft> {
    let spinner = shell.spinner("Creating project files...");
    let draft = ProjectDraft::create(
        host,
        &plan.project_path,
        CreateOptions {
            name: plan.name.clone(),
            bundle_identifier: BundleIdentifier::try_from(plan.bundle_id.as_str())
                .map_err(|error| eyre!(error))?,
            waterui_path: plan.waterui_path.clone(),
            channel: plan.channel,
            framework_manifest: plan.framework_manifest.clone(),
            framework: None,
            framework_lock: None,
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
    // The channel resolution is the one fact of a `create` a user cannot
    // see in the files it wrote without opening Water.toml.
    if let Some(framework) = &draft.project().manifest().framework {
        success!(shell, "Resolved framework: {framework}");
    }
    Ok(draft)
}

async fn scaffold_web_frontend(shell: &Shell, plan: &CreatePlan, project: &Project) -> Result<()> {
    super::web::create_vite(
        project.host(),
        shell,
        project.root(),
        "web",
        plan.package_manager,
        plan.vite_template.as_deref(),
    )
    .await?;
    super::web::brand_overlay(shell, &project.root().join("web"), &plan.name)?;
    super::web::install_dependencies(
        project.host(),
        shell,
        plan.package_manager,
        &project.root().join("web"),
    )
    .await
}

fn print_create_summary(shell: &Shell, plan: &CreatePlan) {
    line!(shell);
    success!(shell, "Project created at {}", plan.project_path.display());
    line!(shell);
    line!(shell, "Next steps:");
    line!(shell, "  cd {}", plan.folder_name);
    if let Some(command) = next_run_command() {
        line!(shell, "  {command}");
    }
}

fn prompt_name() -> Result<String> {
    Ok(Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Project name")
        .interact_text()?)
}

/// The bundle identifier a new project gets when `--bundle-id` is not given:
/// `dev.waterui.<name>` in lower camel case — the one casing valid on every
/// supported platform. An error means the project name cannot derive one.
fn default_bundle_id(app_name: &str) -> Result<String, String> {
    default_bundle_identifier(app_name).map(|identifier| identifier.to_string())
}

fn prompt_bundle_id(app_name: &str) -> Result<String> {
    let theme = ColorfulTheme::default();
    let mut input = Input::<String>::with_theme(&theme).with_prompt("Bundle identifier");
    if let Ok(default) = default_bundle_id(app_name) {
        input = input.default(default);
    }
    Ok(input.interact_text()?)
}

/// The command that runs a fresh project on this host, when the host is a
/// desktop platform the CLI runs apps on.
const fn next_run_command() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("water run --platform macos")
    } else if cfg!(target_os = "linux") {
        Some("water run --platform linux")
    } else if cfg!(target_os = "windows") {
        Some("water run --platform windows")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, FrameworkChannel, Host, resolve_create_plan};
    use crate::shell::Shell;
    use clap::Parser;
    use std::path::{Path, PathBuf};
    /// Scaffolds a `WaterUI` project plus a `create vite` frontend into `root`
    /// against the framework checkout at `waterui_checkout`, applies the brand
    /// overlay, and installs dependencies — the same steps `run()` performs
    /// for `--template web`.
    async fn scaffold_web_project(
        root: &std::path::Path,
        waterui_checkout: &std::path::Path,
        name: &str,
        vite_template: &str,
    ) {
        let shell = Shell::new(false);
        let project = waterui_cli::project::Project::create(
            &waterui_cli::toolchain::Host::current(),
            root,
            waterui_cli::project::CreateOptions {
                name: name.to_string(),
                bundle_identifier: waterui_cli::project_types::BundleIdentifier::try_from(
                    "dev.waterui.webapp",
                )
                .expect("bundle identifier"),
                waterui_path: Some(waterui_checkout.to_path_buf()),
                channel: None,
                framework_manifest: None,
                framework: None,
                framework_lock: None,
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
            project.host(),
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
            project.host(),
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
    /// the frontend, and `cargo check`s the result against the pinned
    /// framework revision, cloned on demand. `bun` must be on `PATH`; the
    /// nightly job installs it.
    #[test]
    #[ignore = "builds the generated crate against the enclosing workspace"]
    fn create_template_web_scaffolds_and_checks() {
        smol::block_on(async {
            let checkout = crate::pinned_framework::checkout();
            let temp = tempfile::tempdir().expect("tempdir");
            let project_path = temp.path().join("web-app");
            scaffold_web_project(&project_path, &checkout, "WaterUI App", "vanilla-ts").await;

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
                lib_rs.contains("include_web!(resources, \"web\")")
                    && lib_rs.contains("use_env(|env: Environment|")
                    && lib_rs.contains("ResourceContext::from_environment(&env)"),
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
            let status = waterui_cli::toolchain::Host::current()
                .command("bun")
                .args(["run", "build"])
                .current_dir(project_path.join("web"))
                .status()
                .await
                .expect("bun run build runs");
            assert!(status.success(), "the branded frontend must build");

            // The scaffold's `cargo check` compiles the pinned framework into
            // a scratch target dir; pointing it under `target/` lets a nextest
            // retry — and the next cached CI run — resume instead of
            // restarting a cold graph every attempt.
            let status = waterui_cli::toolchain::Host::current()
                .command("cargo")
                .arg("check")
                .current_dir(&project_path)
                .env(
                    "CARGO_TARGET_DIR",
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-fixtures/cargo-target"),
                )
                .status()
                .await
                .expect("cargo check runs");
            assert!(status.success(), "the generated project must check");
        });
    }

    /// The React overlay compiles: `react-ts` scaffolds get a branded
    /// `App.tsx` that `tsc -b && vite build` accepts. `bun` must be on
    /// `PATH`; the nightly job installs it.
    #[test]
    #[ignore = "builds the generated crate against the enclosing workspace"]
    fn create_template_web_react_frontend_builds() {
        smol::block_on(async {
            let checkout = crate::pinned_framework::checkout();
            let temp = tempfile::tempdir().expect("tempdir");
            let project_path = temp.path().join("web-app");
            scaffold_web_project(&project_path, &checkout, "WaterUI App", "react-ts").await;

            let app_tsx = project_path.join("web/src/App.tsx");
            assert!(app_tsx.is_file(), "react-ts scaffolds src/App.tsx");
            let contents = std::fs::read_to_string(&app_tsx).expect("App.tsx");
            assert!(contents.contains("WaterUI + React"), "{contents}");
            assert!(project_path.join("web/public/waterui.svg").is_file());

            let status = waterui_cli::toolchain::Host::current()
                .command("bun")
                .args(["run", "build"])
                .current_dir(project_path.join("web"))
                .status()
                .await
                .expect("bun run build runs");
            assert!(status.success(), "the branded React frontend must build");
        });
    }

    #[derive(Parser)]
    struct CreateCommand {
        #[command(flatten)]
        args: Args,
    }

    /// `water create` derives one identifier every platform accepts —
    /// `to_snake_case` produced underscores `CFBundleIdentifier` rejects, so
    /// the default renders the display name in lower camel case instead. An
    /// explicit `--bundle-id` is preserved verbatim, never rewritten.
    #[test]
    fn default_bundle_id_is_platform_neutral() {
        let shell = Shell::new(true);
        let host = Host::current();
        for name in ["Menu Example", "menu-example", "menu_example"] {
            let command = CreateCommand::try_parse_from(["water", name]).expect("args");
            let plan = resolve_create_plan(&shell, &host, &command.args).expect("create plan");
            assert_eq!(plan.bundle_id, "dev.waterui.menuExample");
        }
        let command = CreateCommand::try_parse_from([
            "water",
            "Example",
            "--bundle-id",
            "com.example.my-app",
        ])
        .expect("args");
        let plan = resolve_create_plan(&shell, &host, &command.args).expect("create plan");
        assert_eq!(plan.bundle_id, "com.example.my-app");
    }

    /// The project lands under the host's declared working directory: a
    /// host carrying another cwd re-roots `create` where the process
    /// working directory cannot.
    #[test]
    fn project_path_is_rooted_at_the_host_working_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let host =
            Host::new(Vec::<PathBuf>::new(), Vec::<(&str, &str)>::new()).with_cwd(temp.path());
        let command = CreateCommand::try_parse_from(["water", "My App"]).expect("args");
        let plan =
            resolve_create_plan(&Shell::new(true), &host, &command.args).expect("create plan");
        assert_eq!(plan.project_path, temp.path().join("my-app"));
    }

    /// A name that cannot derive an identifier every platform accepts —
    /// here one whose identifier segment would lead with a digit — fails at
    /// plan resolution naming the rejecting grammar, never a silently
    /// rewritten identifier.
    #[test]
    fn an_underivable_default_bundle_id_is_an_actionable_error() {
        let command = CreateCommand::try_parse_from(["water", "3D Printer"]).expect("args");
        let Err(error) = resolve_create_plan(&Shell::new(true), &Host::current(), &command.args)
        else {
            panic!("an underivable default must be rejected")
        };
        assert!(format!("{error:#}").contains("Android"), "{error:#}");
    }

    #[test]
    fn framework_selection_never_implicitly_uses_a_local_checkout() {
        let shell = Shell::new(true);
        let host = Host::current();
        for channel in ["dev", "nightly", "stable"] {
            let command = CreateCommand::try_parse_from(["water", "Example", "--channel", channel])
                .expect("channel arguments");
            let plan = resolve_create_plan(&shell, &host, &command.args).expect("create plan");
            assert_eq!(
                plan.channel,
                Some(channel.parse::<FrameworkChannel>().expect("channel"))
            );
            assert_eq!(plan.waterui_path, None);
        }
        let command = CreateCommand::try_parse_from(["water", "Example", "--waterui-path", ".."])
            .expect("local source arguments");
        let plan = resolve_create_plan(&shell, &host, &command.args).expect("local create plan");
        assert_eq!(plan.waterui_path, Some(PathBuf::from("..")));
        assert_eq!(plan.channel, None);
        // The removed package types left no `--mode` flag behind.
        assert!(CreateCommand::try_parse_from(["water", "Example", "--mode", "app"]).is_err());
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
}

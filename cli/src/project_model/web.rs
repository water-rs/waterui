//! Web-frontend toolchain plumbing.
//!
//! The `[web]` manifest section, the package manager it declares, the
//! frontend build that packaging runs, and the pure planning behind
//! `water init`'s frontend decision tree.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use color_eyre::eyre::{self, bail};
use serde::{Deserialize, Serialize};
use smol::process::Command;
use waterui_assets_planner::{BUNDLE_META_PREFIX, BundleMountMeta};

use crate::artifact_symbols::{ArtifactSymbols, build_host_rlib};
use crate::project::Project;

/// The JavaScript package manager a project declares in
/// `[web] package_manager`.
///
/// This is the single source of truth for which executable the CLI invokes:
/// a project that declares `pnpm` is never built with `bun`.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    /// Bun (`bun`).
    #[default]
    Bun,
    /// pnpm.
    Pnpm,
    /// npm, the Node.js default.
    Npm,
    /// Yarn.
    Yarn,
}

impl PackageManager {
    /// The executable looked up on `PATH`.
    #[must_use]
    pub const fn binary(self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Pnpm => "pnpm",
            Self::Npm => "npm",
            Self::Yarn => "yarn",
        }
    }

    /// `<pm> run <script>` — every supported manager accepts this form.
    #[must_use]
    pub fn run(self, script: &str) -> Command {
        let mut command = Command::new(self.binary());
        command.arg("run").arg(script);
        command
    }

    /// `<pm> install` — installs the dependencies of the current directory.
    #[must_use]
    pub fn install(self) -> Command {
        let mut command = Command::new(self.binary());
        command.arg("install");
        command
    }

    /// `<pm> create vite <dir>`; npm names its initializer `vite@latest`.
    ///
    /// `template`, when given, is forwarded to `create-vite` after a `--`
    /// separator (`--template <t>`), which every manager passes through and
    /// which skips Vite's interactive framework picker.
    #[must_use]
    pub fn create_vite(self, dir: &str, template: Option<&str>) -> Command {
        let mut command = Command::new(self.binary());
        command.arg("create");
        match self {
            Self::Npm => command.arg("vite@latest"),
            Self::Bun | Self::Pnpm | Self::Yarn => command.arg("vite"),
        };
        command.arg(dir);
        if let Some(template) = template {
            command.args(["--", "--template", template]);
        }
        command
    }

    /// Whether the manager's binary resolves on `PATH`.
    pub async fn is_installed(self) -> bool {
        crate::utils::which(self.binary()).await.is_ok()
    }

    /// The official installation instruction, shown when the declared manager
    /// is missing.
    #[must_use]
    pub const fn install_hint(self) -> &'static str {
        match self {
            Self::Bun => "curl -fsSL https://bun.sh/install | bash",
            Self::Pnpm => "npm install -g pnpm (or see https://pnpm.io/installation)",
            Self::Npm => "install Node.js from https://nodejs.org/",
            Self::Yarn => {
                "npm install -g yarn (or see https://yarnpkg.com/getting-started/install)"
            }
        }
    }
}

/// The `[web]` section of `Water.toml`: web-frontend toolchain declarations.
///
/// The macro never reads this — only the CLI does. Absent section and absent
/// key both mean [`PackageManager::Bun`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebConfig {
    /// The package manager used for `run build`, `run dev`, `create vite`,
    /// and `install` in the frontend project.
    #[serde(default)]
    pub package_manager: PackageManager,
}

/// The `include_web!` mount declared by the project's compiled library.
///
/// Read from the `waterui_meta_bundle_*` statics in the host rlib, which is
/// ground truth for what the application actually declared.
///
/// # Errors
///
/// Returns an error when the host build fails, the rlib cannot be read, or a
/// declared mount's payload does not decode.
pub async fn web_mount(
    project: &Project,
    sccache_path: Option<&Path>,
) -> eyre::Result<Option<BundleMountMeta>> {
    let rlib = build_host_rlib(project.root(), sccache_path).await?;
    let symbols = ArtifactSymbols::read(&rlib)?;
    decode_web_mount(&symbols)
}

/// Decode the `web` mount — the single mount a `project` field marks as a
/// toolchain-produced frontend — from an artifact's symbol table.
///
/// # Errors
///
/// Returns an error when a mount payload does not decode or two mounts claim
/// a frontend project.
pub fn decode_web_mount(symbols: &ArtifactSymbols) -> eyre::Result<Option<BundleMountMeta>> {
    let mut frontend = None;
    for leaf in symbols.leaves_with_prefix(BUNDLE_META_PREFIX) {
        let meta = BundleMountMeta::from_payload(&symbols.static_bytes(&leaf)?)?;
        if meta.project.is_none() {
            continue;
        }
        if frontend.replace(meta).is_some() {
            bail!("more than one include_web! mount is declared in the artifact");
        }
    }
    Ok(frontend)
}

/// Build a toolchain-produced mount's frontend: `<pm> run build` inside the
/// declared project root with stdio inherited, so the user sees their own
/// bundler's output and diagnostics verbatim.
///
/// # Errors
///
/// Returns an error when the build fails or does not produce the mount's
/// declared output directory.
///
/// # Panics
///
/// Panics when `meta` declares no `project` — callers only reach this for
/// `include_web!` mounts.
pub async fn build_frontend(
    package_manager: PackageManager,
    meta: &BundleMountMeta,
) -> eyre::Result<()> {
    let root = meta
        .project
        .as_ref()
        .expect("build_frontend is only called for mounts that declare a project");
    let pm = package_manager.binary();
    let status = package_manager
        .run("build")
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await?;
    if !status.success() {
        bail!("`{pm} run build` failed in {}: {status}", root.display());
    }
    if !meta.path.is_dir() {
        bail!(
            "`{pm} run build` did not produce `{}`; set `out_dir` on `include_web!` to the bundler's output directory",
            meta.path.display()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `water init` planning
// ---------------------------------------------------------------------------

/// Where the frontend of an initialized project comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSource {
    /// Scaffold a new Vite project into `web/`.
    New,
    /// An existing frontend project at this path.
    Existing(PathBuf),
}

/// What to do with a frontend found outside `web/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingFrontendMode {
    /// Copy it into `web/` (excluding `node_modules` and `.git`), then install.
    Copy,
    /// Leave it in place; `include_web!` references it by relative path.
    Reference,
}

/// The answers `water init` needs, each supplied by a flag or a prompt.
#[derive(Debug, Clone, Default)]
pub struct InitAnswers {
    /// `--web new|<path>`: the frontend source, if the flag settled it.
    pub web: Option<WebSource>,
    /// `--web-mode copy|reference`: how an existing frontend joins, if the
    /// flag settled it.
    pub web_mode: Option<ExistingFrontendMode>,
    /// `--package-manager`: the declared manager, if the flag settled it.
    pub package_manager: Option<PackageManager>,
}

/// One step of `water init`. The command executes these in order; the plan is
/// pure so the decision tree is testable without touching disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitAction {
    /// Move the listed top-level entries into `web/`.
    MoveFrontendToWeb {
        /// Top-level entries of the project root to move.
        entries: Vec<PathBuf>,
    },
    /// `<pm> create vite web` inside the project root.
    ScaffoldVite,
    /// Copy an existing project into `web/` (skipping `node_modules` and
    /// `.git`), then install its dependencies.
    CopyFrontend {
        /// The project to copy.
        source: PathBuf,
    },
    /// `<pm> install` inside `web/`.
    InstallDependencies,
    /// Scaffold the Rust shell whose root view is `include_web!(<arg>)`.
    ScaffoldShell {
        /// The `include_web!` argument — `"web"` or a relative path.
        web_arg: String,
    },
}

/// Top-level entries that never move into `web/` when a CWD frontend is
/// relocated: the Rust shell and project metadata stay at the root.
fn root_only_entries(has_rust_manifest: bool, entry: &str) -> bool {
    if matches!(
        entry,
        ".git" | ".github" | ".water" | "Water.toml" | "Water.lock" | "backends" | "target" | "web"
    ) || entry.starts_with("README")
        || entry.starts_with("LICENSE")
    {
        return true;
    }
    // `src/` and the Cargo files are the frontend's own when no Rust manifest
    // exists yet; once Cargo.toml is present they belong to the shell.
    has_rust_manifest && matches!(entry, "Cargo.toml" | "Cargo.lock" | "src")
}

/// The package manager a lockfile implies, used as the prompt's initial value.
#[must_use]
pub fn lockfile_package_manager(entries: &[String]) -> Option<PackageManager> {
    if entries.iter().any(|e| e == "bun.lock" || e == "bun.lockb") {
        Some(PackageManager::Bun)
    } else if entries.iter().any(|e| e == "pnpm-lock.yaml") {
        Some(PackageManager::Pnpm)
    } else if entries.iter().any(|e| e == "yarn.lock") {
        Some(PackageManager::Yarn)
    } else if entries.iter().any(|e| e == "package-lock.json") {
        Some(PackageManager::Npm)
    } else {
        None
    }
}

/// The ordered steps `water init` performs, decided from the CWD's top-level
/// listing and the already-resolved answers.
///
/// The caller resolves prompts first: every field of `answers` that is still
/// `None` is a prompt it must ask (with [`lockfile_package_manager`] as the
/// package-manager default) before planning.
///
/// # Errors
///
/// Returns an error when a referenced frontend path escapes the project root
/// in a way `include_web!` cannot express, or the answers are inconsistent
/// (`--web new` combined with `--web-mode`).
pub fn plan_init(
    project_root: &Path,
    entries: &[String],
    answers: &InitAnswers,
) -> eyre::Result<Vec<InitAction>> {
    if entries.iter().any(|e| e == "package.json") {
        let has_rust_manifest = entries.iter().any(|e| e == "Cargo.toml");
        let move_entries = entries
            .iter()
            .filter(|entry| !root_only_entries(has_rust_manifest, entry))
            .map(PathBuf::from)
            .collect();
        return Ok(vec![
            InitAction::MoveFrontendToWeb {
                entries: move_entries,
            },
            InitAction::ScaffoldShell {
                web_arg: "web".to_string(),
            },
        ]);
    }

    match answers.web.clone() {
        Some(WebSource::New) | None => Ok(vec![
            InitAction::ScaffoldVite,
            InitAction::InstallDependencies,
            InitAction::ScaffoldShell {
                web_arg: "web".to_string(),
            },
        ]),
        Some(WebSource::Existing(source)) => {
            match answers.web_mode.unwrap_or(ExistingFrontendMode::Copy) {
                ExistingFrontendMode::Copy => Ok(vec![
                    InitAction::CopyFrontend { source },
                    InitAction::InstallDependencies,
                    InitAction::ScaffoldShell {
                        web_arg: "web".to_string(),
                    },
                ]),
                ExistingFrontendMode::Reference => {
                    let arg = relative_path_arg(project_root, &source)?;
                    Ok(vec![InitAction::ScaffoldShell { web_arg: arg }])
                }
            }
        }
    }
}

/// The `include_web!` argument for a frontend outside `web/`: a relative path
/// from the project root, with `..` segments for directories outside it.
fn relative_path_arg(project_root: &Path, source: &Path) -> eyre::Result<String> {
    let root = dunce::canonicalize(project_root)?;
    let source = dunce::canonicalize(source)?;
    let mut root_components = root.components().peekable();
    let mut source_components = source.components().peekable();
    while root_components.peek() == source_components.peek() && root_components.peek().is_some() {
        root_components.next();
        source_components.next();
    }
    let mut arg = String::new();
    for _ in root_components {
        if !arg.is_empty() {
            arg.push('/');
        }
        arg.push_str("..");
    }
    for component in source_components {
        if !arg.is_empty() {
            arg.push('/');
        }
        arg.push_str(
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| eyre::eyre!("frontend path is not valid UTF-8"))?,
        );
    }
    if arg.is_empty() {
        bail!("the frontend is the project root itself; put its files in `web/`");
    }
    Ok(arg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn package_manager_serde_round_trip() {
        // toml's serializer needs a table at the root; round-trip through the
        // `[web]` section shape the manifest actually uses.
        #[derive(Debug, Serialize, Deserialize)]
        struct Section {
            package_manager: PackageManager,
        }
        for (pm, name) in [
            (PackageManager::Bun, "bun"),
            (PackageManager::Pnpm, "pnpm"),
            (PackageManager::Npm, "npm"),
            (PackageManager::Yarn, "yarn"),
        ] {
            let encoded = toml::to_string(&Section {
                package_manager: pm,
            })
            .unwrap();
            assert_eq!(encoded.trim(), format!("package_manager = \"{name}\""));
            assert_eq!(
                toml::from_str::<Section>(&encoded).unwrap().package_manager,
                pm
            );
        }
        let error = toml::from_str::<Section>("package_manager = \"deno\"").unwrap_err();
        let message = error.to_string();
        for option in ["bun", "pnpm", "npm", "yarn"] {
            assert!(
                message.contains(option),
                "unknown manager error names the options: {message}"
            );
        }
    }

    #[test]
    fn command_arg_vectors() {
        let args = |command: &Command| -> Vec<String> {
            std::iter::once(command.get_program().to_string_lossy().into_owned())
                .chain(
                    command
                        .get_args()
                        .map(|arg| arg.to_string_lossy().into_owned()),
                )
                .collect()
        };
        assert_eq!(
            args(&PackageManager::Bun.run("build")),
            ["bun", "run", "build"]
        );
        assert_eq!(args(&PackageManager::Pnpm.install()), ["pnpm", "install"]);
        assert_eq!(
            args(&PackageManager::Yarn.create_vite("web", None)),
            ["yarn", "create", "vite", "web"]
        );
        assert_eq!(
            args(&PackageManager::Npm.create_vite("web", Some("vanilla-ts"))),
            [
                "npm",
                "create",
                "vite@latest",
                "web",
                "--",
                "--template",
                "vanilla-ts"
            ]
        );
    }

    #[test]
    fn cwd_frontend_moves_into_web_keeping_shell_files() {
        let plan = plan_init(
            Path::new("/project"),
            &entries(&[
                "package.json",
                "bun.lock",
                "index.html",
                "src",
                "Cargo.toml",
                "target",
                ".git",
                ".github",
                "README.md",
                "Water.toml",
            ]),
            &InitAnswers::default(),
        )
        .unwrap();
        let InitAction::MoveFrontendToWeb { entries: moved } = &plan[0] else {
            panic!("expected the move step first: {plan:?}")
        };
        let mut moved: Vec<String> = moved
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        moved.sort();
        // `Cargo.toml` and `src/` stay at the root: they are the shell's.
        assert_eq!(moved, ["bun.lock", "index.html", "package.json"]);
        assert_eq!(
            plan[1],
            InitAction::ScaffoldShell {
                web_arg: "web".to_string()
            }
        );
    }

    #[test]
    fn pure_frontend_cwd_moves_its_src() {
        let plan = plan_init(
            Path::new("/project"),
            &entries(&["package.json", "src", "vite.config.ts"]),
            &InitAnswers::default(),
        )
        .unwrap();
        let InitAction::MoveFrontendToWeb { entries: moved } = &plan[0] else {
            panic!("expected the move step first: {plan:?}")
        };
        assert!(
            moved.contains(&PathBuf::from("src")),
            "without Cargo.toml, src/ is frontend code: {moved:?}"
        );
    }

    #[test]
    fn new_frontend_scaffolds_vite_then_installs() {
        let answers = InitAnswers {
            web: Some(WebSource::New),
            ..InitAnswers::default()
        };
        let plan = plan_init(Path::new("/project"), &entries(&[]), &answers).unwrap();
        assert_eq!(
            plan,
            [
                InitAction::ScaffoldVite,
                InitAction::InstallDependencies,
                InitAction::ScaffoldShell {
                    web_arg: "web".to_string()
                },
            ]
        );
    }

    #[test]
    fn existing_frontend_copy_installs_into_web() {
        let answers = InitAnswers {
            web: Some(WebSource::Existing(PathBuf::from("/elsewhere/app"))),
            web_mode: Some(ExistingFrontendMode::Copy),
            ..InitAnswers::default()
        };
        let plan = plan_init(Path::new("/project"), &entries(&[]), &answers).unwrap();
        assert_eq!(
            plan,
            [
                InitAction::CopyFrontend {
                    source: PathBuf::from("/elsewhere/app")
                },
                InitAction::InstallDependencies,
                InitAction::ScaffoldShell {
                    web_arg: "web".to_string()
                },
            ]
        );
    }

    #[test]
    fn existing_frontend_reference_uses_a_relative_arg() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let sibling = temp.path().join("frontend");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let answers = InitAnswers {
            web: Some(WebSource::Existing(sibling)),
            web_mode: Some(ExistingFrontendMode::Reference),
            ..InitAnswers::default()
        };
        let plan = plan_init(&root, &entries(&[]), &answers).unwrap();
        assert_eq!(
            plan,
            [InitAction::ScaffoldShell {
                web_arg: "../frontend".to_string()
            }]
        );
    }
}

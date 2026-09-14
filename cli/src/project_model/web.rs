//! Web-frontend toolchain plumbing.
//!
//! The `[web]` manifest section, the package manager it declares, the
//! frontend build that packaging runs, and the pure planning behind
//! `water init`'s frontend decision tree.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use askama::Template;
use color_eyre::eyre::{self, bail};
use serde::{Deserialize, Serialize};
use smol::process::Command;
use waterui_assets_planner::{BUNDLE_META_PREFIX, BundleMountMeta};

use crate::artifact_symbols::{ArtifactSymbols, build_host_rlib};
use crate::project::Project;
use crate::project_model::templates::embedded;

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
            // npm needs `--` to forward args to the initializer; bun, pnpm
            // and yarn pass them through directly.
            if self == Self::Npm {
                command.arg("--");
            }
            command.args(["--template", template]);
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

// ---------------------------------------------------------------------------
// Branded starter overlay
// ---------------------------------------------------------------------------

/// The framework a freshly scaffolded Vite project declares.
///
/// Read from `web/package.json` dependencies — the dependencies the template
/// ships are the one ground truth `create vite` gives us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebFramework {
    /// `vanilla`/`vanilla-ts`: no framework dependency.
    Vanilla,
    /// `react` in the dependencies.
    React,
    /// `preact` in the dependencies.
    Preact,
    /// `vue` in the dependencies.
    Vue,
    /// `svelte` — a devDependency in Vite's starter.
    Svelte,
    /// `solid-js` in the dependencies.
    Solid,
    /// `lit` in the dependencies.
    Lit,
    /// Dependencies declare something we do not know — e.g. Qwik.
    Other,
}

impl WebFramework {
    /// The name the branded page prints.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Vanilla => "Vanilla",
            Self::React => "React",
            Self::Preact => "Preact",
            Self::Vue => "Vue",
            Self::Svelte => "Svelte",
            Self::Solid => "Solid",
            Self::Lit => "Lit",
            Self::Other => "web",
        }
    }

    /// Whether the overlay knows how to replace the starter's page.
    const fn supports_branding(self) -> bool {
        matches!(self, Self::Vanilla | Self::React | Self::Vue | Self::Svelte)
    }
}

/// What `web/package.json` says about a scaffolded frontend: the framework
/// dependency and whether `typescript` is a devDependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebFrontend {
    /// The framework the starter's dependencies declare.
    pub framework: WebFramework,
    /// Whether `typescript` is a devDependency (else JavaScript).
    pub typescript: bool,
}

/// The dependency names that identify each framework, checked in order.
const FRAMEWORK_DEPENDENCIES: &[(&str, WebFramework)] = &[
    ("react", WebFramework::React),
    ("preact", WebFramework::Preact),
    ("vue", WebFramework::Vue),
    ("svelte", WebFramework::Svelte),
    ("solid-js", WebFramework::Solid),
    ("lit", WebFramework::Lit),
];

/// Read the framework and language of a scaffolded frontend from its
/// `package.json` text. `None` when the manifest cannot be parsed.
///
/// Framework markers are looked up in both `dependencies` and
/// `devDependencies` — Vite's Svelte starter compiles the framework away and
/// declares it as a devDependency. `typescript` counts only in
/// `devDependencies`. A manifest with no `dependencies` at all is Vanilla;
/// one whose dependencies name none of the known frameworks is
/// [`WebFramework::Other`].
#[must_use]
pub fn detect_web_frontend(package_json: &str) -> Option<WebFrontend> {
    let package: serde_json::Value = serde_json::from_str(package_json).ok()?;
    let dependencies = package
        .get("dependencies")
        .and_then(serde_json::Value::as_object);
    let dev_dependencies = package
        .get("devDependencies")
        .and_then(serde_json::Value::as_object);
    let has_marker = |name: &str| {
        dependencies.is_some_and(|deps| deps.contains_key(name))
            || dev_dependencies.is_some_and(|deps| deps.contains_key(name))
    };
    let framework = FRAMEWORK_DEPENDENCIES
        .iter()
        .find(|(name, _)| has_marker(name))
        .map_or_else(
            || {
                if dependencies.is_none_or(serde_json::Map::is_empty) {
                    WebFramework::Vanilla
                } else {
                    WebFramework::Other
                }
            },
            |(_, framework)| *framework,
        );
    let typescript = dev_dependencies.is_some_and(|deps| deps.contains_key("typescript"));
    Some(WebFrontend {
        framework,
        typescript,
    })
}

/// What [`apply_brand_overlay`] did, and what it could not.
#[derive(Debug, Default)]
pub struct WebOverlayReport {
    /// The detected frontend (`None` when `package.json` did not parse).
    pub frontend: Option<WebFrontend>,
    /// Whether the starter's visible page was replaced with the branded one —
    /// `false` means the framework's own starter page remains.
    pub branded: bool,
    /// Non-fatal misses: expected files the layout did not ship.
    pub warnings: Vec<String>,
}

/// Values the branded starter templates interpolate.
struct WebOverlayContext<'a> {
    /// The name the heading prints — "React", or "TypeScript" for a vanilla
    /// starter.
    framework: &'a str,
    /// The file the "edit and save" line names — "src/App.tsx".
    entry: &'a str,
    /// The import specifier the framework logo resolves to — the starter's
    /// own logo file, e.g. `./assets/typescript.svg`.
    logo: &'a str,
    /// Whether the project is TypeScript; selects `lang="ts"` in SFCs and the
    /// typed bridge signature.
    typescript: bool,
}

macro_rules! web_overlay_templates {
    ($($name:ident => $path:literal),* $(,)?) => {$(
        #[derive(Template)]
        #[template(path = $path, escape = "none")]
        struct $name<'a> {
            ctx: &'a WebOverlayContext<'a>,
        }
    )*};
}

web_overlay_templates! {
    VanillaMainTsTemplate => "src/templates/web/vanilla/main.ts.tpl",
    VanillaMainJsTemplate => "src/templates/web/vanilla/main.js.tpl",
    ReactAppTsxTemplate => "src/templates/web/react/App.tsx.tpl",
    ReactAppJsxTemplate => "src/templates/web/react/App.jsx.tpl",
    VueAppTemplate => "src/templates/web/vue/App.vue.tpl",
    SvelteAppTemplate => "src/templates/web/svelte/App.svelte.tpl",
}

/// One entry-file template the overlay can render.
#[derive(Clone, Copy)]
enum OverlayTemplate {
    VanillaTs,
    VanillaJs,
    ReactTsx,
    ReactJsx,
    Vue,
    Svelte,
}

impl OverlayTemplate {
    fn render(self, ctx: &WebOverlayContext) -> io::Result<String> {
        let rendered = match self {
            Self::VanillaTs => VanillaMainTsTemplate { ctx }.render(),
            Self::VanillaJs => VanillaMainJsTemplate { ctx }.render(),
            Self::ReactTsx => ReactAppTsxTemplate { ctx }.render(),
            Self::ReactJsx => ReactAppJsxTemplate { ctx }.render(),
            Self::Vue => VueAppTemplate { ctx }.render(),
            Self::Svelte => SvelteAppTemplate { ctx }.render(),
        };
        rendered.map_err(|error| {
            io::Error::other(format!("web overlay template render failed: {error}"))
        })
    }
}

/// A destination the overlay may fill with a rendered entry file, probed in
/// order — `src/main.ts` first, `src/main.js` when the project is JavaScript.
struct EntryCandidate {
    /// The destination path inside `web/`; also the path the page's "edit and
    /// save" line names.
    dest: &'static str,
    /// The template that renders the file.
    template: OverlayTemplate,
    /// Candidate paths of the framework logo the branded page shows beside
    /// the `WaterUI` mark — the starter moved it between Vite versions.
    logos: &'static [&'static str],
}

/// The branded-overlay layout of one supported framework.
struct OverlaySpec {
    /// Entry-file candidates in probe order.
    entries: &'static [EntryCandidate],
    /// Stylesheets that receive the branded stylesheet verbatim.
    styles: &'static [&'static str],
    /// The starter's second, global stylesheet receiving the small baseline
    /// (React's `index.css`; every other supported framework has only the one
    /// branded stylesheet).
    base_style: Option<&'static str>,
    /// Starter files the overlay removes, grouped so several candidates count
    /// as one expected file; a group matching nothing warns.
    deletions: &'static [&'static [&'static str]],
}

const fn overlay_spec(framework: WebFramework) -> Option<OverlaySpec> {
    Some(match framework {
        WebFramework::Vanilla => OverlaySpec {
            entries: &[
                EntryCandidate {
                    dest: "src/main.ts",
                    template: OverlayTemplate::VanillaTs,
                    logos: &["src/assets/typescript.svg", "src/typescript.svg"],
                },
                EntryCandidate {
                    dest: "src/main.js",
                    template: OverlayTemplate::VanillaJs,
                    logos: &["src/assets/javascript.svg", "src/javascript.svg"],
                },
            ],
            styles: &["src/style.css"],
            base_style: None,
            deletions: &[&["src/counter.ts", "src/counter.js"]],
        },
        WebFramework::React => OverlaySpec {
            entries: &[
                EntryCandidate {
                    dest: "src/App.tsx",
                    template: OverlayTemplate::ReactTsx,
                    logos: &["src/assets/react.svg", "src/react.svg"],
                },
                EntryCandidate {
                    dest: "src/App.jsx",
                    template: OverlayTemplate::ReactJsx,
                    logos: &["src/assets/react.svg", "src/react.svg"],
                },
            ],
            styles: &["src/App.css"],
            base_style: Some("src/index.css"),
            deletions: &[],
        },
        WebFramework::Vue => OverlaySpec {
            entries: &[EntryCandidate {
                dest: "src/App.vue",
                template: OverlayTemplate::Vue,
                logos: &["src/assets/vue.svg", "src/vue.svg"],
            }],
            styles: &["src/style.css"],
            base_style: None,
            deletions: &[&["src/components/HelloWorld.vue"]],
        },
        WebFramework::Svelte => OverlaySpec {
            entries: &[EntryCandidate {
                dest: "src/App.svelte",
                template: OverlayTemplate::Svelte,
                logos: &["src/assets/svelte.svg", "src/svelte.svg"],
            }],
            styles: &["src/app.css"],
            base_style: None,
            deletions: &[&["src/lib/Counter.svelte"]],
        },
        _ => return None,
    })
}

/// A verbatim asset the embedded `web/` template dir ships.
fn web_template_asset(relative: &str) -> &'static [u8] {
    embedded::ROOT
        .get_file(format!("web/{relative}"))
        .unwrap_or_else(|| panic!("web overlay asset `{relative}` must ship in the CLI"))
        .contents()
}

/// Apply the WaterUI-branded overlay to a freshly `create vite`-scaffolded
/// frontend in `web_dir`.
///
/// Every project — branded or not — gets `public/waterui.svg`, loses
/// `public/vite.svg`, and has its `index.html` retitled to `display_name`
/// with its favicon repointed to the `WaterUI` mark. The supported matrix
/// (Vanilla, React, Vue, Svelte — TypeScript or JavaScript) additionally gets
/// its starter page replaced; anything else keeps the framework's default
/// page and reports `branded: false`. Files an unexpected layout does not
/// ship are skipped with a warning, never an error.
///
/// # Errors
///
/// Returns an error only on real I/O failures writing the overlay.
///
/// # Panics
///
/// Panics when the embedded `web/` template assets are missing — they are
/// compiled into the CLI.
pub fn apply_brand_overlay(web_dir: &Path, display_name: &str) -> io::Result<WebOverlayReport> {
    let mut report = WebOverlayReport::default();

    let logo = embedded::ROOT
        .get_file("icon.svg")
        .expect("the WaterUI logo ships in the template bundle");
    write_overlay_file(web_dir, "public/waterui.svg", logo.contents())?;
    // `index.html` is repointed at `/waterui.svg` below, so the starter's own
    // favicons are orphaned — Vite 7 ships `vite.svg`, Vite 8 `favicon.svg`.
    for orphaned in ["public/vite.svg", "public/favicon.svg"] {
        let path = web_dir.join(orphaned);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
    }
    retitle_index_html(web_dir, display_name, &mut report.warnings)?;

    let Some(frontend) = read_frontend(web_dir, &mut report.warnings) else {
        return Ok(report);
    };
    report.frontend = Some(frontend);
    if frontend.framework.supports_branding() {
        report.branded = brand_framework_page(web_dir, frontend, &mut report.warnings)?;
    }
    Ok(report)
}

/// Write `contents` to `web_dir/relative`, creating parent directories.
fn write_overlay_file(web_dir: &Path, relative: &str, contents: &[u8]) -> io::Result<()> {
    let dest = web_dir.join(relative);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, contents)
}

/// Read the scaffolded frontend's framework and language; an unreadable or
/// unparsable `package.json` warns and yields `None`.
fn read_frontend(web_dir: &Path, warnings: &mut Vec<String>) -> Option<WebFrontend> {
    if let Ok(manifest) = std::fs::read_to_string(web_dir.join("package.json")) {
        detect_web_frontend(&manifest).or_else(|| {
            warnings.push(
                "web/package.json did not parse — the starter page was left in place".to_string(),
            );
            None
        })
    } else {
        warnings
            .push("web/package.json is missing — the starter page was left in place".to_string());
        None
    }
}

/// Replace the starter's visible page with the branded one. Returns `false`
/// when the layout is not recognized — the framework's page then stays.
fn brand_framework_page(
    web_dir: &Path,
    frontend: WebFrontend,
    warnings: &mut Vec<String>,
) -> io::Result<bool> {
    let Some(spec) = overlay_spec(frontend.framework) else {
        return Ok(false);
    };
    let Some(entry) = spec
        .entries
        .iter()
        .find(|candidate| web_dir.join(candidate.dest).is_file())
    else {
        warnings.push(format!(
            "{} is missing — the {} starter layout is not recognized; its default page remains",
            spec.entries[0].dest,
            frontend.framework.display_name(),
        ));
        return Ok(false);
    };

    // The heading pairs WaterUI with what the starter actually showcases —
    // the framework logo it ships. A vanilla starter's mark is the language
    // logo, so "TypeScript" reads truer than "Vanilla".
    let framework = if frontend.framework == WebFramework::Vanilla {
        if frontend.typescript {
            "TypeScript"
        } else {
            "JavaScript"
        }
    } else {
        frontend.framework.display_name()
    };
    // Every entry file lives in `src/`; the logo specifier is relative to it.
    // When the starter ships no logo the import points at the WaterUI mark we
    // just wrote to `public/` — an import the bundler still resolves.
    let logo = entry
        .logos
        .iter()
        .find(|logo| web_dir.join(logo).is_file())
        .map_or_else(
            || {
                warnings.push(format!(
                    "{} is missing — the branded page falls back to the WaterUI mark",
                    entry.logos[0]
                ));
                "../public/waterui.svg".to_string()
            },
            |logo| format!("./{}", logo.strip_prefix("src/").unwrap_or(logo)),
        );

    let ctx = WebOverlayContext {
        framework,
        entry: entry.dest,
        logo: &logo,
        typescript: frontend.typescript,
    };
    write_overlay_file(web_dir, entry.dest, entry.template.render(&ctx)?.as_bytes())?;

    for style in spec.styles {
        if web_dir.join(style).is_file() {
            write_overlay_file(web_dir, style, web_template_asset("brand.css"))?;
        } else {
            warnings.push(format!("{style} is missing — branded stylesheet skipped"));
        }
    }
    if let Some(base_style) = spec.base_style {
        if web_dir.join(base_style).is_file() {
            write_overlay_file(web_dir, base_style, web_template_asset("base.css"))?;
        } else {
            warnings.push(format!(
                "{base_style} is missing — baseline stylesheet skipped"
            ));
        }
    }
    for group in spec.deletions {
        let mut removed = false;
        for file in *group {
            let path = web_dir.join(file);
            if path.is_file() {
                std::fs::remove_file(path)?;
                removed = true;
            }
        }
        if !removed {
            warnings.push(format!("{} is missing — nothing to remove", group[0]));
        }
    }
    // The branded page references nothing else the starter put in `public/`;
    // `icons.svg` is its sprite sheet.
    let sprite = web_dir.join("public/icons.svg");
    if sprite.exists() {
        std::fs::remove_file(&sprite)?;
    }
    if frontend.typescript {
        write_overlay_file(
            web_dir,
            "src/waterui.d.ts",
            web_template_asset("waterui.d.ts"),
        )?;
    }
    Ok(true)
}

/// Retitle `index.html` to the app's display name and repoint its favicon to
/// `/waterui.svg`. The file is patched rather than replaced, so markup a
/// starter puts there survives.
fn retitle_index_html(
    web_dir: &Path,
    display_name: &str,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let path = web_dir.join("index.html");
    if !path.is_file() {
        warnings.push("index.html is missing — title and favicon unchanged".to_string());
        return Ok(());
    }
    let mut html = std::fs::read_to_string(&path)?;
    match (html.find("<title>"), html.find("</title>")) {
        (Some(start), Some(end)) if start + "<title>".len() <= end => {
            html.replace_range(
                start + "<title>".len()..end,
                &escape_html_text(display_name),
            );
        }
        _ => warnings.push("index.html has no <title> to retitle".to_string()),
    }
    // Repoint whichever favicon the starter links — `/vite.svg` on Vite 7,
    // `/favicon.svg` on Vite 8 — at the WaterUI mark.
    let mut repointed = false;
    for favicon in ["/favicon.svg", "./favicon.svg", "/vite.svg", "./vite.svg"] {
        let quoted = format!("\"{favicon}\"");
        if html.contains(&quoted) {
            html = html.replace(&quoted, "\"/waterui.svg\"");
            repointed = true;
        }
    }
    if !repointed {
        if let Some(head_end) = html.find("</head>") {
            html.insert_str(
                head_end,
                "    <link rel=\"icon\" type=\"image/svg+xml\" href=\"/waterui.svg\" />\n  ",
            );
        } else {
            warnings.push("index.html has no favicon link or </head> to repoint".to_string());
        }
    }
    std::fs::write(&path, html)
}

/// Escape the handful of characters that break a `<title>` text node.
fn escape_html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
            args(&PackageManager::Bun.create_vite("web", Some("react-ts"))),
            ["bun", "create", "vite", "web", "--template", "react-ts"]
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

    #[test]
    fn detect_frontend_reads_framework_and_language() {
        let assert = |manifest: &str, framework: WebFramework, typescript: bool| {
            assert_eq!(
                detect_web_frontend(manifest),
                Some(WebFrontend {
                    framework,
                    typescript
                }),
                "{manifest}"
            );
        };
        // vanilla-ts / vanilla ship no dependencies at all.
        assert(
            r#"{"devDependencies":{"typescript":"~5.9","vite":"^7"}}"#,
            WebFramework::Vanilla,
            true,
        );
        assert(
            r#"{"devDependencies":{"vite":"^7"}}"#,
            WebFramework::Vanilla,
            false,
        );
        assert(
            r#"{"dependencies":{"react":"^19","react-dom":"^19"},"devDependencies":{"typescript":"~5.9"}}"#,
            WebFramework::React,
            true,
        );
        assert(
            r#"{"dependencies":{"react":"^19","react-dom":"^19"}}"#,
            WebFramework::React,
            false,
        );
        assert(
            r#"{"dependencies":{"preact":"^10"},"devDependencies":{"typescript":"~5.9"}}"#,
            WebFramework::Preact,
            true,
        );
        assert(
            r#"{"dependencies":{"vue":"^3"},"devDependencies":{"typescript":"~5.9","vue-tsc":"^3"}}"#,
            WebFramework::Vue,
            true,
        );
        // Vite's Svelte starter declares the framework as a devDependency —
        // it compiles away.
        assert(
            r#"{"devDependencies":{"svelte":"^5","typescript":"~5.9"}}"#,
            WebFramework::Svelte,
            true,
        );
        assert(
            r#"{"dependencies":{"solid-js":"^1"}}"#,
            WebFramework::Solid,
            false,
        );
        assert(r#"{"dependencies":{"lit":"^3"}}"#, WebFramework::Lit, false);
        // A framework we do not know: dependencies exist but match nothing.
        assert(
            r#"{"dependencies":{"@qwik.dev/core":"^2"}}"#,
            WebFramework::Other,
            false,
        );
        assert!(detect_web_frontend("not json").is_none());
    }

    /// The layout `create vite --template vanilla-ts` produces on Vite 8.
    fn write_vanilla_layout(web: &Path) {
        std::fs::create_dir_all(web.join("public")).unwrap();
        std::fs::create_dir_all(web.join("src/assets")).unwrap();
        std::fs::write(
            web.join("package.json"),
            r#"{"devDependencies":{"typescript":"~6.0","vite":"^8"}}"#,
        )
        .unwrap();
        std::fs::write(
            web.join("index.html"),
            "<html><head><title>web</title>\
             <link rel=\"icon\" type=\"image/svg+xml\" href=\"/favicon.svg\" />\
             </head><body><div id=\"app\"></div></body></html>",
        )
        .unwrap();
        std::fs::write(web.join("public/favicon.svg"), "<svg/>").unwrap();
        std::fs::write(web.join("public/icons.svg"), "<svg/>").unwrap();
        std::fs::write(web.join("src/main.ts"), "// vite starter").unwrap();
        std::fs::write(web.join("src/counter.ts"), "// counter").unwrap();
        std::fs::write(web.join("src/style.css"), "/* vite */").unwrap();
        std::fs::write(web.join("src/assets/typescript.svg"), "<svg/>").unwrap();
    }

    #[test]
    fn overlay_brands_a_vanilla_layout() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        write_vanilla_layout(&web);

        let report = apply_brand_overlay(&web, "My App").unwrap();
        assert!(report.branded);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(
            report.frontend,
            Some(WebFrontend {
                framework: WebFramework::Vanilla,
                typescript: true
            })
        );

        assert!(web.join("public/waterui.svg").is_file());
        assert!(!web.join("public/favicon.svg").exists());
        assert!(!web.join("public/icons.svg").exists());
        assert!(!web.join("src/counter.ts").exists());
        assert!(web.join("src/waterui.d.ts").is_file());
        let main = std::fs::read_to_string(web.join("src/main.ts")).unwrap();
        assert!(main.contains("WaterUI + TypeScript"), "{main}");
        assert!(main.contains("'./assets/typescript.svg'"), "{main}");
        assert!(main.contains("invoke<string>('greet'"), "{main}");
        let html = std::fs::read_to_string(web.join("index.html")).unwrap();
        assert!(html.contains("<title>My App</title>"), "{html}");
        assert!(html.contains("\"/waterui.svg\""), "{html}");
        let style = std::fs::read_to_string(web.join("src/style.css")).unwrap();
        assert!(style.contains(".page"), "{style}");
    }

    #[test]
    fn overlay_skips_missing_files_with_warnings() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        std::fs::create_dir_all(&web).unwrap();
        // A react-ts manifest but no src/, no index.html: the overlay must
        // warn, not fail, and still ship the favicon.
        std::fs::write(
            web.join("package.json"),
            r#"{"dependencies":{"react":"^19"},"devDependencies":{"typescript":"~5.9"}}"#,
        )
        .unwrap();

        let report = apply_brand_overlay(&web, "App").unwrap();
        assert!(!report.branded);
        assert!(!report.warnings.is_empty());
        assert!(
            report.warnings.iter().any(|w| w.contains("src/App.tsx")),
            "{:?}",
            report.warnings
        );
        assert!(web.join("public/waterui.svg").is_file());
    }
}

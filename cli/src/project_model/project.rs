//! Project management and build utilities for `WaterUI` CLI.

use std::fmt::Write as _;

use cargo_toml::Manifest as CargoManifest;
use eyre::WrapErr as _;
use futures_util::future::{BoxFuture, Shared};
use futures_util::stream::{self, StreamExt as _};
use futures_util::{FutureExt as _, TryFutureExt as _};
use target_lexicon::Triple;
use tracing::info;

use crate::build::{BuildProgress, RustLinkage};
use crate::framework::{
    FrameworkChannel, ResolvedFramework, validate_local_cli, validate_resolved_cli,
};
use crate::toolchain::Host;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenMode {
    Full,
    PreviewBuild,
}

/// The managed native backends [`Project::open`] initialises.
///
/// A project delegates its Apple and Android projects to the CLI, which
/// scaffolds them into the build cache when the project is opened. Each
/// scaffold costs time and leaves a generated project behind, so a command
/// declares the platforms it is about to act on and only their backends are
/// initialised. The other managed backends (GTK4, hydrolysis, `WinUI`, ESP32)
/// are generated on demand by the command that runs them and are not part of
/// this selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ManagedBackends {
    apple: bool,
    android: bool,
}

impl ManagedBackends {
    /// No managed native backend.
    pub const NONE: Self = Self {
        apple: false,
        android: false,
    };

    /// Every managed native backend, for commands that act on all of them.
    pub const ALL: Self = Self {
        apple: true,
        android: true,
    };

    /// The backends `platform` builds with: Apple for the Apple platforms,
    /// Android for Android, none for the rest.
    #[must_use]
    pub const fn for_platform(platform: TargetPlatform) -> Self {
        Self {
            apple: crate::apple::platform::is_apple_platform(platform),
            android: crate::android::platform::is_android_platform(platform),
        }
    }

    /// The union of [`Self::for_platform`] over `platforms`.
    #[must_use]
    pub fn for_platforms(platforms: &[TargetPlatform]) -> Self {
        platforms.iter().fold(Self::NONE, |selected, platform| {
            selected.union(Self::for_platform(*platform))
        })
    }

    /// The managed backend `backend` itself is, if it is one: Apple or Android.
    #[must_use]
    pub const fn for_backend(backend: TargetBackend) -> Self {
        Self {
            apple: matches!(backend, TargetBackend::Apple),
            android: matches!(backend, TargetBackend::Android),
        }
    }

    #[must_use]
    const fn union(self, other: Self) -> Self {
        Self {
            apple: self.apple || other.apple,
            android: self.android || other.android,
        }
    }

    /// Whether the Apple backend is selected.
    #[must_use]
    pub const fn apple(self) -> bool {
        self.apple
    }

    /// Whether the Android backend is selected.
    #[must_use]
    pub const fn android(self) -> bool {
        self.android
    }
}

/// What `cargo metadata` reports about the tree a project builds in.
///
/// Resolved once per [`Project`] and shared by everything that needs it, so
/// one `cargo metadata` run serves the target directory, the lockfile, and
/// the package spec every `cargo tree` evaluation roots at.
#[derive(Debug, Clone)]
struct CargoLayout {
    target_dir: PathBuf,
    /// Root of the Cargo workspace the project belongs to — the project itself
    /// when it is not a workspace member. This is where its `Cargo.lock` lives.
    workspace_root: PathBuf,
    /// The application package's resolved id — the `--package` spec every
    /// `cargo tree` evaluation passes so the application crate, not the
    /// workspace root, roots the printed graph.
    root_package_id: String,
}

enum CargoResolution {
    Local,
    Locked,
    Update,
}

/// The per-triple cache one kind of `cargo tree` evaluation fills: a
/// shared in-flight future per build triple, so concurrent consumers of the
/// same target share one resolve and different targets never share an
/// answer.
type TargetGraph<T> =
    Arc<async_lock::Mutex<BTreeMap<String, Shared<BoxFuture<'static, Result<Arc<T>, String>>>>>>;

/// The triples the managed FFI companion compiles for — the Apple
/// platforms plus the Android ABIs — the serving set of its shared
/// `[profile]` package-override table.
fn ffi_companion_targets() -> Vec<Triple> {
    crate::platform::apple_target_triples()
        .into_iter()
        .chain(crate::android::platform::android_target_triples())
        .collect()
}

fn spawn_cargo_layout_resolution(
    host: &Host,
    current_dir: &Path,
    framework: Option<ResolvedFramework>,
    local: bool,
) -> Shared<BoxFuture<'static, Result<CargoLayout, String>>> {
    let host = host.clone();
    let current_dir = current_dir.to_path_buf();
    let mode = if local {
        CargoResolution::Local
    } else if framework.is_some() {
        CargoResolution::Locked
    } else {
        CargoResolution::Update
    };
    smol::spawn(async move {
        resolve_cargo_layout(&host, &current_dir, framework, mode)
            .await
            .map_err(|error| error.to_string())
    })
    .boxed()
    .shared()
}

/// The inputs every persisted `cargo tree` answer keys on — anything
/// that changes cfg or feature resolution for the same `--target` set:
/// the toolchain pair, every manifest Cargo reads (the project's own,
/// its workspace root's, the `.cargo/config.toml` chain), and the
/// rustflags the host declares. Nothing here is resolved at [`Project`]
/// open: a key is hashed fresh each time an entry is read or written, so
/// a long-lived process never stores output resolved against new inputs
/// under the key the old ones hashed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct GraphKey {
    /// `cargo --version` output, trimmed.
    cargo_version: String,
    /// `rustc -vV` output — the cfg values `cfg(...)` tables match are
    /// rustc's, so an answer holds only while the compiler does.
    rustc_version: String,
    /// sha256 of the project's `Cargo.toml`.
    manifest_sha256: String,
    /// sha256 of the workspace `Cargo.lock`; `None` when none exists.
    lockfile_sha256: Option<String>,
    /// sha256 of the workspace root's `Cargo.toml` — the
    /// `[workspace.dependencies]` feature selections and `resolver` a
    /// member inherits resolve into the graph from there, touching
    /// neither the member's own manifest nor the lock.
    workspace_manifest_sha256: String,
    /// sha256 of every `.cargo/config.toml` (or legacy `config`) Cargo's
    /// discovery applies to the project root, keyed by the file's path —
    /// the recorded set is re-enumerated on read, so a config file
    /// appearing or disappearing in the chain is a miss, not a stale
    /// replay.
    cargo_config_sha256: BTreeMap<String, String>,
    /// The rustflags variables the host's environment declares —
    /// `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS` and each target's
    /// `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` — keyed by the variable's name.
    rustflags_env: BTreeMap<String, String>,
    /// The rustflags the configuration resolves to per build target —
    /// `--cfg` flags in them enter `cfg(...)` matching.
    rustflags: BTreeMap<String, Vec<String>>,
}

/// `program`'s stdout run through `host` with the project root as its
/// working directory — the directory a `rust-toolchain.toml` beside the
/// project selects the toolchain from.
async fn toolchain_output(
    host: &Host,
    project_root: &Path,
    program: &str,
    args: &[&str],
) -> eyre::Result<String> {
    let mut process = host.command(program);
    let output = command(&mut process)
        .args(args)
        .current_dir(project_root)
        .output()
        .await
        .map_err(|error| eyre::eyre!("could not run `{program}`: {error}"))?;
    if !output.status.success() {
        eyre::bail!("`{program}` exited with {}", output.status);
    }
    String::from_utf8(output.stdout)
        .map_err(|error| eyre::eyre!("`{program}` printed non-UTF-8 output: {error}"))
}

/// `triple`'s `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` variable — Cargo's
/// uppercased, `-`→`_` encoding of the triple.
fn target_rustflags_env(triple: &Triple) -> String {
    format!(
        "CARGO_TARGET_{}_RUSTFLAGS",
        triple.to_string().to_uppercase().replace('-', "_")
    )
}

/// The toolchain versions plus the files every `cargo tree` evaluation
/// reads, hashed as they stand at call time — the project manifest and
/// lockfile, the workspace root's manifest, the `.cargo/config.toml`
/// chain and the rustflags it resolves to for each target.
async fn resolve_graph_key(
    host: &Host,
    project_root: &Path,
    layout: &CargoLayout,
    targets: &[Triple],
) -> eyre::Result<GraphKey> {
    let (cargo_version, rustc_version, rustc_path) = futures_util::future::try_join3(
        toolchain_output(host, project_root, "cargo", &["--version"]),
        toolchain_output(host, project_root, "rustc", &["-vV"]),
        host.which("rustc").map_err(Into::into),
    )
    .await?;

    let host = host.clone();
    let project_root = project_root.to_path_buf();
    let workspace_root = layout.workspace_root.clone();
    let targets = targets.to_vec();
    let cargo_version = cargo_version.trim().to_string();
    smol::unblock(move || {
        // Canonicalize before the ancestor walk so the `.cargo` chain
        // enumeration covers every directory Cargo's own canonicalized
        // discovery does.
        let project_root = dunce::canonicalize(project_root)?;
        let manifest_sha256 = sha256_hex(&std::fs::read(project_root.join("Cargo.toml"))?);
        let lockfile_sha256 = match std::fs::read(workspace_root.join("Cargo.lock")) {
            Ok(bytes) => Some(sha256_hex(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let workspace_manifest_sha256 =
            sha256_hex(&std::fs::read(workspace_root.join("Cargo.toml"))?);

        // Cargo's `$CARGO_HOME` resolution: absolute, anchored at the
        // working directory when relative, `~/.cargo` when unset — the
        // same walk `cargo_config2` performs.
        let cargo_home = host
            .env_string("CARGO_HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .map(|home| {
                if home.is_absolute() {
                    home
                } else {
                    project_root.join(home)
                }
            })
            .or_else(|| host.home_dir().map(|home| home.join(".cargo")));
        let cargo_config_sha256 =
            cargo_config2::Walk::with_cargo_home(&project_root, cargo_home.clone())
                .map(|file| {
                    std::fs::read(&file)
                        .map(|bytes| (file.display().to_string(), sha256_hex(&bytes)))
                })
                .collect::<Result<BTreeMap<String, String>, std::io::Error>>()?;
        // `cargo-config2` invokes rustc itself when a `cfg(...)` table in
        // the chain must resolve — the probe still goes through the host's
        // own `rustc`, resolved on the host's `PATH`.
        let options = cargo_config2::ResolveOptions::default()
            .env(
                host.envs()
                    .map(|(key, value)| (key.to_os_string(), value.to_os_string())),
            )
            .cargo_home(cargo_home)
            .rustc(cargo_config2::PathAndArgs::new(rustc_path));
        let config = cargo_config2::Config::load_with_options(&project_root, options)
            .wrap_err_with(|| {
                format!(
                    "could not resolve the Cargo configuration applying to {}",
                    project_root.display()
                )
            })?;

        let mut rustflags_env = BTreeMap::new();
        for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
            if let Some(value) = host.env_string(name) {
                rustflags_env.insert(name.to_string(), value);
            }
        }
        let mut rustflags = BTreeMap::new();
        for target in &targets {
            let name = target_rustflags_env(target);
            if let Some(value) = host.env_string(&name) {
                rustflags_env.insert(name, value);
            }
            rustflags.insert(
                target.to_string(),
                config
                    .rustflags(target.to_string())
                    .wrap_err_with(|| format!("could not resolve the rustflags for {target}"))?
                    .map(|flags| flags.flags)
                    .unwrap_or_default(),
            );
        }

        Ok(GraphKey {
            cargo_version,
            rustc_version,
            manifest_sha256,
            lockfile_sha256,
            workspace_manifest_sha256,
            cargo_config_sha256,
            rustflags_env,
            rustflags,
        })
    })
    .await
}

/// The workspace root manifest a path package's directory inherits
/// `[workspace.*]` settings from, per Cargo's own rule: an explicit
/// `[package] workspace = "<path>"` pointer when the package's manifest
/// carries one, else the nearest ancestor `Cargo.toml` carrying a
/// `[workspace]` table.
fn workspace_manifest_of(package_dir: &Path) -> Option<PathBuf> {
    for ancestor in package_dir.ancestors() {
        let manifest_path = ancestor.join("Cargo.toml");
        let Ok(manifest) =
            std::fs::read_to_string(&manifest_path).map(|text| text.parse::<toml::Table>())
        else {
            continue;
        };
        let Ok(manifest) = manifest else { continue };
        if ancestor == package_dir
            && let Some(pointer) = manifest
                .get("package")
                .and_then(|package| package.get("workspace"))
                .and_then(toml::Value::as_str)
            && let Ok(root) = dunce::canonicalize(ancestor.join(pointer))
        {
            return Some(root.join("Cargo.toml"));
        }
        if manifest.contains_key("workspace") {
            return Some(manifest_path);
        }
    }
    None
}

/// Re-hash every manifest a [`CachedTree`] entry recorded — each path
/// package's own `Cargo.toml` and the workspace root manifest each
/// inherits from, re-derived the way the write derived them so a
/// `[workspace]` table appearing above a package after the resolve reads
/// as a different set.
fn path_inputs_match(
    path_manifest_sha256: &BTreeMap<String, String>,
    workspace_manifest_sha256: &BTreeMap<String, String>,
) -> bool {
    let mut workspace = BTreeMap::new();
    for (manifest, recorded) in path_manifest_sha256 {
        let Ok(bytes) = std::fs::read(manifest) else {
            return false;
        };
        if sha256_hex(&bytes) != *recorded {
            return false;
        }
        let Some(package_dir) = Path::new(manifest).parent() else {
            return false;
        };
        if let Some(root_manifest) = workspace_manifest_of(package_dir) {
            let Ok(bytes) = std::fs::read(&root_manifest) else {
                return false;
            };
            workspace.insert(root_manifest.display().to_string(), sha256_hex(&bytes));
        }
    }
    workspace == *workspace_manifest_sha256
}

/// A persisted `cargo tree` evaluation: the raw `{p}` output plus every
/// input it resolved from — replayed only while all of them still hash
/// the same.
#[derive(Debug, Serialize, Deserialize)]
struct CachedTree {
    /// The `--target` set the evaluation filtered by, joined the way its
    /// cache key joins it — a guard against a renamed or colliding file.
    targets: String,
    /// Every input the evaluation resolved against, hashed at write time.
    key: GraphKey,
    /// sha256 of every path package's `Cargo.toml` the tree carried,
    /// keyed by manifest path — the recorded list is re-hashed on read,
    /// so a path package that changed after the resolve replays nothing.
    path_manifest_sha256: BTreeMap<String, String>,
    /// sha256 of the workspace root manifest each recorded path package
    /// inherits from, keyed by manifest path — `[workspace.dependencies]`
    /// feature selections and `resolver` reach the graph from there
    /// without touching the member manifests or the lock.
    workspace_manifest_sha256: BTreeMap<String, String>,
    /// The evaluation's raw stdout.
    output: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

fn cached_tree_path(cache_dir: &Path, edges: &str, key: &str) -> PathBuf {
    cache_dir.join(format!(
        "{edges}-{}.json",
        &sha256_hex(key.as_bytes())[..16]
    ))
}

/// The recorded output of a `(edges, key)` evaluation whose inputs still
/// hash the same — `None` on any miss: no entry, a stale input, a path
/// manifest that no longer reads, or an entry that does not parse.
async fn read_cached_tree(
    cache_dir: &Path,
    edges: &str,
    key: &str,
    graph_key: &GraphKey,
) -> Option<String> {
    let path = cached_tree_path(cache_dir, edges, key);
    let contents = smol::fs::read(&path).await.ok()?;
    let entry: CachedTree = serde_json::from_slice(&contents).ok()?;
    if entry.targets != key || entry.key != *graph_key {
        return None;
    }
    let path_manifest_sha256 = entry.path_manifest_sha256.clone();
    let workspace_manifest_sha256 = entry.workspace_manifest_sha256.clone();
    let output = entry.output;
    smol::unblock(move || path_inputs_match(&path_manifest_sha256, &workspace_manifest_sha256))
        .await
        .then_some(output)
}

/// Persist a fresh `(edges, key)` evaluation — every input was hashed as
/// it stood when the write began, and the recorded path-package manifests
/// and their workspace roots ride inside the entry so a later read
/// re-hashes exactly the list the resolve picked up. The entry is staged
/// to a temporary file and renamed over the destination — a reader
/// mid-write never sees a torn entry, the same discipline
/// `metadata.toml`'s writes follow.
async fn write_cached_tree(
    cache_dir: &Path,
    edges: &str,
    key: &str,
    graph_key: GraphKey,
    output: &str,
) -> eyre::Result<()> {
    let cache_dir = cache_dir.to_path_buf();
    let path = cached_tree_path(&cache_dir, edges, key);
    let key = key.to_string();
    let output = output.to_string();
    smol::unblock(move || {
        use std::io::Write as _;
        let mut path_manifest_sha256 = BTreeMap::new();
        let mut workspace_manifest_sha256 = BTreeMap::new();
        for directory in output.lines().filter_map(tree_package_directory) {
            let manifest = directory.join("Cargo.toml");
            let bytes = std::fs::read(&manifest)?;
            path_manifest_sha256.insert(manifest.display().to_string(), sha256_hex(&bytes));
            if let Some(root_manifest) = workspace_manifest_of(&directory)
                && let std::collections::btree_map::Entry::Vacant(slot) =
                    workspace_manifest_sha256.entry(root_manifest.display().to_string())
            {
                let bytes = std::fs::read(&root_manifest)?;
                slot.insert(sha256_hex(&bytes));
            }
        }
        let entry = CachedTree {
            targets: key,
            key: graph_key,
            path_manifest_sha256,
            workspace_manifest_sha256,
            output,
        };
        std::fs::create_dir_all(&cache_dir)?;
        let mut file = tempfile::NamedTempFile::new_in(&cache_dir)?;
        file.write_all(&serde_json::to_vec(&entry)?)?;
        file.persist(&path)?;
        eyre::Result::Ok(())
    })
    .await
}

/// The generated-manifest section a graph answer is written to, named in
/// a disagreement error so the message says which section cannot express
/// the per-target difference — and, through `remedy`, how a manifest that
/// legitimately needs one can express it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GraphSection {
    /// The generated manifest the section is written into.
    pub manifest: &'static str,
    /// The table the answer is written under — a `cfg(...)` key or the
    /// consumer that carries the answer.
    pub table: &'static str,
    /// How a manifest that needs different answers per target expresses
    /// the split — named per section because not every split axis can
    /// fix every disagreement: a gnu/musl difference is a `target_env`
    /// split, not a `target_os` one, and a feature list shared by every
    /// ABI's build cannot be split at all.
    pub remedy: &'static str,
}

/// Represents a `WaterUI` project with its manifest and crate information.
#[derive(Debug, Clone)]
pub struct Project {
    host: Host,
    root: PathBuf,
    manifest: Manifest,
    crate_name: CrateName,
    cargo_layout: Shared<BoxFuture<'static, Result<CargoLayout, String>>>,
    /// The normal-edge `cargo tree` resolutions already run for this
    /// project, keyed by the build triple each `--target` filtered for.
    linked_packages: TargetGraph<LinkedPackages>,
    /// The `--edges features` `cargo tree` resolutions, keyed by triple the
    /// same way — a feature answer resolved for one target is never served
    /// to a build for another.
    enabled_features: TargetGraph<BTreeSet<String>>,
    /// The per-package enabled-feature sets of generated manifests —
    /// `cargo metadata --filter-platform <triple>` — keyed by manifest and
    /// triple, so Gradle's four ABI checks share one resolve each and an
    /// answer resolved for one target is never served to another.
    generated_features: TargetGraph<BTreeMap<String, BTreeSet<String>>>,
    /// The bound [`Self::CARGO_RESOLVE_PERMITS`] states, shared by every
    /// resolution this project's cached answers drive.
    cargo_resolve_permits: Arc<async_lock::Semaphore>,
    managed_backends_root: PathBuf,
    /// The persisted `cargo tree` evaluations' directory — a sibling of
    /// `managed_backends_root`: it holds cached answers, not a generated
    /// backend.
    graph_cache_dir: PathBuf,
    /// The runtime backends this open generated — project-owned state, never
    /// persisted. Persisted backend-facing configuration lives in the
    /// manifest's typed tables (`[esp32]`, `[hydrolysis]`).
    backends: Backends,
    /// The canonical local backend sources the manifest's `waterui_path`
    /// checkout supplies, resolved before any template or backend
    /// generation ran — a malformed slot already failed the open.
    local_sources: crate::templates::LocalBackendSources,
}

impl Project {
    /// Select or update a framework channel and persist its exact dependency selection.
    /// `rev` pins `dev` to an exact commit of the branch's history; the
    /// certified channels reject it.
    ///
    /// # Errors
    /// Returns an error when resolution, native-project merging, or dependency verification fails.
    pub async fn select_channel(
        host: &Host,
        path: impl AsRef<Path>,
        channel: FrameworkChannel,
        rev: Option<&str>,
    ) -> eyre::Result<Self> {
        let path = smol::fs::canonicalize(path.as_ref()).await?;
        let water_path = path.join("Water.toml");
        let cargo_path = path.join("Cargo.toml");
        let mut water: toml_edit::DocumentMut =
            smol::fs::read_to_string(&water_path).await?.parse()?;
        let previous = Manifest::parse(&water.to_string())?;
        let mut cargo: toml_edit::DocumentMut =
            smol::fs::read_to_string(&cargo_path).await?.parse()?;
        let (framework, lockfile) = ResolvedFramework::resolve(host, channel, rev).await?;
        // A configured backend whose scaffold packages the target channel
        // withholds could never be regenerated — refuse the switch before a
        // manifest is rewritten.
        if previous.esp32.is_some() {
            for package in TargetBackend::Dew.scaffold_packages() {
                framework.require_distributable(package)?;
            }
        }
        let mut updates = Vec::new();
        framework.update_manifest(&mut cargo, &templates::project_patches(&path, &previous)?)?;
        water.remove("waterui_path");
        water["framework"] =
            toml_edit::Item::Table(toml_edit::ser::to_document(&framework)?.into_table());
        updates.push((water_path, water.to_string().into_bytes()));
        updates.push((cargo_path, cargo.to_string().into_bytes()));
        if let Some(lockfile) = lockfile {
            updates.push((
                path.join("Cargo.lock"),
                framework.cargo_lock(&lockfile)?.to_string().into_bytes(),
            ));
            updates.push((path.join("Water.lock"), lockfile));
        }
        let mut updates: Vec<_> = updates
            .into_iter()
            .map(|(file, contents)| (file, Some(contents)))
            .collect();
        if channel == FrameworkChannel::Stable
            && previous
                .framework
                .as_ref()
                .is_some_and(|previous| previous.channel() != Some(FrameworkChannel::Stable))
        {
            updates.push((path.join("Water.lock"), None));
        }
        apply_channel_selection(host, &path, framework, updates).await?;
        Self::open_for_preview_build(host, path)
            .await
            .map_err(Into::into)
    }

    /// Run the `WaterUI` project on the specified device.
    ///
    /// This method handles building, packaging, and running the project.
    ///
    /// # Arguments
    /// - `backend`: The backend to use for building and packaging
    /// - `platform`: The target platform to build for
    /// - `device`: The device to run on
    ///
    /// # Errors
    /// - If any step in the build, package, or run process fails.
    pub async fn run<B: Backend, D: Device>(
        &self,
        backend: &B,
        platform: TargetPlatform,
        device: D,
    ) -> Result<Running, FailToRun> {
        self.run_with_options(backend, platform, device, RunOptions::new(), None)
            .await
    }

    /// Run the `WaterUI` project with explicit run options.
    ///
    /// This allows callers (like preview) to inject extra environment variables.
    /// `progress`, when given, receives cargo compile events from both the
    /// library build and the packaging pass's asset-manifest compile.
    ///
    /// # Errors
    /// Returns an error if building, packaging, or launching the app fails.
    pub async fn run_with_options<B: Backend, D: Device>(
        &self,
        backend: &B,
        platform: TargetPlatform,
        device: D,
        run_options: RunOptions,
        progress: Option<BuildProgress>,
    ) -> Result<Running, FailToRun> {
        let mut build_options = BuildOptions::development(BuildProfile::Debug);
        if let Some(progress) = &progress {
            build_options = build_options.with_progress(progress.clone());
        }
        // Build rust library for the target platform
        let built = backend
            .build(self, platform, build_options)
            .await
            .map_err(FailToRun::Build)?;

        let mut package_options = PackageOptions::development();
        if let Some(progress) = progress {
            package_options = package_options.with_progress(progress);
        }
        // A physical-device package must be provisioned for the exact
        // device it will install on — the device reports its UDID.
        if let Some(udid) = device.device_udid() {
            package_options = package_options.with_device_udid(Some(udid.to_string()));
        }
        // Package the build artifacts for the target platform
        let artifact = backend
            .package(self, platform, package_options, &built)
            .await
            .map_err(FailToRun::Package)?;

        self.run_packaged(device, artifact, run_options).await
    }

    async fn run_packaged<D: Device>(
        &self,
        device: D,
        artifact: Artifact,
        run_options: RunOptions,
    ) -> Result<Running, FailToRun> {
        info!("Running on device");

        let running = device.run(self.host(), artifact, run_options).await?;
        Ok(running)
    }

    /// Get the root path of the project.
    ///
    /// Same as the directory containing `Water.toml`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The host the project was opened on — the machine its toolchain
    /// probes, builds, and launches run against.
    #[must_use]
    pub const fn host(&self) -> &Host {
        &self.host
    }

    /// Get the target directory for Rust build artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot resolve the target directory.
    pub async fn target_dir(&self) -> eyre::Result<PathBuf> {
        Ok(self.cargo_layout().await?.target_dir)
    }

    /// The lockfile the application builds against.
    ///
    /// The workspace `Cargo.lock` when the project is a workspace member,
    /// otherwise the project's own. It need not exist yet: a project that has
    /// never been resolved has none.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot resolve the workspace.
    pub async fn lockfile_path(&self) -> eyre::Result<PathBuf> {
        Ok(self.cargo_layout().await?.workspace_root.join("Cargo.lock"))
    }

    async fn cargo_layout(&self) -> eyre::Result<CargoLayout> {
        self.cargo_layout
            .clone()
            .await
            .map_err(|error| eyre::eyre!(error))
    }

    /// Resolve the Cargo target directory every generated backend crate builds into.
    ///
    /// The directory is shared by every project on the machine — one
    /// `~/.water/build_cache/target` subtree — because Cargo already keys each
    /// compiled unit by target triple, resolved features, and profile: a second
    /// project's build reuses the dependency graph the first one compiled
    /// instead of cold-building it, the way sccache-equipped machines behave.
    /// The shared root sits beside the per-project managed containers rather
    /// than inside one: generated backend sources are deleted and regenerated
    /// whenever the CLI's scaffold templates change, while compiled artifacts
    /// do not become stale for that reason — keeping them together meant one
    /// CLI upgrade discarded the compiled dependency graph of every project on
    /// the machine.
    ///
    /// One directory serves every backend, platform, and feature set of a
    /// linkage: switching backends only rebuilds the units the two graphs do
    /// not share — measured on an example app, over 80% of the Apple FFI graph
    /// resolves identically to the Hydrolysis graph and is reused as-is.
    /// Builds must therefore agree on everything Cargo hashes into every unit —
    /// pass an explicit `--target` and keep final-artifact link flags out of
    /// `RUSTFLAGS` (see `RustBuild::with_final_rustc_arg`) — or two variants
    /// sharing this directory re-fingerprint each other's entire dependency
    /// graph on every switch.
    ///
    /// Linkage is the one axis Cargo cannot separate: shared-runtime development
    /// builds carry `-Cprefer-dynamic -Crpath` in `RUSTFLAGS` and static packaging
    /// builds carry none, so each linkage keeps its own directory instead of the two
    /// variants invalidating each other whenever a developer alternates `water run`
    /// and `water package`.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared build-cache directory cannot be resolved.
    pub async fn water_target_dir(&self, linkage: RustLinkage) -> eyre::Result<PathBuf> {
        let variant = match linkage {
            RustLinkage::SharedRuntime => "shared",
            RustLinkage::Static => "static",
        };
        Ok(crate::water_dir::shared_target_dir(self.host())
            .await?
            .join(variant))
    }

    /// Resolve an isolated target directory for a backend built by a different Rust
    /// toolchain.
    ///
    /// Cargo hashes the compiler into every unit fingerprint, so a backend that pins
    /// its own toolchain (ESP32's Espressif Rust fork) would invalidate the host
    /// units of [`Self::water_target_dir`] on every switch if it shared the directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared build-cache directory cannot be resolved.
    pub async fn toolchain_target_dir(&self, toolchain: &str) -> eyre::Result<PathBuf> {
        Ok(crate::water_dir::shared_target_dir(self.host())
            .await?
            .join(format!("toolchain-{toolchain}")))
    }

    /// Get the runtime backends configured for the project.
    #[must_use]
    pub const fn backends(&self) -> &Backends {
        &self.backends
    }

    /// Get the crate name of the project.
    #[must_use]
    pub const fn crate_name(&self) -> &CrateName {
        &self.crate_name
    }

    /// Get configured or default FFI crate name.
    ///
    /// The default is tagged with this project's root — see
    /// [`generated_crate_name`]; an explicit `[crates]` override is verbatim.
    #[must_use]
    pub fn ffi_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.ffi.clone())
            .unwrap_or_else(|| generated_crate_name(&self.crate_name, "ffi", &self.root))
    }

    /// Get configured preview wrapper crate name for preview dylib builds.
    #[must_use]
    pub fn preview_ffi_crate_name(&self) -> CrateName {
        generated_crate_name(&self.crate_name, "preview-ffi", &self.root)
    }

    /// Get the generated Apple in-process preview binary's crate name.
    #[must_use]
    pub fn apple_preview_crate_name(&self) -> CrateName {
        generated_crate_name(&self.crate_name, "apple-preview", &self.root)
    }

    /// Get the crate root path used to build preview dylibs.
    #[must_use]
    pub fn preview_dylib_crate_path(&self, workspace_root: &Path) -> PathBuf {
        self.preview_ffi_crate_path(workspace_root)
    }

    /// Get the crate name used to build preview dylibs.
    #[must_use]
    pub fn preview_dylib_crate_name(&self) -> CrateName {
        self.preview_ffi_crate_name()
    }

    /// Get configured or default GTK backend crate name.
    #[must_use]
    pub fn gtk_backend_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.gtk.clone())
            .unwrap_or_else(|| generated_crate_name(&self.crate_name, "gtk4", &self.root))
    }

    /// Get configured or default hydrolysis backend crate name.
    #[must_use]
    pub fn hydrolysis_backend_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.hydrolysis.clone())
            .unwrap_or_else(|| generated_crate_name(&self.crate_name, "hydrolysis", &self.root))
    }

    /// Get configured or default `WinUI` backend crate name.
    #[must_use]
    pub fn winui_backend_crate_name(&self) -> CrateName {
        self.app_crate_overrides()
            .and_then(|crates| crates.winui.clone())
            .unwrap_or_else(|| generated_crate_name(&self.crate_name, "winui", &self.root))
    }

    /// Get the generated ESP32 firmware harness crate name.
    #[must_use]
    pub fn esp32_backend_crate_name(&self) -> CrateName {
        generated_crate_name(&self.crate_name, "esp32", &self.root)
    }

    /// Get the crate name of the generated experimental TUI launcher.
    #[must_use]
    pub fn tui_backend_crate_name(&self) -> CrateName {
        generated_crate_name(&self.crate_name, "tui", &self.root)
    }

    /// The executable name a packaged backend binary ships under: the
    /// configured `[crates]` override verbatim, or `<crate>-<suffix>` when
    /// the crate is generated.
    ///
    /// [`generated_crate_name`]'s project-root tag exists to keep a shared
    /// Cargo target directory unambiguous; it is internal to the build and
    /// must never name a shipped executable.
    fn shipped_backend_binary_name(
        &self,
        suffix: &str,
        configured: Option<&CrateName>,
    ) -> CrateName {
        configured
            .cloned()
            .unwrap_or_else(|| self.crate_name.with_suffix(suffix))
    }

    /// The executable name the packaged GTK4 binary ships under.
    #[must_use]
    pub fn gtk4_binary_name(&self) -> CrateName {
        self.shipped_backend_binary_name(
            "gtk4",
            self.app_crate_overrides()
                .and_then(|crates| crates.gtk.as_ref()),
        )
    }

    /// The executable name the packaged hydrolysis binary ships under.
    #[must_use]
    pub fn hydrolysis_binary_name(&self) -> CrateName {
        self.shipped_backend_binary_name(
            "hydrolysis",
            self.app_crate_overrides()
                .and_then(|crates| crates.hydrolysis.as_ref()),
        )
    }

    /// The executable name the packaged `WinUI` binary ships under.
    #[must_use]
    pub fn winui_binary_name(&self) -> CrateName {
        self.shipped_backend_binary_name(
            "winui",
            self.app_crate_overrides()
                .and_then(|crates| crates.winui.as_ref()),
        )
    }

    /// The name the packaged ESP32 firmware image ships under.
    #[must_use]
    pub fn esp32_binary_name(&self) -> CrateName {
        self.shipped_backend_binary_name("esp32", None)
    }

    /// Get the Apple backend if this open generated one.
    #[must_use]
    pub const fn apple_backend(&self) -> Option<&AppleBackend> {
        self.backends.apple()
    }

    /// Get the full path to a generated backend directory in the managed
    /// build cache.
    #[must_use]
    pub fn backend_path<B: Backend>(&self) -> PathBuf {
        self.managed_backends_root.join(B::DEFAULT_PATH)
    }

    /// Get the full path to the managed native FFI companion crate.
    #[must_use]
    pub fn ffi_crate_path(&self) -> PathBuf {
        self.managed_backends_root.join("ffi")
    }

    /// Get the full path to the managed Apple in-process preview package.
    ///
    /// It sits next to the FFI companion crate: the two share one
    /// dependency-table function so the preview binary resolves the same
    /// `waterui` and `libwaterui_dylib` build `water run` produces.
    #[must_use]
    pub fn apple_preview_crate_path(&self) -> PathBuf {
        self.managed_backends_root.join("apple-preview")
    }

    /// Directory name this project's preview module occupies inside a workspace.
    #[must_use]
    pub fn preview_module_member_path(&self) -> PathBuf {
        Path::new(crate::templates::PREVIEW_MODULES_DIR)
            .join(self.preview_ffi_crate_name().to_string())
    }

    /// Get the full path to the managed preview-only companion crate.
    ///
    /// The crate lives inside the support runtime's workspace rather than this
    /// project's build cache, because a preview module and the runtime it is
    /// loaded into must come out of one Cargo resolution to agree on the
    /// `-C metadata` hash mangled into every symbol.
    #[must_use]
    pub fn preview_ffi_crate_path(&self, workspace_root: &Path) -> PathBuf {
        workspace_root.join(self.preview_module_member_path())
    }

    /// Get the Android backend if this open generated one.
    #[must_use]
    pub const fn android_backend(&self) -> Option<&AndroidBackend> {
        self.backends.android()
    }

    /// Get the project's `[esp32]` device configuration, if declared.
    #[must_use]
    pub const fn esp32_config(&self) -> Option<&crate::esp32::backend::Esp32Config> {
        self.manifest.esp32.as_ref()
    }

    /// The canonical local backend sources the `waterui_path` checkout
    /// supplies, resolved at open — empty means the pinned remote sources.
    #[must_use]
    pub const fn local_sources(&self) -> &crate::templates::LocalBackendSources {
        &self.local_sources
    }

    /// Get the manifest of the project.
    #[must_use]
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The framework this project resolves generated code against — the
    /// channel selection `Water.toml` records, or the checkout `waterui_path`
    /// names.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest records no framework source, its
    /// saved selection is invalid, or the local checkout's framework facts
    /// cannot be read.
    pub async fn resolved_framework(&self) -> eyre::Result<ResolvedFramework> {
        ResolvedFramework::for_manifest(self.host(), self.manifest(), &self.root).await
    }

    /// Assert the selected framework channel distributes every scaffold
    /// package `backend` links — the git-pinned experimental set `stable`
    /// withholds. Runs before the backend writes a file, so a withheld
    /// package fails the init with the channel fix rather than partway
    /// through the generated tree.
    ///
    /// # Errors
    ///
    /// Returns an error naming the withheld package and the channel fix.
    pub async fn require_distributable_backend(
        &self,
        backend: TargetBackend,
    ) -> Result<(), crate::backend::FailToInitBackend> {
        let packages = backend.scaffold_packages();
        if packages.is_empty() {
            return Ok(());
        }
        let framework = self
            .resolved_framework()
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;
        for package in packages {
            framework
                .require_distributable(package)
                .map_err(crate::backend::FailToInitBackend::Config)?;
        }
        Ok(())
    }

    /// `cargo tree` evaluations issued to answer one generated-manifest
    /// question run concurrently up to this bound — the serving sets a
    /// render resolves hold a dozen-plus triples on a cold cache, and an
    /// unbounded fan-out would race Cargo's own lockfile and the machine's
    /// I/O.
    const TARGET_GRAPH_CONCURRENCY: usize = 8;

    /// Cargo resolutions across the whole project run under this many
    /// permits — the bound is global to the `Project`, not per call, so a
    /// backend render's dozen-plus `cargo tree`/`cargo metadata` probes
    /// cannot all contend on Cargo's package-cache and lockfile lock and
    /// on the machine's I/O at once.
    const CARGO_RESOLVE_PERMITS: usize = 8;

    /// The parsed `cargo tree --edges <edges>` evaluation for `targets`,
    /// cached under the canonical join of the target list — one triple
    /// keys on itself, a serving set on the whole set — so an evaluation
    /// resolved for one build target is never served to another, and a
    /// single-target call and the same single-element set share one
    /// entry. Behind each key's `Shared` future sits the project build
    /// cache: an entry whose recorded inputs still hash the same replays
    /// its stored output without resolving anything.
    async fn cached_tree_answer<T: Send + Sync + 'static>(
        &self,
        cache: &TargetGraph<T>,
        edges: &'static str,
        targets: &[Triple],
        parse: fn(&str) -> eyre::Result<T>,
    ) -> eyre::Result<Arc<T>> {
        // The cache key names the exact set the evaluation filtered by;
        // triples contain no commas, so the join is unambiguous.
        let mut names: Vec<String> = targets.iter().map(ToString::to_string).collect();
        names.sort_unstable();
        names.dedup();
        let key = names.join(",");
        let host = self.host.clone();
        let project_root = self.root.clone();
        let cache_dir = self.graph_cache_dir.clone();
        let cargo_layout = self.cargo_layout.clone();
        let permits = Arc::clone(&self.cargo_resolve_permits);
        let targets = targets.to_vec();
        let shared = cache
            .lock()
            .await
            .entry(key.clone())
            .or_insert_with(move || {
                async move {
                    let layout = cargo_layout.await?;
                    let output = if let Some(output) = {
                        let fresh = resolve_graph_key(&host, &project_root, &layout, &targets)
                            .await
                            .map_err(|error| error.to_string())?;
                        read_cached_tree(&cache_dir, edges, &key, &fresh).await
                    } {
                        output
                    } else {
                        let output = {
                            let _permit = permits.acquire_arc().await;
                            cargo_tree(
                                &host,
                                &project_root,
                                &layout.root_package_id,
                                edges,
                                &targets,
                                false,
                            )
                            .await
                            .map_err(|error| error.to_string())?
                        };
                        // Hash the inputs as they stand at write time —
                        // not the read's copy — so a manifest or toolchain
                        // that changed mid-resolve never has the fresh
                        // output recorded under its old key.
                        let written = resolve_graph_key(&host, &project_root, &layout, &targets)
                            .await
                            .map_err(|error| error.to_string())?;
                        write_cached_tree(&cache_dir, edges, &key, written, &output)
                            .await
                            .map_err(|error| error.to_string())?;
                        output
                    };
                    parse(&output)
                        .map(Arc::new)
                        .map_err(|error| error.to_string())
                }
                .boxed()
                .shared()
            })
            .clone();
        shared.await.map_err(|error| eyre::eyre!(error))
    }

    /// The resolved `{p}` entries of the application's normal-edge
    /// dependency graph for `target` — every printed occurrence, keyed by
    /// package name.
    async fn linked_packages(&self, target: &Triple) -> eyre::Result<Arc<LinkedPackages>> {
        self.cached_tree_answer(
            &self.linked_packages,
            "normal",
            std::slice::from_ref(target),
            |tree| Ok(linked_packages_from_tree(tree)),
        )
        .await
    }

    /// The feature names the application's `cargo tree --edges features
    /// --target <triple>` evaluation reports.
    async fn enabled_features(&self, target: &Triple) -> eyre::Result<Arc<BTreeSet<String>>> {
        self.cached_tree_answer(
            &self.enabled_features,
            "features",
            std::slice::from_ref(target),
            |tree| Ok(enabled_features_from_tree(tree)),
        )
        .await
    }

    /// Every package's enabled features in `build_manifest`'s resolved
    /// graph for `target` — `cargo metadata --filter-platform <triple>`,
    /// cached per manifest and triple so a build's ABI-by-ABI feature
    /// checks share one resolve each and an answer resolved for one
    /// target is never served to another. Packages whose names repeat in
    /// the graph merge their resolved feature sets, the question asked of
    /// this map always being "is package X's feature Y enabled".
    ///
    /// # Errors
    ///
    /// Returns an error when `cargo metadata` cannot resolve the manifest.
    pub(crate) async fn generated_manifest_features(
        &self,
        build_manifest: &Path,
        target: &Triple,
    ) -> eyre::Result<Arc<BTreeMap<String, BTreeSet<String>>>> {
        // `\u{1f}` separates the manifest from the triple — neither side
        // can contain it, so the key is unambiguous.
        let key = format!("{}\u{1f}{target}", build_manifest.display());
        let host = self.host.clone();
        let build_manifest = build_manifest.to_path_buf();
        let target = target.clone();
        let permits = Arc::clone(&self.cargo_resolve_permits);
        let shared = self
            .generated_features
            .lock()
            .await
            .entry(key)
            .or_insert_with(move || {
                async move {
                    let metadata = {
                        let _permit = permits.acquire_arc().await;
                        unblock(move || {
                            let mut command = cargo_metadata::MetadataCommand::new();
                            command.manifest_path(&build_manifest).other_options(vec![
                                "--filter-platform".to_string(),
                                target.to_string(),
                            ]);
                            metadata_on(&host, &command)
                        })
                        .await
                    }
                    .map_err(|error| error.to_string())?;
                    let mut features: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                    if let Some(resolve) = &metadata.resolve {
                        for node in &resolve.nodes {
                            let Some(package) = metadata.packages.iter().find(|p| p.id == node.id)
                            else {
                                continue;
                            };
                            features
                                .entry(package.name.to_string())
                                .or_default()
                                .extend(node.features.iter().map(ToString::to_string));
                        }
                    }
                    Ok(Arc::new(features))
                }
                .boxed()
                .shared()
            })
            .clone();
        shared.await.map_err(|error| eyre::eyre!(error))
    }

    /// The one answer `query` gives for every build target `targets`
    /// names, each resolved through its own `cargo tree --target`
    /// evaluation — concurrently, bounded by
    /// [`Self::TARGET_GRAPH_CONCURRENCY`].
    ///
    /// `targets` is the serving set of the generated-manifest section
    /// `section` names: the answer is written once and must hold for every
    /// triple the section compiles for, so it has to agree across the set.
    /// A query that answers differently for two triples fails naming the
    /// manifest and table, every triple grouped by the answer it
    /// resolved, and the remedy — the manifest cannot express a per-target
    /// difference, and silently picking one side would misrender it.
    ///
    /// `query` runs against `self` and each triple; the futures it hands
    /// back stay inside this call — they are polled by the bounded stream
    /// and never escape — so they may borrow `self` but must be `Send`.
    pub(crate) async fn unanimous_graph_answer<'q, T, F, Fut>(
        &'q self,
        targets: &[Triple],
        what: &'static str,
        section: GraphSection,
        describe: impl Fn(&T) -> String + Send + Sync,
        query: F,
    ) -> eyre::Result<T>
    where
        T: PartialEq + Send,
        F: Fn(&'q Self, Triple) -> Fut + Send + Sync,
        Fut: Future<Output = eyre::Result<T>> + Send,
    {
        let mut resolved = stream::iter(targets.iter().cloned())
            .map(|target| {
                let answer = query(self, target.clone());
                async move { answer.await.map(|answer| (target, answer)) }
            })
            .buffered(Self::TARGET_GRAPH_CONCURRENCY);
        let mut answers = Vec::new();
        while let Some(next) = resolved.next().await {
            answers.push(next?);
        }
        let Some((_, first)) = answers.first() else {
            eyre::bail!("{what} was queried for no build targets");
        };
        if answers.iter().all(|(_, answer)| answer == first) {
            return Ok(answers.into_iter().next().expect("answers is non-empty").1);
        }
        // Every triple the table serves groups under the answer its own
        // graph resolved — naming two would still leave the rest of the
        // disagreement unnamed.
        let mut groups: BTreeMap<String, Vec<&Triple>> = BTreeMap::new();
        for (triple, answer) in &answers {
            groups.entry(describe(answer)).or_default().push(triple);
        }
        let mut grouped = String::new();
        for (answer, triples) in &groups {
            let triples = triples
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(grouped, "\n    {answer}: {triples}");
        }
        eyre::bail!(
            "{what} differs across the build targets {manifest}'s {table} serves:{grouped}\n\
             {remedy}",
            manifest = section.manifest,
            table = section.table,
            remedy = section.remedy,
        )
    }

    /// Returns whether the packaged application links `package_name` for
    /// `target`.
    ///
    /// Development-only and build-only dependencies are excluded because they
    /// do not become part of the packaged application. The resolved graph is
    /// cached per triple so backend regeneration and scaffolding share each
    /// Cargo resolution.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo cannot resolve the application graph or
    /// omits a package referenced by that graph.
    pub async fn links_runtime_package(
        &self,
        target: &Triple,
        package_name: &str,
    ) -> eyre::Result<bool> {
        Ok(self
            .linked_packages(target)
            .await?
            .contains_key(package_name))
    }

    /// The application's own packages in `target`'s graph: the app crate
    /// plus every other package in its normal-edge dependency graph that is
    /// a path package and does not belong to the resolved framework — the
    /// set `generated_profiles` keeps at `opt-level = 0` with line tables so
    /// the user's own code stays steppable in development builds.
    ///
    /// A package belongs to the framework when its manifest lies inside one of
    /// the framework's local roots — see `framework_local_roots`. Channel
    /// and git framework packages have non-path sources, so the path condition
    /// already excludes them. `framework` is the framework the caller already
    /// resolved — [`Self::resolved_framework`] — so a command resolves it
    /// once rather than once per profile lookup.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo cannot resolve the application graph, a
    /// framework source or a path package's directory cannot be canonicalized.
    pub async fn project_packages(
        &self,
        framework: &ResolvedFramework,
        target: &Triple,
    ) -> eyre::Result<BTreeSet<String>> {
        let packages = self.linked_packages(target).await?;
        let framework_roots = framework_local_roots(&self.root, self.manifest(), framework)?;
        project_packages_from_tree(self.crate_name.as_str(), &packages, &framework_roots)
    }

    /// The application's own packages across `targets` — a union, not an
    /// agreement: a `[profile.dev.package]` override applies to whichever
    /// build's resolve carries the package, and an entry for a package a
    /// given target does not link merely never applies. One `cargo tree`
    /// evaluation answers the whole set — every triple rides the same
    /// resolve as its own `--target` flag, and the printed union is the
    /// per-target union exactly.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo cannot resolve the application graph for
    /// one of the targets.
    pub(crate) async fn project_packages_for(
        &self,
        framework: &ResolvedFramework,
        targets: &[Triple],
    ) -> eyre::Result<BTreeSet<String>> {
        if targets.is_empty() {
            return Ok(BTreeSet::new());
        }
        let packages = self
            .cached_tree_answer(&self.linked_packages, "normal", targets, |tree| {
                Ok(linked_packages_from_tree(tree))
            })
            .await?;
        let framework_roots = framework_local_roots(&self.root, self.manifest(), framework)?;
        project_packages_from_tree(self.crate_name.as_str(), &packages, &framework_roots)
    }

    /// Whether the application's graph turns on the standard `WebView`
    /// component for `target`.
    ///
    /// The signal is a `webview` feature enabled inside the application's own
    /// subtree — the facade's `webview` feature, or an engine crate's `webview`
    /// hookup — resolved by `cargo tree --edges features`. The
    /// `waterui-webview` *package* cannot be the signal: engines link it for
    /// the shared asset-server types a `ChromiumPage` answers over CDP
    /// (#586), so its presence no longer means the component is used.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo cannot resolve the application graph or
    /// omits a package referenced by that graph.
    pub async fn uses_standard_webview(&self, target: &Triple) -> eyre::Result<bool> {
        Ok(self.enabled_features(target).await?.contains("webview"))
    }

    /// The one `WebView` answer for every target `targets` names — the
    /// serving set of the generated manifest `section` the caller writes.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo cannot resolve the application graph, or
    /// when the answer differs across `targets`.
    pub(crate) async fn uses_standard_webview_for(
        &self,
        targets: &[Triple],
        section: GraphSection,
    ) -> eyre::Result<bool> {
        self.unanimous_graph_answer(
            targets,
            "the standard WebView usage",
            section,
            |enabled: &bool| enabled.to_string(),
            |project, target| async move { project.uses_standard_webview(&target).await },
        )
        .await
    }

    /// The two `WebView` answers `os`'s generated-manifest table gets from
    /// the application's own graphs — usage and linked engine — resolved
    /// together from `os`'s serving set. Every graph-derived answer a
    /// table writes goes through this one resolution, so a table can never
    /// read half its answers from a different set.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo cannot resolve the application graph or
    /// an answer differs across `os`'s serving set.
    pub(crate) async fn native_browser_answers(
        &self,
        os: crate::platform::NativeOs,
        section: GraphSection,
    ) -> eyre::Result<crate::templates::BrowserAnswers> {
        let targets = os.serving_triples();
        let (webview_enabled, engine) = futures_util::future::try_join(
            self.uses_standard_webview_for(&targets, section),
            self.linked_browser_engine_for(&targets, section),
        )
        .await?;
        Ok(crate::templates::BrowserAnswers {
            webview_enabled,
            engine,
        })
    }

    /// Resolve and validate the standard `WebView` engine for a build.
    ///
    /// The application's own dependency graph is the selection: linking
    /// `waterui-browser-cef` or `waterui-browser-wpe` picks that engine, and an
    /// app that links neither uses whatever web engine the target platform
    /// bridges. Nothing in `Water.toml` names an engine, because nothing else
    /// could keep the packaged runtime and the code that loads it in step.
    ///
    /// Returns `None` when no `webview` feature is enabled in the
    /// application's graph, so an engine crate reaching the graph through some
    /// other component never adds a `WebView` runtime to the package on its
    /// own.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot be resolved, when the
    /// application links two engines at once, or when the selected engine is
    /// unsupported for the requested platform and backend.
    pub async fn resolved_webview_backend(
        &self,
        platform: TargetPlatform,
        backend: TargetBackend,
        target: &Triple,
    ) -> eyre::Result<Option<ResolvedWebViewBackend>> {
        if !self.uses_standard_webview(target).await? {
            return Ok(None);
        }
        let engine = self.linked_browser_engine(target).await?;
        engine
            .unwrap_or(ResolvedWebViewBackend::System)
            .validate(platform, backend)
            .map(Some)
            .map_err(Into::into)
    }

    /// The browser engine crate the application links for `target`, if any.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot be resolved, or when the
    /// application links more than one engine — two engines cannot both draw
    /// one `WebView`, and the second `install` would fail at startup.
    pub async fn linked_browser_engine(
        &self,
        target: &Triple,
    ) -> eyre::Result<Option<ResolvedWebViewBackend>> {
        let cef = self
            .links_runtime_package(target, "waterui-browser-cef")
            .await?;
        let wpe = self
            .links_runtime_package(target, "waterui-browser-wpe")
            .await?;
        match (cef, wpe) {
            (true, true) => eyre::bail!(
                "the application links both waterui-browser-cef and waterui-browser-wpe; \
                 exactly one browser engine can draw a WebView"
            ),
            (true, false) => Ok(Some(ResolvedWebViewBackend::Cef)),
            (false, true) => Ok(Some(ResolvedWebViewBackend::Wpe)),
            (false, false) => Ok(None),
        }
    }

    /// The one engine answer for every target `targets` names — the
    /// serving set of the generated manifest `section` the caller writes.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot be resolved, when the
    /// application links two engines at once, or when the answer differs
    /// across `targets`.
    pub(crate) async fn linked_browser_engine_for(
        &self,
        targets: &[Triple],
        section: GraphSection,
    ) -> eyre::Result<Option<ResolvedWebViewBackend>> {
        self.unanimous_graph_answer(
            targets,
            "the linked browser engine",
            section,
            |engine: &Option<ResolvedWebViewBackend>| {
                engine.map_or_else(|| "none".to_string(), |e| e.to_string())
            },
            |project, target| async move { project.linked_browser_engine(&target).await },
        )
        .await
    }

    /// Whether the generated backend manifests declare the CEF subprocess
    /// helper `[[bin]]` for a build of `target`.
    ///
    /// This is the manifest's own predicate: the helper exists only when the
    /// application links the CEF engine crate, while a `waterui-chromium`
    /// link alone does not declare it. Builds and packaging that touch the
    /// helper must gate on this rather than
    /// [`BrowserRuntimePlan::requires_cef`], which is wider — it also turns
    /// on for chromium — and would request a bin target Cargo never
    /// received.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo metadata cannot be resolved or the
    /// application links two engines at once.
    pub async fn declares_cef_helper(&self, target: &Triple) -> eyre::Result<bool> {
        Ok(crate::project_types::declares_cef_helper(
            self.linked_browser_engine(target).await?,
        ))
    }

    /// Resolves and validates every embedded browser runtime linked by the
    /// application for a build of `target`.
    ///
    /// # Errors
    ///
    /// Returns an error when standard `WebView` or Chromium is unsupported for
    /// the requested platform and backend.
    pub async fn browser_runtime_plan(
        &self,
        platform: TargetPlatform,
        backend: TargetBackend,
        target: &Triple,
    ) -> eyre::Result<BrowserRuntimePlan> {
        let webview = self
            .resolved_webview_backend(platform, backend, target)
            .await?;
        let chromium = self
            .links_runtime_package(target, "waterui-chromium")
            .await?;
        if chromium && !cef_is_supported(platform, backend) {
            eyre::bail!(
                "waterui-chromium requires CEF, which is unsupported for platform {platform:?} \
                 with backend {backend:?}"
            );
        }
        Ok(BrowserRuntimePlan { webview, chromium })
    }

    /// Get the bundle identifier of the project.
    #[must_use]
    pub const fn bundle_identifier(&self) -> &BundleIdentifier {
        &self.manifest.package.bundle_identifier
    }

    /// Get the assets directory path relative to project root.
    #[must_use]
    pub fn assets_path(&self) -> &str {
        &self.manifest.package.assets_path
    }

    /// Get the full path to the assets directory.
    #[must_use]
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join(&self.manifest.package.assets_path)
    }

    /// Clean build artifacts for the project using the specified backend.
    ///
    /// # Errors
    ///
    /// Returns an error if cleaning fails.
    pub async fn clean<B: Backend>(
        &self,
        backend: &B,
        platform: TargetPlatform,
    ) -> Result<(), eyre::Report> {
        backend.clean(self, platform).await
    }

    /// The names of every crate the CLI generates for this project: the
    /// backend, FFI, preview and launcher crates. Each is tagged with this
    /// project's root (see [`generated_crate_name`]) unless `[crates]`
    /// overrides it, so their units in the shared Cargo target directory are
    /// this project's alone.
    #[must_use]
    pub fn generated_crate_names(&self) -> Vec<String> {
        let mut names: Vec<String> = [
            self.ffi_crate_name(),
            self.preview_ffi_crate_name(),
            self.apple_preview_crate_name(),
            self.gtk_backend_crate_name(),
            self.hydrolysis_backend_crate_name(),
            self.winui_backend_crate_name(),
            self.esp32_backend_crate_name(),
            self.tui_backend_crate_name(),
        ]
        .into_iter()
        .map(|name| name.as_str().to_owned())
        .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Remove this project's own units from the shared Cargo target directory.
    ///
    /// The shared target is one directory for every project on the machine,
    /// so only units Cargo named after this project's generated crates go;
    /// dependency artifacts stay for the other projects that resolve them.
    async fn clean_shared_target_units(&self) -> Result<(), eyre::Report> {
        let removed = crate::water_dir::remove_project_units_from_shared_target(
            self.host(),
            &self.generated_crate_names(),
        )
        .await?;
        for path in &removed {
            info!(path = %path.display(), "removed this project's unit from the shared target");
        }
        Ok(())
    }

    /// Clean all build artifacts for the project.
    ///
    /// This removes this project's own units in the shared Cargo target
    /// directory and its managed build cache, where every generated backend
    /// lives. Dependency artifacts in the shared Cargo target directory are
    /// left in place: every project on the machine resolves them identically,
    /// and `water gc build-cache --shared-target` drops them all.
    ///
    /// # Errors
    ///
    /// Returns an error if any cleaning operation fails.
    pub async fn clean_all(&self) -> Result<(), eyre::Report> {
        // The generated backends' units go first: once the managed manifests
        // below are gone, nothing else names them.
        self.clean_shared_target_units().await?;
        crate::water_dir::remove_project_build_cache(self.host(), self.root()).await?;
        // What remains to sweep is the `water-backends` subtree older CLI
        // layouts left under the project's own Cargo target directory — never
        // the user's other compiled artifacts.
        let water_backends_root = self.target_dir().await?.join("water-backends");
        if water_backends_root.exists() {
            smol::fs::remove_dir_all(&water_backends_root).await?;
        }
        Ok(())
    }

    /// Package the project for the specified platform.
    ///
    /// # Errors
    ///
    /// Returns an error if packaging fails.
    pub async fn package<B: Backend>(
        &self,
        backend: &B,
        platform: TargetPlatform,
        options: PackageOptions,
        built: &crate::build::BuiltTarget,
    ) -> Result<Artifact, eyre::Report> {
        backend.package(self, platform, options, built).await
    }

    fn app_crate_overrides(&self) -> Option<&AppCrates> {
        self.manifest.app.as_ref()?.crates.as_ref()
    }
}

/// Errors that can occur when opening a `WaterUI` project.
#[derive(Debug, thiserror::Error)]
pub enum FailToOpenProject {
    /// Failed to open the Water.toml manifest.
    #[error("Failed to open project manifest: {0}")]
    Manifest(FailToOpenManifest),
    /// Failed to read the Cargo.toml file.
    #[error("Failed to read Cargo.toml: {0}")]
    CargoManifest(cargo_toml::Error),

    /// Failed to get Cargo metadata.
    #[error("Failed to get Cargo metadata: {0}")]
    TargetDirError(#[from] cargo_metadata::Error),

    /// The selected framework could not be validated for this CLI.
    #[error("Framework compatibility check failed: {0}")]
    Framework(eyre::Report),
    /// The project's `[patch]` tables could not be brought in line with the
    /// local checkout's.
    #[error("Failed to refresh the [patch] tables from the local checkout: {0}")]
    LocalPatches(eyre::Report),

    /// A canonical `backends/<name>` slot under the `waterui_path` checkout
    /// is present but not a usable backend source.
    #[error("Invalid local backend source under `waterui_path`: {0}")]
    LocalSources(eyre::Report),

    /// Missing crate name in Cargo.toml.
    #[error("Invalid Cargo.toml: missing crate name")]
    MissingCrateName,

    /// Crate name in Cargo.toml is invalid.
    #[error("Invalid Cargo.toml crate name: {0}")]
    InvalidCrateName(String),

    /// Failed to initialize a managed backend.
    #[error("Failed to initialize backend: {0}")]
    BackendInit(#[from] crate::backend::FailToInitBackend),

    /// Failed to manage the global build cache directory.
    #[error("Failed to prepare managed build cache: {0}")]
    BuildCache(#[from] eyre::Report),
}

/// Errors that can occur when creating a new `WaterUI` project.
#[derive(Debug, thiserror::Error)]
pub enum FailToCreateProject {
    /// Failed to resolve a coherent framework distribution.
    #[error("Failed to resolve framework: {0}")]
    Framework(eyre::Report),
    /// A canonical `backends/<name>` slot under the `waterui_path` checkout
    /// is present but not a usable backend source.
    #[error("Invalid local backend source under `waterui_path`: {0}")]
    LocalSources(eyre::Report),
    /// The project directory already exists.
    #[error("Directory already exists: {0}")]
    DirectoryExists(PathBuf),
    /// The directory is already a `WaterUI` project.
    #[error("{0} is already a WaterUI project (Water.toml exists)")]
    AlreadyProject(PathBuf),
    /// The directory already contains a Cargo manifest that scaffolding
    /// would overwrite.
    #[error(
        "{0} already contains a Cargo.toml; merge the generated scaffold manually or remove it first"
    )]
    CargoManifestExists(PathBuf),
    /// The display name derives a crate name Cargo rejects as a package
    /// name — a leading digit is the common case. The derivation runs ahead
    /// of every scaffold write, so a rejected name leaves nothing behind.
    #[error(
        "the crate name '{derived}' derived from the project name '{name}' is not usable: {reason}"
    )]
    UnderivableCrateName {
        /// The project display name.
        name: String,
        /// The crate name the derivation produced.
        derived: String,
        /// Why Cargo rejects it as a package name.
        reason: String,
    },
    /// Failed to create project directory.
    #[error("Failed to create directory: {0}")]
    CreateDir(std::io::Error),
    /// Failed to scaffold project files.
    #[error("Failed to scaffold project: {0}")]
    Scaffold(std::io::Error),
    /// Failed to save manifest.
    #[error("Failed to save manifest: {0}")]
    SaveManifest(#[from] FailToSaveManifest),

    /// Failed to get Cargo metadata.
    #[error("Failed to get Cargo metadata: {0}")]
    TargetDirError(#[from] cargo_metadata::Error),

    /// Failed to resolve the managed build cache path.
    #[error("Failed to resolve managed build cache: {0}")]
    BuildCache(#[from] eyre::Report),

    /// Failed to initialize git repository.
    #[error("Failed to initialize git repository: {0}")]
    GitInit(std::io::Error),
    /// Failed to check git repository status.
    #[error("Failed to check git repository status: {0}")]
    GitStatus(std::io::Error),
    /// A generated crate a build compiles could not be scaffolded.
    #[error("Failed to scaffold the project's generated crates: {0:#}")]
    ScaffoldGeneratedCrates(eyre::Report),
    /// The project's declared fonts could not be read or fetched.
    #[error("Failed to fetch the project's declared fonts: {0:#}")]
    FetchFonts(eyre::Report),
    /// Declared fonts could not be satisfied by fetching.
    #[error(
        "{} declared font(s) cannot be satisfied by fetching — the build reports them the same way:{}",
        .0.len(),
        bullet_list(.0)
    )]
    UnsatisfiableFonts(
        /// Reports for every declaration that cannot be satisfied.
        Vec<eyre::Report>,
    ),
    /// Creation failed and its partial output could not be removed.
    #[error(
        "{error:#}; removing the partially created project at {} also failed: {cleanup:#}",
        root.display()
    )]
    Rollback {
        /// The error that caused creation to fail.
        error: eyre::Report,
        /// The partially created project root.
        root: PathBuf,
        /// The error encountered while removing partial output.
        cleanup: eyre::Report,
    },
}

fn bullet_list(errors: &[eyre::Report]) -> String {
    let mut bullets = String::new();
    for error in errors {
        write!(bullets, "\n  - {error:#}").expect("writing to a string cannot fail");
    }
    bullets
}

/// Options for creating a new `WaterUI` project.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    /// Application display name (e.g., "Water Example").
    pub name: String,
    /// Bundle identifier (e.g., "dev.waterui.waterexample").
    pub bundle_identifier: BundleIdentifier,
    /// Path to local `WaterUI` repository for development.
    pub waterui_path: Option<PathBuf>,
    /// Framework channel, mutually exclusive with a local source path and a
    /// manifest file.
    pub channel: Option<FrameworkChannel>,
    /// A certified `framework.json` on disk, mutually exclusive with a channel
    /// and a local source path: the project pins the channel and revision the
    /// manifest declares.
    pub framework_manifest: Option<PathBuf>,
    /// An already-resolved framework selection — how a support app inherits
    /// the host project's framework exactly. Mutually exclusive with every
    /// resolving source above.
    pub framework: Option<ResolvedFramework>,
    /// The `Water.lock` bytes `framework` was resolved against: a caller
    /// passing `framework` carries the host project's lock verbatim, so the
    /// scaffolded project's `Water.lock`/`Cargo.lock` match it.
    pub framework_lock: Option<Vec<u8>>,
    /// Author name for Cargo.toml.
    pub author: String,
    /// The declared web frontend: `Some` generates the `include_web!` root
    /// view and writes `[web] package_manager`.
    pub web: Option<WebScaffold>,
}

/// How `create`/`init` wires a declared web frontend into the scaffold.
#[derive(Debug, Clone)]
pub struct WebScaffold {
    /// The package manager written to `[web] package_manager`.
    pub package_manager: web::PackageManager,
    /// The `include_web!` argument: `"web"` for the conventional layout, or a
    /// path relative to the project root for a frontend referenced in place.
    pub include_arg: String,
}

/// A project `water create` has written but not finished: its directory and
/// build-cache container belong to this create, so any failure before
/// [`ProjectDraft::finish`] succeeds removes both.
#[must_use = "a draft is finished or discarded; dropping it keeps a half-created project"]
#[derive(Debug)]
pub struct ProjectDraft {
    project: Project,
}

impl ProjectDraft {
    /// Create a draft using the given host.
    ///
    /// # Errors
    /// Returns an error if project scaffolding fails. Any partial project and
    /// its managed build-cache container are removed before returning.
    pub async fn create(
        host: &Host,
        path: impl AsRef<Path>,
        options: CreateOptions,
    ) -> Result<Self, FailToCreateProject> {
        Project::create_on(host, path, options)
            .await
            .map(|project| Self { project })
    }

    /// The project being created.
    #[must_use]
    pub const fn project(&self) -> &Project {
        &self.project
    }

    /// Scaffold every generated crate a build compiles and seed declared fonts.
    ///
    /// # Errors
    /// Returns an error if generated-crate scaffolding, font declaration
    /// scanning, font fetching, or cleanup fails. A failure removes the
    /// project and its managed build-cache container.
    pub async fn finish(self) -> Result<Vec<(String, PathBuf)>, FailToCreateProject> {
        let outcomes = match crate::seed_font_cache(&self.project).await {
            Ok(outcomes) => outcomes,
            Err(crate::SeedFontCacheError::Scaffold(error)) => {
                return Err(self
                    .fail(FailToCreateProject::ScaffoldGeneratedCrates(error))
                    .await);
            }
            Err(crate::SeedFontCacheError::Fonts(error)) => {
                return Err(self.fail(FailToCreateProject::FetchFonts(error)).await);
            }
        };

        let mut fetched = Vec::new();
        let mut unsatisfiable = Vec::new();
        for outcome in outcomes {
            match outcome {
                crate::FetchOutcome::Satisfied { .. } => {}
                crate::FetchOutcome::Fetched { name, path } => fetched.push((name, path)),
                crate::FetchOutcome::Unsatisfiable { error, .. } => unsatisfiable.push(error),
            }
        }
        if !unsatisfiable.is_empty() {
            return Err(self
                .fail(FailToCreateProject::UnsatisfiableFonts(unsatisfiable))
                .await);
        }
        Ok(fetched)
    }

    /// Remove this draft after a failure outside it and return that error.
    ///
    /// # Errors
    /// The returned report wraps `error` in a rollback error if removing the
    /// project or its managed build-cache container fails.
    pub async fn discard(self, error: eyre::Report) -> eyre::Report {
        let root = self.project.root.clone();
        match remove_project_and_cache(&self.project.host, &root).await {
            Ok(()) => error,
            Err(cleanup) => eyre::Report::new(FailToCreateProject::Rollback {
                error,
                root,
                cleanup,
            }),
        }
    }

    async fn fail(self, error: FailToCreateProject) -> FailToCreateProject {
        let root = self.project.root.clone();
        roll_back(&self.project.host, root, error).await
    }
}

async fn remove_project_and_cache(host: &Host, root: &Path) -> eyre::Result<()> {
    let cache_container = crate::water_dir::build_cache_container_for_on(host, root)?;
    if root.exists() {
        smol::fs::remove_dir_all(root).await.wrap_err_with(|| {
            format!(
                "failed to remove the partially created project at {}",
                root.display()
            )
        })?;
    }
    if cache_container.exists() {
        smol::fs::remove_dir_all(&cache_container)
            .await
            .wrap_err_with(|| {
                format!(
                    "failed to remove the managed build-cache container at {}",
                    cache_container.display()
                )
            })?;
    }

    Ok(())
}

async fn roll_back(host: &Host, root: PathBuf, error: FailToCreateProject) -> FailToCreateProject {
    match remove_project_and_cache(host, &root).await {
        Ok(()) => error,
        Err(cleanup) => FailToCreateProject::Rollback {
            error: error.into(),
            root,
            cleanup,
        },
    }
}

impl CreateOptions {
    /// The Cargo package name `water create` derives from the display name.
    ///
    /// # Errors
    ///
    /// Returns [`FailToCreateProject::UnderivableCrateName`] when the derived
    /// name is not a valid package name.
    pub fn crate_name(&self) -> Result<CrateName, FailToCreateProject> {
        let name = self
            .name
            .chars()
            .map(|character| {
                if character.is_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        CrateName::try_from(name.clone()).map_err(|reason| {
            FailToCreateProject::UnderivableCrateName {
                name: self.name.clone(),
                derived: name,
                reason,
            }
        })
    }

    /// The framework the scaffold resolves against — always resolved: a
    /// channel's certified release, a manifest file's, a caller-supplied
    /// selection, or the checkout `waterui_path` names. A checkout's framework
    /// is a filesystem source, so it is never persisted into `Water.toml`;
    /// `waterui_path` itself is the record.
    async fn resolve_framework(
        &mut self,
        host: &Host,
    ) -> eyre::Result<(ResolvedFramework, Option<Vec<u8>>)> {
        let selected = [
            self.waterui_path.is_some(),
            self.channel.is_some(),
            self.framework_manifest.is_some(),
            self.framework.is_some(),
        ]
        .into_iter()
        .filter(|selected| *selected)
        .count();
        if selected > 1 {
            eyre::bail!(
                "a framework channel, a local source path, a framework manifest, \
                 and a resolved framework are mutually exclusive"
            );
        }
        if let Some(path) = &self.waterui_path {
            // `dunce`, not `std`'s canonicalize: on Windows the standard one
            // returns an extended-length path (`\\?\C:\…`), and a scaffolded
            // manifest that carries it as a dependency `path` is one Cargo
            // refuses to parse ("invalid path url").
            let path = path.clone();
            let root = unblock(move || dunce::canonicalize(path)).await?;
            self.waterui_path = Some(root.clone());
            return Ok((
                ResolvedFramework::for_local_checkout(host, &root).await?,
                None,
            ));
        }
        if let Some(path) = &self.framework_manifest {
            return ResolvedFramework::resolve_manifest(host, path).await;
        }
        if let Some(framework) = &self.framework {
            return Ok((framework.clone(), self.framework_lock.take()));
        }
        ResolvedFramework::resolve(host, self.channel.unwrap_or_default(), None).await
    }
}

impl Project {
    /// The `TemplateContext` the generated crates this project builds for
    /// Apple targets share — the FFI companion and the Apple preview
    /// package read one context so their generated dependency tables cannot
    /// drift.
    ///
    /// The render is a function of the project, not of the invocation: on
    /// macOS the Apple pieces — the `waterui-apple` pin included — render
    /// into every FFI companion render, whatever the invocation selected,
    /// because that is the only host an Apple build can run from. A host
    /// that cannot produce an Apple build must not resolve the Apple
    /// backend crate. The graph-derived answers resolve from the triples
    /// each manifest section serves: the `cfg(target_os = "macos")`
    /// table's answers come from the macOS build, and `profile_targets`
    /// is the serving set of the shared `[profile]` package overrides.
    async fn apple_managed_crate_context(
        &self,
        backend_project_path: PathBuf,
        profile_targets: &[Triple],
    ) -> Result<(TemplateContext, ResolvedFramework), crate::backend::FailToInitBackend> {
        let apple_selected = cfg!(target_os = "macos");
        let manifest = self.manifest();
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        // The managed Apple crates' sections read only the engine their
        // macOS table links — `cef_runtime_enabled` — so the context
        // records that answer alone; the `--edges features` resolve that
        // would answer `webview_enabled` feeds nothing here.
        let (engine, framework) = futures_util::future::try_join(
            self.linked_browser_engine_for(
                &crate::platform::NativeOs::MacOs.serving_triples(),
                GraphSection {
                    manifest: "the generated FFI companion manifest",
                    table: crate::platform::NativeOs::MacOs.cfg(),
                    remedy: "that section needs a per-target split inside its own OS — a \
                             narrower cfg separates the disagreeing targets",
                },
            )
            .map_err(crate::backend::FailToInitBackend::Config),
            self.resolved_framework()
                .map_err(crate::backend::FailToInitBackend::Config),
        )
        .await?;
        let ctx = TemplateContext::for_project_manifest(
            self.host(),
            manifest,
            self.crate_name().clone(),
            app_name,
            &framework,
            self.local_sources(),
        )
        .with_backend_project_path(backend_project_path)
        .with_project_root_path(self.root.clone())
        .with_project_packages(
            self.project_packages_for(&framework, profile_targets)
                .await
                .map_err(crate::backend::FailToInitBackend::Config)?,
        )
        .with_apple_backend_selected(apple_selected)
        .with_host(self.host.clone())
        .with_browser(crate::templates::BrowserTemplateContext::apple_managed(
            engine,
        ));
        Ok((ctx, framework))
    }

    /// Seed `crate_dir`'s `Cargo.lock` from the project's lock and the
    /// channel's certified `Water.lock` — the lock every managed crate
    /// resolves with.
    async fn seed_managed_crate_lock(
        &self,
        crate_dir: &Path,
        framework: &ResolvedFramework,
    ) -> Result<(), crate::backend::FailToInitBackend> {
        let lockfile = self
            .lockfile_path()
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;
        let canonical = framework
            .canonical_lock(&self.root)
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;
        templates::seed_lockfile(crate_dir, &lockfile, canonical.as_ref())
            .await
            .map_err(crate::backend::FailToInitBackend::Io)
    }

    /// Scaffold the managed native FFI companion crate.
    ///
    /// The crate is a pure function of the project: every path that builds,
    /// packages, previews or font-scans it renders it the same way, so the
    /// manifest does not change between a macOS build and an Android build
    /// of one project.
    ///
    /// # Errors
    ///
    /// Returns an error when the generated crate cannot be written.
    pub(crate) async fn scaffold_ffi_companion(
        &self,
    ) -> Result<(), crate::backend::FailToInitBackend> {
        let (ctx, framework) = self
            .apple_managed_crate_context(self.ffi_crate_path(), &ffi_companion_targets())
            .await?;

        templates::ffi::scaffold(&self.ffi_crate_path(), &ctx, &self.ffi_crate_name())
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;

        self.seed_managed_crate_lock(&self.ffi_crate_path(), &framework)
            .await
    }

    /// Scaffold the managed Apple in-process preview package `water preview
    /// --platform macos` builds and execs.
    ///
    /// The package sits next to the FFI companion in the managed build
    /// cache and shares its dependency tables through
    /// `templates::apple_preview`, so its binary resolves the same
    /// `waterui` and `libwaterui_dylib` `water run` produces. Its lock is
    /// seeded like the other managed crates.
    ///
    /// # Errors
    ///
    /// Returns an error when the generated crate cannot be written.
    pub(crate) async fn scaffold_apple_preview_companion(
        &self,
    ) -> Result<(), crate::backend::FailToInitBackend> {
        let (ctx, framework) = self
            .apple_managed_crate_context(
                self.apple_preview_crate_path(),
                &[TargetPlatform::MacOS.triple()],
            )
            .await?;

        templates::apple_preview::scaffold(
            &self.apple_preview_crate_path(),
            &ctx,
            &self.apple_preview_crate_name(),
        )
        .await
        .map_err(crate::backend::FailToInitBackend::Io)?;

        self.seed_managed_crate_lock(&self.apple_preview_crate_path(), &framework)
            .await
    }

    /// Scaffold this project's preview module inside `workspace_root`.
    ///
    /// # Errors
    ///
    /// Returns an error when the generated crate cannot be written.
    pub async fn scaffold_preview_ffi_companion(
        &self,
        workspace_root: &Path,
    ) -> Result<PathBuf, crate::backend::FailToInitBackend> {
        let manifest = self.manifest();
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let framework = self
            .resolved_framework()
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;
        let ctx = TemplateContext::for_project_manifest(
            self.host(),
            manifest,
            self.crate_name().clone(),
            app_name,
            &framework,
            self.local_sources(),
        )
        .with_backend_project_path(self.preview_ffi_crate_path(workspace_root))
        .with_project_root_path(self.root.clone());

        let crate_path = self.preview_ffi_crate_path(workspace_root);
        templates::preview_ffi::scaffold(&crate_path, &ctx, &self.preview_ffi_crate_name())
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;
        Ok(crate_path)
    }

    /// Create a new `WaterUI` project at the specified path.
    ///
    /// This creates the project directory, scaffolds root files (Cargo.toml, src/lib.rs),
    /// and saves the Water.toml manifest. Backend projects are generated and
    /// managed by the CLI in the build cache, never in the project directory.
    ///
    /// # Errors
    /// - `FailToCreateProject::DirectoryExists`: If the directory already exists.
    /// - `FailToCreateProject::CreateDir`: If creating the directory fails.
    /// - `FailToCreateProject::Scaffold`: If scaffolding files fails.
    /// - `FailToCreateProject::SaveManifest`: If saving the manifest fails.
    /// - `FailToCreateProject::Rollback`: If the partial project cannot be removed after failure.
    pub async fn create(
        host: &Host,
        path: impl AsRef<Path>,
        options: CreateOptions,
    ) -> Result<Self, FailToCreateProject> {
        Self::create_on(host, path, options).await
    }

    async fn create_on(
        host: &Host,
        path: impl AsRef<Path>,
        options: CreateOptions,
    ) -> Result<Self, FailToCreateProject> {
        let path = path.as_ref().to_path_buf();

        if path.exists() {
            return Err(FailToCreateProject::DirectoryExists(path));
        }

        match Self::scaffold_project(host, path.clone(), options).await {
            Ok(project) => Ok(project),
            Err(error) if path.exists() => Err(roll_back(host, path, error).await),
            Err(error) => Err(error),
        }
    }

    /// Initialize a `WaterUI` project inside an existing directory
    /// (`water init`): the same scaffold as [`Project::create`] without the
    /// directory-creation step.
    ///
    /// # Errors
    /// - `FailToCreateProject::AlreadyProject`: If `Water.toml` already exists.
    /// - `FailToCreateProject::CargoManifestExists`: If `Cargo.toml` already
    ///   exists and would be overwritten.
    /// - the [`Project::create`] scaffold errors.
    pub async fn init(
        host: &Host,
        path: impl AsRef<Path>,
        options: CreateOptions,
    ) -> Result<Self, FailToCreateProject> {
        let path = path.as_ref().to_path_buf();
        if path.join("Water.toml").exists() {
            return Err(FailToCreateProject::AlreadyProject(path));
        }
        if path.join("Cargo.toml").exists() {
            return Err(FailToCreateProject::CargoManifestExists(path));
        }
        Self::scaffold_project(host, path, options).await
    }

    async fn scaffold_project(
        host: &Host,
        path: PathBuf,
        mut options: CreateOptions,
    ) -> Result<Self, FailToCreateProject> {
        // Derive crate name from display name
        let crate_name = options.crate_name()?;
        let (framework, lockfile) = options
            .resolve_framework(host)
            .await
            .map_err(FailToCreateProject::Framework)?;

        // Framework validation precedes directory creation so a rejected
        // local checkout leaves nothing behind; on `init` the directory
        // already exists and this is a no-op.
        smol::fs::create_dir_all(&path)
            .await
            .map_err(FailToCreateProject::CreateDir)?;

        // The canonical local source slots a `waterui_path` checkout
        // declares are validated once here — a malformed slot is an error,
        // never a silent remote fallback during template rendering.
        let local_sources =
            crate::templates::project_local_backend_sources(options.waterui_path.as_deref(), &path)
                .await
                .map_err(FailToCreateProject::LocalSources)?;

        // Build template context for root files
        let ctx = TemplateContext::for_create_options(
            host,
            &options,
            crate_name.clone(),
            &framework,
            &local_sources,
        );

        // The assets root is derived once and shared with both the scaffold and
        // the manifest, so the created directory and `Water.toml` cannot disagree.
        let assets_path = default_assets_path();

        // Scaffold root files (Cargo.toml, src/lib.rs, .gitignore, assets/README.md)
        templates::root::scaffold(&path, &ctx, &assets_path)
            .await
            .map_err(FailToCreateProject::Scaffold)?;

        // `.mcp.json` lets MCP clients launched in the project root find
        // `water mcp` without any user configuration.
        crate::mcp::ensure_mcp_json(&path)
            .await
            .map_err(FailToCreateProject::Scaffold)?;
        if let Some(lockfile) = lockfile {
            let contents = ctx
                .framework
                .cargo_lock(&lockfile)
                .map_err(FailToCreateProject::Framework)?
                .to_string();
            smol::fs::write(path.join("Water.lock"), lockfile)
                .await
                .map_err(FailToCreateProject::Scaffold)?;
            smol::fs::write(path.join("Cargo.lock"), contents)
                .await
                .map_err(FailToCreateProject::Scaffold)?;
        }

        let manifest = Self::scaffold_manifest(&options, framework, assets_path);

        // Save Water.toml
        manifest.save(&path).await?;

        // Initialize git repository if not already in one
        Self::ensure_git_init(host, &path).await?;

        let managed_backends_root = crate::water_dir::project_build_cache_dir(host, &path)
            .await
            .map_err(FailToCreateProject::BuildCache)?;

        let cargo_layout = if let Some(framework) = &manifest.framework {
            let layout = resolve_cargo_layout(
                host,
                &path,
                Some(framework.clone()),
                CargoResolution::Update,
            )
            .await
            .map_err(FailToCreateProject::Framework)?;
            futures_util::future::ready(Ok::<CargoLayout, String>(layout))
                .boxed()
                .shared()
        } else {
            spawn_cargo_layout_resolution(host, &path, None, true)
        };
        let graph_cache_dir = crate::water_dir::project_graph_cache_dir_on(host, &path)
            .map_err(FailToCreateProject::BuildCache)?;
        Ok(Self {
            host: host.clone(),
            root: path,
            manifest,
            crate_name,
            cargo_layout,
            linked_packages: Arc::new(async_lock::Mutex::new(BTreeMap::new())),
            enabled_features: Arc::new(async_lock::Mutex::new(BTreeMap::new())),
            generated_features: Arc::new(async_lock::Mutex::new(BTreeMap::new())),
            cargo_resolve_permits: Arc::new(async_lock::Semaphore::new(
                Self::CARGO_RESOLVE_PERMITS,
            )),
            managed_backends_root,
            graph_cache_dir,
            backends: Backends::default(),
            local_sources,
        })
    }

    /// The `Water.toml` a scaffold records: the create options' identity and
    /// web section, the resolved framework's channel when it names one.
    fn scaffold_manifest(
        options: &CreateOptions,
        framework: ResolvedFramework,
        assets_path: String,
    ) -> Manifest {
        Manifest {
            package: Package {
                name: options.name.clone(),
                bundle_identifier: options.bundle_identifier.clone(),
                assets_path,
                accessory: false,
                embedded: false,
            },
            esp32: None,
            hydrolysis: None,
            waterui_path: options
                .waterui_path
                .as_ref()
                .map(|p| p.display().to_string()),
            // The scaffold's copy of the checkout's tables is identical to the
            // checkout's, which the first open adopts and records.
            waterui_patches: cargo_toml::PatchSet::new(),
            // A local checkout's framework is a filesystem source — never
            // persisted; `waterui_path` above is the record.
            framework: framework.channel().is_some().then_some(framework),
            permissions: BTreeMap::default(),
            app: None,
            theme: None,
            launch: None,
            web: options.web.as_ref().map(|scaffold| web::WebConfig {
                package_manager: scaffold.package_manager,
            }),
            signing: SigningConfig::default(),
            assets: None,
            app_values: crate::assets::AppValuesConfig::default(),
        }
    }

    /// Ensure the project is initialized with git.
    ///
    /// Checks if the project directory is already part of a git repository.
    /// If not, initializes a new git repository.
    async fn ensure_git_init(host: &Host, path: &Path) -> Result<(), FailToCreateProject> {
        // Check if already in a git repository

        let mut cmd = host.command("git");

        let is_in_git = command(&mut cmd)
            .args(["rev-parse", "--git-dir"])
            .current_dir(path)
            .output()
            .await
            .map_err(FailToCreateProject::GitStatus)?
            .status
            .success();

        if !is_in_git {
            // Initialize a new git repository
            let mut cmd = host.command("git");
            command(&mut cmd)
                .args(["init"])
                .current_dir(path)
                .status()
                .await
                .map_err(FailToCreateProject::GitInit)?;
        }

        Ok(())
    }

    /// Select the ESP32 target chip, persisting it to `Water.toml`.
    ///
    /// The chip is the single source of truth for the ESP32 backend's target
    /// triple, QEMU model, and firmware parameters. Selecting a platform such
    /// as `esp32c3` calls this so the generated harness and build target follow
    /// the platform. No-ops (and skips the manifest write) when the configured
    /// chip already matches.
    ///
    /// # Errors
    /// Returns an error if saving the manifest fails.
    pub async fn set_esp32_chip(
        &mut self,
        chip: crate::esp32::chip::Esp32Chip,
    ) -> eyre::Result<()> {
        let current = self.esp32_config().cloned().unwrap_or_default();
        if current.chip() == chip.id() {
            return Ok(());
        }
        self.manifest.esp32 = Some(current.with_chip(chip));
        self.save_manifest().await
    }

    /// Open a `WaterUI` project located at the specified path.
    ///
    /// This loads both the `Water.toml` manifest and the `Cargo.toml` file.
    /// The managed backends in `backends` — those the caller's target
    /// platforms need — are initialised; the accessor of a
    /// backend not selected returns `None`.
    ///
    /// # Errors
    /// - `FailToOpenProject::Manifest`: If there was an error opening the `Water.toml` manifest.
    /// - `FailToOpenProject::CargoManifest`: If there was an error reading the `Cargo.toml` file.
    /// - `FailToOpenProject::MissingCrateName`: If the crate name is missing in `Cargo.toml`.
    pub async fn open(
        host: &Host,
        path: impl AsRef<Path>,
        backends: ManagedBackends,
    ) -> Result<Self, FailToOpenProject> {
        Self::open_with_mode(host, path, OpenMode::Full, backends).await
    }

    /// Open a project for preview dylib builds without initializing native app backends.
    ///
    /// Preview dylib builds only need the managed preview wrapper crate. Native
    /// backend initialization is reserved for support app projects that actually launch apps.
    ///
    /// # Errors
    /// - `FailToOpenProject::Manifest`: If there was an error opening the `Water.toml` manifest.
    /// - `FailToOpenProject::CargoManifest`: If there was an error reading the `Cargo.toml` file.
    /// - `FailToOpenProject::MissingCrateName`: If the crate name is missing in `Cargo.toml`.
    pub async fn open_for_preview_build(
        host: &Host,
        path: impl AsRef<Path>,
    ) -> Result<Self, FailToOpenProject> {
        Self::open_with_mode(host, path, OpenMode::PreviewBuild, ManagedBackends::NONE).await
    }

    /// Keep the checkout's entries in a local-checkout project's `[patch]`
    /// tables current, beside the project's own.
    ///
    /// Cargo applies `[patch]` only from the workspace it builds, so a project
    /// on a `waterui_path` carries a copy of the checkout's tables, and the
    /// copy has to follow the checkout: a fork pin moves, an entry is added or
    /// dropped, and a project scaffolded earlier would otherwise build a graph
    /// the checkout no longer produces, silently. `Water.toml` records the
    /// copy last written under `waterui_patches`, so the entries the CLI owns
    /// are known exactly: they are replaced with the checkout's current set,
    /// and every other entry is the project's own and stays (#1997). An entry
    /// identical to the checkout's is the checkout's. A project entry for a
    /// crate the checkout patches differently overrides it, as Cargo's root
    /// `[patch]` does: the checkout's entry is not written beside it, and the
    /// override is logged. The manifests are rewritten only when they change,
    /// so an up-to-date project stays untouched.
    ///
    /// A project that is itself a member of the checkout's workspace — every
    /// example in this repository — needs no copy, because the
    /// tables Cargo reads are the checkout's own. Writing one anyway put a
    /// `[patch.crates-io]` table into a member manifest, where Cargo ignores it
    /// and says so on every single build.
    async fn refresh_local_patches(
        project_root: &Path,
        waterui_path: &Path,
        written: &cargo_toml::PatchSet,
    ) -> eyre::Result<()> {
        let project_root = project_root.to_path_buf();
        let waterui_path = waterui_path.to_path_buf();
        let written = written.clone();
        unblock(move || {
            let checkout = project_root.join(&waterui_path);
            let patch_root = templates::patch_manifest_dir(&project_root)?;
            if same_directory(&patch_root, &checkout)? {
                return Ok(());
            }
            if !same_directory(&patch_root, &project_root)? {
                // Cargo reads `[patch]` from `patch_root` and nothing this
                // function writes into the project could change that, so the
                // honest move is to say which manifest the tables belong in
                // rather than write a copy that is read by nobody.
                eyre::bail!(
                    "This project is a member of the Cargo workspace at {}, so Cargo reads \
                     [patch] from {} and ignores any copy here. Move the WaterUI checkout's \
                     [patch] tables — the ones in {} — into that workspace manifest, or take \
                     the project out of that workspace.",
                    patch_root.display(),
                    patch_root.join("Cargo.toml").display(),
                    checkout.join("Cargo.toml").display(),
                );
            }
            let cargo_path = project_root.join("Cargo.toml");
            let text = std::fs::read_to_string(&cargo_path)?;
            let current = CargoManifest::from_slice(text.as_bytes())?.patch;
            let next = templates::local_framework_patches(&project_root, &waterui_path)?;
            let own = crate::patch_tables::beyond(&current, &written);
            let own = crate::patch_tables::beyond(&own, &next);
            // The checkout's entries the project does not override: exactly
            // what this refresh writes, and so what it records.
            let copy = crate::patch_tables::without(&next, &own);
            let tables = crate::patch_tables::merge(next, own.clone());
            if current != tables {
                // Only the checkout's entries are rewritten; the project's own
                // keep their spelling.
                let mut document: toml_edit::DocumentMut = text.parse()?;
                let copied = crate::patch_tables::beyond(&current, &own);
                crate::framework::rewrite_patch_tables(&mut document, &copied, &copy)?;
                std::fs::write(&cargo_path, document.to_string())?;
                info!(
                    path = %cargo_path.display(),
                    "Refreshed the [patch] tables from the local checkout"
                );
            }
            if written != copy {
                let water_path = project_root.join("Water.toml");
                let mut water: toml_edit::DocumentMut =
                    std::fs::read_to_string(&water_path)?.parse()?;
                water.remove(WATERUI_PATCHES_KEY);
                // Rendered the way `Manifest::save` renders the whole file —
                // one `[waterui_patches.<source>.<crate>]` table per entry —
                // then spliced in, so the rest of the file keeps its spelling.
                let mut rendered: toml_edit::DocumentMut =
                    toml::to_string_pretty(&WateruiPatchesRecord { patches: &copy })?.parse()?;
                if let Some(record) = rendered.remove(WATERUI_PATCHES_KEY) {
                    water[WATERUI_PATCHES_KEY] = record;
                }
                std::fs::write(&water_path, water.to_string())?;
            }
            Ok(())
        })
        .await
    }

    #[expect(
        clippy::too_many_lines,
        reason = "each open mode is one linear sequence; splitting the dispatch would scatter the mode table"
    )]
    async fn open_with_mode(
        host: &Host,
        path: impl AsRef<Path>,
        open_mode: OpenMode,
        backends: ManagedBackends,
    ) -> Result<Self, FailToOpenProject> {
        use crate::backend::Backend;

        let total_start = std::time::Instant::now();
        let path = path.as_ref().to_path_buf();

        let manifest_start = std::time::Instant::now();
        let manifest = Manifest::open(path.join("Water.toml"))
            .await
            .map_err(FailToOpenProject::Manifest)?;
        if let Some(framework) = &manifest.framework {
            framework
                .validate_cli(host)
                .map_err(FailToOpenProject::Framework)?;
        }
        let local_sources = crate::templates::project_local_backend_sources(
            manifest.waterui_path.as_deref().map(Path::new),
            &path,
        )
        .await
        .map_err(FailToOpenProject::LocalSources)?;
        if let Some(local) = &manifest.waterui_path {
            validate_local_cli(host, &path.join(local))
                .await
                .map_err(FailToOpenProject::Framework)?;
            Self::refresh_local_patches(&path, Path::new(local), &manifest.waterui_patches)
                .await
                .map_err(FailToOpenProject::LocalPatches)?;
        }
        info!(
            path = %path.display(),
            open_mode = ?open_mode,
            elapsed_ms = manifest_start.elapsed().as_millis(),
            "Project::open loaded Water.toml"
        );

        let cargo_path = path.join("Cargo.toml");

        let cargo_manifest_start = std::time::Instant::now();
        let cargo_manifest = unblock(move || CargoManifest::from_path(cargo_path))
            .await
            .map_err(FailToOpenProject::CargoManifest)?;
        info!(
            path = %path.display(),
            open_mode = ?open_mode,
            elapsed_ms = cargo_manifest_start.elapsed().as_millis(),
            "Project::open loaded Cargo.toml"
        );
        let crate_name = cargo_manifest
            .package
            .map(|p| p.name)
            .ok_or(FailToOpenProject::MissingCrateName)
            .and_then(|value| {
                CrateName::try_from(value).map_err(FailToOpenProject::InvalidCrateName)
            })?;

        let cargo_layout = spawn_cargo_layout_resolution(
            host,
            &path,
            manifest.framework.clone(),
            manifest.waterui_path.is_some(),
        );
        cargo_layout
            .clone()
            .await
            .map_err(|error| FailToOpenProject::Framework(eyre::eyre!(error)))?;

        let build_cache_start = std::time::Instant::now();
        let managed_backends_root = crate::water_dir::ensure_project_build_cache(host, &path)
            .await
            .map_err(FailToOpenProject::BuildCache)?;
        let graph_cache_dir = crate::water_dir::project_graph_cache_dir_on(&host, &path)
            .map_err(FailToOpenProject::BuildCache)?;
        info!(
            path = %path.display(),
            open_mode = ?open_mode,
            elapsed_ms = build_cache_start.elapsed().as_millis(),
            "Project::open ensured project build cache"
        );

        let mut project = Self {
            host: host.clone(),
            root: path,
            manifest,
            crate_name,
            cargo_layout,
            linked_packages: Arc::new(async_lock::Mutex::new(BTreeMap::new())),
            enabled_features: Arc::new(async_lock::Mutex::new(BTreeMap::new())),
            generated_features: Arc::new(async_lock::Mutex::new(BTreeMap::new())),
            cargo_resolve_permits: Arc::new(async_lock::Semaphore::new(
                Self::CARGO_RESOLVE_PERMITS,
            )),
            managed_backends_root,
            graph_cache_dir,
            backends: Backends::default(),
            local_sources,
        };

        // Initialize the managed backends the caller selected.
        // Always re-scaffold templates on each run to pick up manifest changes (e.g., permissions)
        // Build cache (build/, .gradle/, DerivedData/) is preserved since scaffold only writes template files
        //
        // Skip backend initialization when:
        // 1. Running inside Xcode's sandboxed build script phase (WATERUI_SKIP_RUST_BUILD=1)
        // 2. Running inside any sandbox (sandbox-exec sets __XCODE_BUILT_PRODUCTS_DIR_PATHS or similar)
        // 3. Xcode is the current build tool (ACTION env var is set by Xcode)
        let skip_backend_init = project
            .host()
            .env("WATERUI_SKIP_RUST_BUILD")
            .is_some_and(|value| value == "1")
            || project.host().env("ACTION").is_some() // Xcode sets this during builds
            || project.host().env("XCODE_PRODUCT_BUILD_VERSION").is_some();

        if !skip_backend_init && open_mode == OpenMode::Full {
            // Rendering target-derived scaffolds is not part of opening a
            // project: the ffi companion is rendered by the path that builds
            // it (`scaffold_ffi_companion`), so `water clean`, `water fetch`
            // and backend inits never resolve a dependency graph here.

            if backends.apple() {
                let apple_backend_start = std::time::Instant::now();
                let apple_backend = AppleBackend::init(&project)
                    .await
                    .map_err(FailToOpenProject::BackendInit)?;
                info!(
                    path = %project.root.display(),
                    elapsed_ms = apple_backend_start.elapsed().as_millis(),
                    "Project::open initialized Apple backend"
                );
                project.backends.set_apple(apple_backend);
            }

            if backends.android() {
                let android_backend_start = std::time::Instant::now();
                let android_backend = AndroidBackend::init(&project)
                    .await
                    .map_err(FailToOpenProject::BackendInit)?;
                info!(
                    path = %project.root.display(),
                    elapsed_ms = android_backend_start.elapsed().as_millis(),
                    "Project::open initialized Android backend"
                );
                project.backends.set_android(android_backend);
            }
        }

        info!(
            path = %project.root.display(),
            open_mode = ?open_mode,
            elapsed_ms = total_start.elapsed().as_millis(),
            "Project::open completed"
        );

        Ok(project)
    }
}

impl Project {
    async fn save_manifest(&self) -> eyre::Result<()> {
        self.manifest.save(&self.root).await.map_err(Into::into)
    }
}

async fn apply_channel_selection(
    host: &Host,
    root: &Path,
    framework: ResolvedFramework,
    updates: Vec<(PathBuf, Option<Vec<u8>>)>,
) -> eyre::Result<()> {
    let mut previous = BTreeMap::new();
    for file in updates
        .iter()
        .map(|(file, _)| file.clone())
        .chain([root.join("Cargo.lock")])
    {
        let contents = match smol::fs::read(&file).await {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        previous.insert(file, contents);
    }
    let result = async {
        for (file, contents) in &updates {
            write_channel_file(file, contents.as_deref()).await?;
        }
        resolve_cargo_layout(host, root, Some(framework), CargoResolution::Update).await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        for (file, contents) in previous {
            write_channel_file(&file, contents.as_deref())
                .await
                .map_err(|restore| {
                    eyre::eyre!("{error}; could not restore {}: {restore}", file.display())
                })?;
        }
        return Err(error);
    }
    Ok(())
}

async fn write_channel_file(path: &Path, contents: Option<&[u8]>) -> std::io::Result<()> {
    match contents {
        Some(contents) => smol::fs::write(path, contents).await,
        None => match smol::fs::remove_file(path).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        },
    }
}

async fn resolve_cargo_layout(
    host: &Host,
    current_dir: &Path,
    framework: Option<ResolvedFramework>,
    mode: CargoResolution,
) -> eyre::Result<CargoLayout> {
    let root = current_dir.to_path_buf();
    let metadata_host = host.clone();
    let metadata = unblock(move || {
        let mut command = cargo_metadata::MetadataCommand::new();
        command.current_dir(root);
        match mode {
            CargoResolution::Local => {
                command.no_deps();
            }
            CargoResolution::Locked => {
                command.other_options(vec!["--locked".to_string()]);
            }
            CargoResolution::Update => {}
        }
        metadata_on(&metadata_host, &command)
    })
    .await?;
    validate_resolved_cli(host, &metadata)?;
    if let Some(framework) = framework
        && framework.channel() != Some(FrameworkChannel::Stable)
    {
        let lockfile = smol::fs::read(current_dir.join("Water.lock")).await?;
        framework.validate_dependencies(&metadata, &lockfile)?;
    }

    // `dunce`, not `std::fs::canonicalize`: on Windows the standard one
    // returns an extended-length path (`\\?\D:\...`), while `cargo
    // metadata` reports the plain one — comparing the two would never
    // match the application package (part of #152).
    let application_manifest = dunce::canonicalize(current_dir.join("Cargo.toml"))?;
    let root_package_id = package_at_manifest(&metadata, &application_manifest)?
        .id
        .to_string();

    Ok(CargoLayout {
        target_dir: metadata.target_directory.into_std_path_buf(),
        workspace_root: metadata.workspace_root.into_std_path_buf(),
        root_package_id,
    })
}

/// The package whose manifest is `application_manifest` — the exact
/// manifest path, canonicalized the same `dunce` way cargo reports it.
fn package_at_manifest<'a>(
    metadata: &'a cargo_metadata::Metadata,
    application_manifest: &Path,
) -> eyre::Result<&'a cargo_metadata::Package> {
    metadata
        .packages
        .iter()
        .find(|package| package.manifest_path.as_std_path() == application_manifest)
        .ok_or_else(|| {
            eyre::eyre!(
                "Cargo metadata omitted the application package at {}",
                application_manifest.display()
            )
        })
}

/// `cargo metadata` as `command` configures it, run on `host` — its `PATH`,
/// environment and working directory, the command's own `current_dir`
/// taking precedence — with [`cargo_metadata::MetadataCommand::exec`]'s
/// error semantics.
pub(crate) fn metadata_on(
    host: &Host,
    command: &cargo_metadata::MetadataCommand,
) -> Result<cargo_metadata::Metadata, cargo_metadata::Error> {
    let spec = command.cargo_command();
    let mut cargo = host.std_command("cargo");
    cargo.args(spec.get_args());
    if let Some(dir) = spec.get_current_dir() {
        cargo.current_dir(dir);
    }
    let output = cargo.output()?;
    if !output.status.success() {
        return Err(cargo_metadata::Error::CargoMetadata {
            stderr: String::from_utf8(output.stderr)?,
        });
    }
    let stdout = std::str::from_utf8(&output.stdout)?
        .lines()
        .find(|line| line.starts_with('{'))
        .ok_or(cargo_metadata::Error::NoJson)?;
    cargo_metadata::MetadataCommand::parse(stdout)
}

/// Run `cargo tree` for the application package rooted at `project_root`'s
/// manifest, over the given edge kinds, and return the `{p}`-formatted tree.
///
/// Every triple in `targets` rides the same resolve as its own `--target`
/// flag — one `cargo tree` invocation answers a whole serving set, and the
/// printed union is exactly the per-target union because a package line
/// names its source, never the target that selected it. `package_spec` is
/// the application package's resolved id — [`CargoLayout`] carries it at
/// open for every evaluation this project runs.
///
/// `locked` passes `--locked` to the resolve: trees that are read-only input —
/// the shared pinned-framework checkout — must fail loudly on a stale
/// committed lockfile instead of letting cargo rewrite it in place.
async fn cargo_tree(
    host: &Host,
    project_root: &Path,
    package_spec: &str,
    edges: &str,
    targets: &[Triple],
    locked: bool,
) -> eyre::Result<String> {
    let application_manifest = dunce::canonicalize(project_root.join("Cargo.toml"))?;
    let mut tree = host.command("cargo");
    tree.arg("tree")
        .arg("--manifest-path")
        .arg(&application_manifest)
        .arg("--package")
        .arg(package_spec)
        .arg("--edges")
        .arg(edges)
        .arg("--prefix")
        .arg("none")
        .arg("--format")
        .arg("{p}")
        .current_dir(project_root);
    for target in targets {
        tree.arg("--target").arg(target.to_string());
    }
    if locked {
        tree.arg("--locked");
    }
    let output = tree.output().await?;
    if !output.status.success() {
        return Err(eyre::eyre!(
            "failed to resolve runtime dependency graph for {}: {}",
            application_manifest.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|error| eyre::eyre!("Cargo runtime dependency graph is not UTF-8: {error}"))
}

/// The framework's local source roots — directories whose packages are the
/// framework's, never the user's own code:
///
/// - the resolved framework's local checkout — `Source::Local`, the
///   `waterui_path` project;
/// - `waterui_path` itself: `refresh_local_patches` copies its `[patch]`
///   tables into the project's manifest and the generated manifests resolve
///   backend crates through it whether or not a channel is recorded
///   alongside, so a recorded framework plus a checkout still builds the
///   framework from there;
/// - the `path` entries of the checkout's own `[patch]` tables, recorded
///   project-root-relative under `waterui_patches` — an entry escaping the
///   checkout (`path = "../sibling"`) still resolves inside the framework's
///   source.
///
/// Channel and git sources name no local root and contribute nothing. Every
/// root is canonicalized: `cargo tree` prints a path package's directory as
/// the dependency declared it, symlinks included, so nothing here may be
/// compared against it before both sides are canonical.
///
/// # Errors
///
/// Returns an error naming the framework source whose canonicalization
/// fails.
fn framework_local_roots(
    project_root: &Path,
    manifest: &Manifest,
    framework: &ResolvedFramework,
) -> eyre::Result<BTreeSet<PathBuf>> {
    let mut candidates = Vec::new();
    if let Some(root) = framework.local_checkout_root() {
        candidates.push(root.to_path_buf());
    }
    // `waterui_patches` is the record of the checkout's `[patch]` tables that
    // `refresh_local_patches` maintains while `waterui_path` is set; a
    // channel switch drops the path but not the record, so the record names
    // framework sources only alongside the path it was written for.
    if let Some(local) = &manifest.waterui_path {
        candidates.push(project_root.join(local));
        for dependency in manifest
            .waterui_patches
            .values()
            .flat_map(std::collections::BTreeMap::values)
        {
            if let cargo_toml::Dependency::Detailed(detail) = dependency
                && let Some(path) = &detail.path
            {
                candidates.push(project_root.join(path));
            }
        }
    }
    let mut roots = BTreeSet::new();
    for root in candidates {
        let root = dunce::canonicalize(&root).wrap_err_with(|| {
            format!(
                "the framework source {} cannot be canonicalized",
                root.display()
            )
        })?;
        roots.insert(root);
    }
    Ok(roots)
}

/// The `{p}` lines a `cargo tree` evaluation printed for each package name —
/// every occurrence, in print order. `[profile.dev.package.<name>]` keys on
/// the name alone, so a name's entries stay together until classification.
type LinkedPackages = BTreeMap<String, Vec<String>>;

/// The application package's resolved id at `project_root` — the spec
/// [`cargo_tree`] passes to `--package`. [`CargoLayout`] carries the same
/// value for every [`Project`] evaluation; this is the standalone path for
/// callers that have no open project.
#[cfg(test)]
async fn application_package_spec(
    host: &Host,
    project_root: &Path,
    locked: bool,
) -> eyre::Result<String> {
    let application_manifest = dunce::canonicalize(project_root.join("Cargo.toml"))?;
    let metadata_manifest = application_manifest.clone();
    let host_for_metadata = host.clone();
    let metadata = unblock(move || {
        let mut command = cargo_metadata::MetadataCommand::new();
        command.no_deps().manifest_path(metadata_manifest);
        if locked {
            command.other_options(vec!["--locked".to_string()]);
        }
        metadata_on(&host_for_metadata, &command)
    })
    .await?;
    Ok(package_at_manifest(&metadata, &application_manifest)?
        .id
        .to_string())
}

#[cfg(test)]
async fn resolve_linked_runtime_packages(
    host: &Host,
    project_root: PathBuf,
    target: &Triple,
    locked: bool,
) -> eyre::Result<LinkedPackages> {
    let package_spec = application_package_spec(host, &project_root, locked).await?;
    let tree = cargo_tree(
        host,
        &project_root,
        &package_spec,
        "normal",
        std::slice::from_ref(target),
        locked,
    )
    .await?;
    Ok(linked_packages_from_tree(&tree))
}

/// The `{p}` lines of a normal-edge `cargo tree` evaluation keyed by
/// package name — every printed occurrence, in print order. A multi-target
/// invocation prints one tree per `--target`, blank lines separating the
/// blocks; the union across blocks is the union across targets.
fn linked_packages_from_tree(tree: &str) -> LinkedPackages {
    let mut linked: LinkedPackages = BTreeMap::new();
    for package in tree.lines() {
        let Some(name) = package.split_ascii_whitespace().next() else {
            continue;
        };
        // A package repeated in the graph prints again — a path `foo` beside
        // a registry `foo` included — and every occurrence is kept:
        // `[profile.dev.package.<name>]` keys on the name alone, so whichever
        // line kept the path must not be lost to print order.
        linked
            .entry(name.to_string())
            .or_default()
            .push(package.to_string());
    }

    linked
}

/// Feature names turned on inside the application's subtree. With `--edges
/// features`, `cargo tree` reports each enabled feature as a
/// `<package> feature "<name>"` node; only the names are kept, since the
/// question asked of this set is always "is a feature named X enabled".
#[cfg(test)]
async fn resolve_enabled_features(
    host: &Host,
    project_root: PathBuf,
    target: &Triple,
    locked: bool,
) -> eyre::Result<BTreeSet<String>> {
    let package_spec = application_package_spec(host, &project_root, locked).await?;
    let tree = cargo_tree(
        host,
        &project_root,
        &package_spec,
        "features",
        std::slice::from_ref(target),
        locked,
    )
    .await?;
    Ok(enabled_features_from_tree(&tree))
}

/// The feature names a `cargo tree --edges features` evaluation reported.
fn enabled_features_from_tree(tree: &str) -> BTreeSet<String> {
    let mut features = BTreeSet::new();
    for node in tree.lines() {
        if let Some(feature) = node
            .split_once(" feature \"")
            .and_then(|(_, rest)| rest.strip_suffix('"'))
        {
            features.insert(feature.to_string());
        }
    }
    features
}

/// The package directory a `cargo tree --format {p}` line's annotation
/// carries, when it names a path package. The `{p}` grammar on Cargo 1.99 is
/// `name vX [annotations] [*]`: a path package annotates its directory as a
/// bare absolute path — `mymacro v0.1.0 (proc-macro) (/checkouts/mymacro)` —
/// a git source annotates `git+…`, a registry package annotates nothing, a
/// repeated package ends in ` (*)`, and a directory is printed unescaped,
/// parentheses included.
fn tree_package_directory(entry: &str) -> Option<PathBuf> {
    let mut line = entry.strip_suffix(" (*)").unwrap_or(entry);
    // `(proc-macro)` sits ahead of the path annotation on Cargo 1.99, but a
    // version that prints it last is handled the same way: strip it as a
    // suffix either way and the remaining last group is the source.
    if let Some(stripped) = line.strip_suffix(" (proc-macro)") {
        line = stripped;
    }
    let line = line.strip_suffix(')')?;
    // The source annotation is the line's last parenthesised group — it
    // closes the line — but a directory printed raw may itself contain
    // parentheses, so the group's opening ` (` boundary cannot be located
    // positionally. Of the boundaries present, the first whose contents form
    // an absolute path is the annotation's: a marker like `(proc-macro)`
    // leaves a remainder that is never absolute.
    for (boundary, _) in line.match_indices(" (") {
        let directory = &line[boundary + 2..];
        if Path::new(directory).is_absolute() {
            return Some(PathBuf::from(directory));
        }
    }
    None
}

/// The project's own packages read out of the resolved `{p}` graph:
/// `crate_name` — the app crate, always, even when its own manifest lies
/// inside the framework checkout the way the in-tree examples do — plus
/// every other package whose name has at least one occurrence that is a path
/// package outside `framework_roots`. Channel and git packages carry no
/// path, so the directory condition excludes them on its own; a name is
/// checked against every occurrence because `[profile.dev.package.<name>]`
/// keys on the name alone — a path `foo` beside a registry `foo` is the
/// project's own code whatever the print order.
///
/// `cargo tree` prints a path package's directory as the dependency declared
/// it — symlinks included — so it is canonicalized before the
/// `starts_with` comparison, the same canonicalization the framework roots
/// already went through. A directory that does not resolve is an error
/// naming it.
fn project_packages_from_tree(
    crate_name: &str,
    tree: &LinkedPackages,
    framework_roots: &BTreeSet<PathBuf>,
) -> eyre::Result<BTreeSet<String>> {
    let mut packages = BTreeSet::from([crate_name.to_string()]);
    for (name, entries) in tree {
        if name.as_str() == crate_name {
            continue;
        }
        for entry in entries {
            let Some(directory) = tree_package_directory(entry) else {
                continue;
            };
            let directory = dunce::canonicalize(&directory).wrap_err_with(|| {
                format!(
                    "the path package directory {} cannot be canonicalized",
                    directory.display()
                )
            })?;
            if framework_roots
                .iter()
                .any(|root| directory.starts_with(root))
            {
                continue;
            }
            // The name is a project package: the override applies to every
            // occurrence anyway, so the rest need no evaluation.
            packages.insert(name.clone());
            break;
        }
    }
    Ok(packages)
}

#[cfg(test)]
mod project_package_tests {
    use super::*;

    #[test]
    fn tree_package_directory_reads_only_absolute_annotations() {
        // `cargo tree --edges normal --format {p}` lines observed on Cargo
        // 1.99.0 (b940084d7): a path proc-macro prints `(proc-macro)` ahead
        // of its directory, a repeated package ends in ` (*)`, a directory
        // containing parentheses prints raw, and a registry package
        // annotates nothing. The format prints the platform's own absolute
        // paths, so the fixtures anchor theirs under a temp dir — a
        // `/workspace` or `/private/tmp` literal is absolute only on Unix.
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let probe = temp.path().join("p2072-tree-probe");
        let weird = probe.join("weird (dir)");
        let cases = [
            (
                format!("app v0.1.0 ({})", workspace.join("app").display()),
                Some(workspace.join("app")),
            ),
            (
                format!("dep v0.1.0 ({}) (*)", workspace.join("dep").display()),
                Some(workspace.join("dep")),
            ),
            (
                format!(
                    "mymacro v0.1.0 (proc-macro) ({})",
                    probe.join("mymacro").display()
                ),
                Some(probe.join("mymacro")),
            ),
            (
                format!("weirdname v0.1.0 ({})", weird.display()),
                Some(weird),
            ),
            (
                format!(
                    "bitflags v2.9.4 ({})",
                    probe.join("bitflags-fork").display()
                ),
                Some(probe.join("bitflags-fork")),
            ),
            ("serde v1.0.229".to_string(), None),
            ("bitflags v2.13.2".to_string(), None),
            ("serde_derive v1.0.229 (proc-macro)".to_string(), None),
            ("proc-macro2 v1.0.107 (*)".to_string(), None),
            (
                "wgpu v26.0.0 (git+https://github.com/gfx-rs/wgpu?rev=abc#abc)".to_string(),
                None,
            ),
        ];
        for (entry, expected) in cases {
            assert_eq!(tree_package_directory(&entry), expected, "entry: {entry}");
        }
    }

    /// A `water create` app gives the app crate: registry and git packages
    /// carry no path, so nothing else qualifies.
    #[test]
    fn project_packages_keep_only_path_packages() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app = temp.path().join("workspace").join("app");
        std::fs::create_dir_all(&app).expect("app dir");
        let dep = temp.path().join("my_dep");
        std::fs::create_dir_all(&dep).expect("dep dir");
        let tree = BTreeMap::from([
            (
                "app".to_string(),
                vec![format!("app v0.1.0 ({})", app.display())],
            ),
            (
                "my_dep".to_string(),
                vec![format!("my_dep v0.1.0 ({})", dep.display())],
            ),
            ("serde".to_string(), vec!["serde v1.0.228".to_string()]),
            (
                "wgpu".to_string(),
                vec!["wgpu v26.0.0 (git+https://github.com/gfx-rs/wgpu?rev=abc#abc)".to_string()],
            ),
        ]);
        assert_eq!(
            project_packages_from_tree("app", &tree, &BTreeSet::new())
                .expect("path packages resolve"),
            BTreeSet::from(["app".to_string(), "my_dep".to_string()])
        );
    }

    /// A package whose manifest lies inside the `waterui_path` checkout root
    /// belongs to the framework, whatever the package is — except the app
    /// crate itself, which an in-tree example puts there on purpose.
    #[test]
    fn project_packages_exclude_the_framework_checkout() {
        let temp = tempfile::tempdir().expect("tempdir");
        let checkout = temp.path().join("checkout");
        std::fs::create_dir_all(checkout.join("waterui/core")).expect("framework dirs");
        let app = temp.path().join("workspace").join("app");
        std::fs::create_dir_all(&app).expect("app dir");
        let my_dep = temp.path().join("my_dep");
        std::fs::create_dir_all(&my_dep).expect("dep dir");
        let framework_roots =
            BTreeSet::from([dunce::canonicalize(&checkout).expect("checkout canonicalizes")]);
        let tree = BTreeMap::from([
            (
                "app".to_string(),
                vec![format!("app v0.1.0 ({})", app.display())],
            ),
            (
                "my_dep".to_string(),
                vec![format!("my_dep v0.1.0 ({})", my_dep.display())],
            ),
            (
                "waterui".to_string(),
                vec![format!(
                    "waterui v0.6.0 ({})",
                    checkout.join("waterui").display()
                )],
            ),
            (
                "waterui-core".to_string(),
                vec![format!(
                    "waterui-core v0.6.0 ({})",
                    checkout.join("waterui/core").display()
                )],
            ),
        ]);
        assert_eq!(
            project_packages_from_tree("app", &tree, &framework_roots)
                .expect("path packages resolve"),
            BTreeSet::from(["app".to_string(), "my_dep".to_string()])
        );

        let gallery = checkout.join("waterui/examples/gallery");
        std::fs::create_dir_all(&gallery).expect("example dir");
        let example_tree = BTreeMap::from([
            (
                "waterui-example-gallery".to_string(),
                vec![format!(
                    "waterui-example-gallery v0.1.0 ({})",
                    gallery.display()
                )],
            ),
            (
                "waterui".to_string(),
                vec![format!(
                    "waterui v0.6.0 ({})",
                    checkout.join("waterui").display()
                )],
            ),
        ]);
        assert_eq!(
            project_packages_from_tree("waterui-example-gallery", &example_tree, &framework_roots)
                .expect("path packages resolve"),
            BTreeSet::from(["waterui-example-gallery".to_string()])
        );
    }

    /// `cargo tree` prints a path package's directory as declared — symlinks
    /// included — while the framework roots were canonicalized. A checkout
    /// reached through a symlinked parent must still classify as framework.
    #[cfg(unix)]
    #[test]
    fn project_packages_canonicalize_the_tree_directories() {
        let temp = tempfile::tempdir().expect("tempdir");
        let checkout = temp.path().join("real/checkout");
        std::fs::create_dir_all(checkout.join("waterui")).expect("framework dirs");
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(temp.path().join("real"), &alias).expect("symlink");
        let aliased_checkout = alias.join("checkout");
        let framework_roots = BTreeSet::from([
            dunce::canonicalize(&aliased_checkout).expect("checkout canonicalizes")
        ]);
        let app = temp.path().join("workspace").join("app");
        std::fs::create_dir_all(&app).expect("app dir");
        let tree = BTreeMap::from([
            (
                "app".to_string(),
                vec![format!("app v0.1.0 ({})", app.display())],
            ),
            (
                "waterui".to_string(),
                vec![format!(
                    "waterui v0.6.0 ({})",
                    aliased_checkout.join("waterui").display()
                )],
            ),
        ]);
        assert_eq!(
            project_packages_from_tree("app", &tree, &framework_roots)
                .expect("path packages resolve"),
            BTreeSet::from(["app".to_string()])
        );
    }

    /// A path package whose directory cannot be canonicalized is an error
    /// naming that directory.
    #[test]
    fn project_packages_fail_on_an_unresolvable_directory() {
        // The directory must be absolute on every platform and guaranteed
        // missing: a not-yet-created path under a fresh temp dir is both.
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("missing").join("path").join("package");
        let tree = BTreeMap::from([(
            "my_dep".to_string(),
            vec![format!("my_dep v0.1.0 ({})", missing.display())],
        )]);
        let error = project_packages_from_tree("app", &tree, &BTreeSet::new())
            .expect_err("an unresolvable directory is an error");
        assert!(
            error.to_string().contains(&missing.display().to_string()),
            "the error names the directory: {error}"
        );
    }

    /// `[profile.dev.package.<name>]` keys on the package name alone, so a
    /// name whose occurrences mix a path package with a registry one is the
    /// project's own code whichever line `cargo tree` printed first.
    #[test]
    fn project_packages_classify_a_name_by_every_occurrence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fork = temp.path().join("bitflags-fork");
        std::fs::create_dir_all(&fork).expect("fork dir");
        let path_line = format!("bitflags v2.9.4 ({})", fork.display());
        for entries in [
            vec!["bitflags v2.13.2".to_string(), path_line.clone()],
            vec![path_line, "bitflags v2.13.2".to_string()],
        ] {
            let tree = BTreeMap::from([("bitflags".to_string(), entries)]);
            assert_eq!(
                project_packages_from_tree("app", &tree, &BTreeSet::new())
                    .expect("path packages resolve"),
                BTreeSet::from(["app".to_string(), "bitflags".to_string()])
            );
        }
    }

    /// A checkout `[patch]` path escaping the checkout (`path =
    /// "../sibling"`) is recorded project-root-relative under
    /// `waterui_patches`, and the directory it resolves to is the
    /// framework's — a package inside it is excluded from the project's own
    /// packages.
    #[test]
    fn framework_roots_include_an_escaping_patch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path();
        let checkout = project_root.join("vendor/waterui");
        // `../sibling` declared by the checkout names the directory beside
        // it, recorded project-root-relative as
        // `vendor/waterui/../sibling`.
        let sibling = project_root.join("vendor/sibling");
        std::fs::create_dir_all(&checkout).expect("checkout dir");
        std::fs::create_dir_all(sibling.join("crates/sibling_crate")).expect("sibling dirs");
        let mut manifest = Manifest::new(Package {
            name: "Test App".to_string(),
            bundle_identifier: crate::project_types::BundleIdentifier::try_from("dev.test.app")
                .expect("bundle identifier"),
            assets_path: "assets".to_string(),
            accessory: false,
            embedded: false,
        });
        manifest.waterui_path = Some("vendor/waterui".to_string());
        manifest.waterui_patches.insert(
            "crates-io".to_string(),
            BTreeMap::from([(
                "sibling_crate".to_string(),
                cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
                    path: Some("vendor/waterui/../sibling".to_string()),
                    ..cargo_toml::DependencyDetail::default()
                })),
            )]),
        );
        let framework = crate::framework::test_fixtures::stable_framework();
        let roots = framework_local_roots(project_root, &manifest, &framework)
            .expect("framework roots resolve");
        assert_eq!(
            roots,
            BTreeSet::from([
                dunce::canonicalize(&checkout).expect("checkout canonicalizes"),
                dunce::canonicalize(&sibling).expect("sibling canonicalizes"),
            ])
        );
        let tree = BTreeMap::from([(
            "sibling_crate".to_string(),
            vec![format!(
                "sibling_crate v0.1.0 ({})",
                sibling.join("crates/sibling_crate").display()
            )],
        )]);
        assert_eq!(
            project_packages_from_tree("app", &tree, &roots).expect("path packages resolve"),
            BTreeSet::from(["app".to_string()])
        );
    }
}

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use smol::{fs::read_to_string, unblock};
use waterui_assets_planner::{LaunchConfig, ThemeConfig};

use crate::{
    android::{backend::AndroidBackend, signing::AndroidSigningConfig},
    apple::backend::AppleBackend,
    backend::{Backend, Backends},
    build::{BuildOptions, BuildProfile},
    device::{Artifact, Device, FailToRun, RunOptions, Running},
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project_types::{BundleIdentifier, CrateName, PermissionKey, generated_crate_name},
    templates::{self, TemplateContext},
    utils::command,
    web,
};

/// The `Water.toml` key of [`Manifest::waterui_patches`].
const WATERUI_PATCHES_KEY: &str = "waterui_patches";

/// [`Manifest::waterui_patches`] alone, serialized to splice into an existing
/// `Water.toml`.
#[derive(Serialize)]
struct WateruiPatchesRecord<'a> {
    #[serde(
        rename = "waterui_patches",
        skip_serializing_if = "cargo_toml::PatchSet::is_empty"
    )]
    patches: &'a cargo_toml::PatchSet,
}

/// Configuration for a `WaterUI` project persisted to `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Manifest {
    /// Package information.
    pub package: Package,
    /// ESP32 device configuration (`[esp32]`): chip, panel geometry, and
    /// the fonts firmware embeds — product configuration, not source state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub esp32: Option<crate::esp32::backend::Esp32Config>,
    /// Hydrolysis backend selections (`[hydrolysis]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hydrolysis: Option<crate::backend::HydrolysisConfig>,
    /// Path to local `WaterUI` repository for dev mode.
    /// When set, all backends will use this path instead of the published versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waterui_path: Option<String>,
    /// The `[patch]` entries the CLI last copied from the `waterui_path`
    /// checkout into the project's `Cargo.toml`. Every other entry there is
    /// the project's own — an override of a checkout entry included — which
    /// the next copy keeps.
    #[serde(default, skip_serializing_if = "cargo_toml::PatchSet::is_empty")]
    pub waterui_patches: cargo_toml::PatchSet,
    /// Exact framework and backend selection, resolved only by explicit version operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<ResolvedFramework>,
    /// Permission configuration.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<PermissionKey, PermissionEntry>,
    /// App-only configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppConfig>,
    /// Cross-platform app theme slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeConfig>,
    /// The launch screen shown until the app's first frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchConfig>,
    /// Web-frontend toolchain declarations (`[web]`); only the CLI reads this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<web::WebConfig>,
    /// Assets the project bundles beyond what dependency crates declare for
    /// themselves (`[assets]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<AssetsConfig>,
    /// Distribution signing configuration (`[signing]`).
    #[serde(default, skip_serializing_if = "SigningConfig::is_empty")]
    pub signing: SigningConfig,
    /// Values dependency crates request from the app (`[app_values]`):
    /// the Firebase configuration, Apple Pay merchant IDs, the Cast receiver.
    #[serde(
        default,
        skip_serializing_if = "crate::assets::AppValuesConfig::is_empty"
    )]
    pub app_values: crate::assets::AppValuesConfig,
}

/// Distribution signing configuration (`[signing]`).
///
/// Each platform's distribution packaging reads its own subsection; a
/// project that never packages for distribution leaves the table out.
/// Development builds keep each platform's own signing (the Android debug
/// keystore); these entries apply to release packaging only, and carry no
/// secrets — passwords are read from the environment at package time.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SigningConfig {
    /// macOS Developer ID distribution signing (`[signing.macos]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<MacosSigningConfig>,
    /// Android release signing (`[signing.android]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android: Option<AndroidSigningConfig>,
}

impl SigningConfig {
    /// Whether no platform carries signing configuration.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.macos.is_none() && self.android.is_none()
    }
}

/// macOS Developer ID distribution signing (`[signing.macos]`).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MacosSigningConfig {
    /// The Apple Developer team the package signs for — matched against the
    /// Developer ID Application certificate's subject organizational unit.
    pub team_id: String,
    /// The `notarytool` keychain profile the submission authenticates with,
    /// created with `xcrun notarytool store-credentials`.
    pub notary_profile: String,
}

/// Permission entry in `[permissions]`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PermissionEntry {
    enable: bool,
    /// Explain why this permission is needed.
    description: String,
}

impl PermissionEntry {
    /// Create an enabled permission entry with the given rationale.
    #[must_use]
    pub fn enabled(description: impl Into<String>) -> Self {
        Self {
            enable: true,
            description: description.into(),
        }
    }

    /// Check if this permission is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enable
    }

    /// Get the description of why this permission is needed.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Errors that can occur when opening a `Water.toml` manifest file.
#[derive(Debug, thiserror::Error)]
pub enum FailToOpenManifest {
    /// Failed to read the manifest file from the filesystem.
    #[error("Failed to read manifest file: {0}")]
    ReadError(std::io::Error),
    /// The manifest file is invalid or malformed.
    #[error("Invalid manifest file: {0}")]
    InvalidManifest(toml::de::Error),

    /// The manifest file was not found at the specified path.
    #[error("Manifest file not found at the specified path")]
    NotFound,

    /// The project carries configuration from the removed app mode.
    #[error("{0}")]
    AppMode(crate::project_model::app_mode::AppModeLeftovers),
}

/// Errors that can occur when saving a `Water.toml` manifest file.
#[derive(Debug, thiserror::Error)]
pub enum FailToSaveManifest {
    /// Failed to serialize the manifest to TOML.
    #[error("Failed to serialize manifest: {0}")]
    Serialize(toml::ser::Error),
    /// Failed to write the manifest file to disk.
    #[error("Failed to write manifest file: {0}")]
    Write(std::io::Error),
}
impl Manifest {
    /// Open and parse a `Water.toml` manifest file from the specified path.
    ///
    /// # Errors
    /// - `FailToOpenManifest::ReadError`: If there was an error reading the file.
    /// - `FailToOpenManifest::InvalidManifest`: If the file contents are not valid TOML.
    /// - `FailToOpenManifest::NotFound`: If the file does not exist at the specified path.
    /// - `FailToOpenManifest::AppMode`: If the project carries app-mode keys.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, FailToOpenManifest> {
        match read_to_string(path.as_ref()).await {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(FailToOpenManifest::NotFound),
            Err(e) => Err(FailToOpenManifest::ReadError(e)),
        }
    }

    /// Parse `Water.toml` text, refusing a project that still carries
    /// app-mode leftovers.
    ///
    /// # Errors
    /// - `FailToOpenManifest::InvalidManifest`: If the text is not a valid manifest.
    /// - `FailToOpenManifest::AppMode`: If the project carries app-mode keys.
    pub fn parse(text: &str) -> Result<Self, FailToOpenManifest> {
        let table: toml::Table = text.parse().map_err(FailToOpenManifest::InvalidManifest)?;
        if let Some(leftovers) = crate::project_model::app_mode::AppModeLeftovers::find(&table) {
            return Err(FailToOpenManifest::AppMode(leftovers));
        }
        toml::from_str(text).map_err(FailToOpenManifest::InvalidManifest)
    }

    /// Save the manifest to a `Water.toml` file at the specified directory.
    ///
    /// # Errors
    /// - If there was an error serializing the manifest to TOML.
    /// - If there was an error writing the file.
    pub async fn save(&self, dir: impl AsRef<Path>) -> Result<(), FailToSaveManifest> {
        let path = dir.as_ref().join("Water.toml");
        let content = toml::to_string_pretty(self).map_err(FailToSaveManifest::Serialize)?;
        smol::fs::write(&path, content)
            .await
            .map_err(FailToSaveManifest::Write)
    }

    /// Create a new `Manifest` with the specified package information.
    #[must_use]
    pub fn new(package: Package) -> Self {
        Self {
            package,
            esp32: None,
            hydrolysis: None,
            waterui_path: None,
            waterui_patches: cargo_toml::PatchSet::new(),
            framework: None,
            permissions: BTreeMap::default(),
            app: None,
            theme: None,
            launch: None,
            web: None,
            assets: None,
            signing: SigningConfig::default(),
            app_values: crate::assets::AppValuesConfig::default(),
        }
    }
}

/// The engine that draws this application's standard `WebView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedWebViewBackend {
    /// Platform-provided `WebView`.
    System,
    /// Bundled WPE `WebKit` runtime.
    Wpe,
    /// Bundled Chromium Embedded Framework runtime.
    Cef,
}

impl std::fmt::Display for ResolvedWebViewBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Browser engines that must be staged for one resolved application graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserRuntimePlan {
    /// Standard `WebView` engine, when `waterui-webview` is linked.
    pub webview: Option<ResolvedWebViewBackend>,
    /// Whether the independent full Chromium component is linked.
    pub chromium: bool,
}

impl BrowserRuntimePlan {
    /// Returns whether this application requires a packaged CEF runtime and
    /// subprocess helper.
    #[must_use]
    pub const fn requires_cef(self) -> bool {
        self.chromium || matches!(self.webview, Some(ResolvedWebViewBackend::Cef))
    }
}

impl ResolvedWebViewBackend {
    /// Return whether this engine can be hosted by a platform and backend pair.
    #[must_use]
    pub const fn supports(self, platform: TargetPlatform, backend: TargetBackend) -> bool {
        match self {
            Self::System => matches!(
                (platform, backend),
                (
                    TargetPlatform::MacOS,
                    TargetBackend::Apple | TargetBackend::Hydrolysis
                ) | (
                    TargetPlatform::IOS
                        | TargetPlatform::IOSSimulator
                        | TargetPlatform::VisionOS
                        | TargetPlatform::VisionOSSimulator,
                    TargetBackend::Apple
                ) | (TargetPlatform::Android, TargetBackend::Android)
                    | (TargetPlatform::Linux, TargetBackend::Gtk4)
                    | (TargetPlatform::Web, TargetBackend::Hydrolysis)
            ),
            Self::Wpe => {
                matches!(platform, TargetPlatform::Linux)
                    && matches!(backend, TargetBackend::Gtk4 | TargetBackend::Hydrolysis)
            }
            Self::Cef => cef_is_supported(platform, backend),
        }
    }

    /// Returns this engine, or an error naming what cannot host it.
    ///
    /// # Errors
    ///
    /// Returns an error when this platform and backend pair cannot host the
    /// engine the application selected.
    pub const fn validate(
        self,
        platform: TargetPlatform,
        backend: TargetBackend,
    ) -> Result<Self, UnsupportedWebViewBackend> {
        if self.supports(platform, backend) {
            Ok(self)
        } else {
            Err(UnsupportedWebViewBackend {
                resolved: self,
                platform,
                backend,
            })
        }
    }

    /// Stable lowercase name used for Cargo features, runtime manifests, and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Wpe => "wpe",
            Self::Cef => "cef",
        }
    }
}

const fn cef_is_supported(platform: TargetPlatform, backend: TargetBackend) -> bool {
    !matches!(backend, TargetBackend::Dew)
        && matches!(
            platform,
            TargetPlatform::MacOS | TargetPlatform::Linux | TargetPlatform::Windows
        )
}

/// Error returned for an unsupported `WebView` engine/platform/backend combination.
#[derive(Debug, thiserror::Error)]
#[error(
    "this application's WebView engine resolves to {resolved:?}, which is unsupported for \
     platform {platform:?} with backend {backend:?}. The engine follows the application's \
     dependencies: link waterui-browser-cef or waterui-browser-wpe to select one, or \
     neither to use the engine this platform bridges."
)]
pub struct UnsupportedWebViewBackend {
    resolved: ResolvedWebViewBackend,
    platform: TargetPlatform,
    backend: TargetBackend,
}

/// `[assets]` section in `Water.toml`: assets the project bundles beyond what
/// dependency crates declare through `[package.metadata.waterui.assets]`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AssetsConfig {
    /// Font families to bundle, one `[[assets.font]]` table per family.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font: Vec<FontConfig>,
}

/// One `[[assets.font]]` declaration in `Water.toml`.
///
/// `name` alone resolves through the CLI's built-in registry; `local_path`
/// bundles a font file relative to the project root; `remote_path` names a
/// face — or an archive containing it — that must already sit in the font
/// cache, since builds perform no network access. A declaration sets at most
/// one source.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FontConfig {
    /// Font family name.
    pub name: String,
    /// Font file relative to the project root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// URL the font — or an archive containing it — is fetched from when
    /// pre-seeding the font cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
}

/// App-specific configuration in `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    /// Optional crate name overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crates: Option<AppCrates>,
}

/// Crate name overrides for the generated crates (`[app.crates]`).
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AppCrates {
    /// Optional override crate name for generated FFI crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffi: Option<CrateName>,
    /// Optional override crate name for generated GTK backend crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gtk: Option<CrateName>,
    /// Optional override crate name for generated hydrolysis backend crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hydrolysis: Option<CrateName>,
    /// Optional override crate name for generated `WinUI` backend crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winui: Option<CrateName>,
}

/// `[package]` section in `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    /// Human-readable name of the application (e.g., "Water Demo").
    pub name: String,
    /// Bundle identifier for the application (e.g., "dev.waterui.waterdemo").
    pub bundle_identifier: BundleIdentifier,
    /// Path to assets directory relative to project root. Defaults to "assets".
    #[serde(
        default = "default_assets_path",
        skip_serializing_if = "is_default_assets_path"
    )]
    pub assets_path: String,
    /// Whether to build as an accessory (headless) app on macOS.
    #[serde(default, skip_serializing_if = "is_false")]
    pub accessory: bool,
    /// Whether the crate is embedded into a host application rather than
    /// owning the app entry itself.
    ///
    /// An embedded crate is a library: `water build` produces the artifact the
    /// host's build system consumes (an AAR on Android) instead of a runnable
    /// app, and `water run`/`water package` refuse.
    #[serde(default, skip_serializing_if = "is_false")]
    pub embedded: bool,
}

/// Reads the `package.name` of a project's `Cargo.toml` — the crate name the
/// generated backends and preview symbols build on.
///
/// Lighter than [`Project::open`]: this only parses the manifest, so callers
/// that need just the crate name (the `water preview`/`water mcp` entry
/// points) do not pay for a full project open.
///
/// # Errors
/// Returns an error if `Cargo.toml` cannot be read or has no `package.name`.
pub async fn read_project_crate_name(project_path: &Path) -> eyre::Result<String> {
    let cargo_toml = project_path.join("Cargo.toml");
    let cargo_content = smol::fs::read_to_string(&cargo_toml).await?;
    let cargo: toml::Table = cargo_content.parse()?;
    cargo
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| eyre::eyre!("Could not find package name in Cargo.toml"))
}

/// Whether two paths name the same directory on disk.
///
/// Compared after canonicalization, because the two sides come from different
/// places — one walked up from the project, one joined from a relative
/// `waterui_path` — and `examples/filter/../..` is the repository root however
/// it is spelled.
fn same_directory(left: &Path, right: &Path) -> std::io::Result<bool> {
    Ok(std::fs::canonicalize(left)? == std::fs::canonicalize(right)?)
}

fn default_assets_path() -> String {
    "assets".to_string()
}

fn is_default_assets_path(path: &str) -> bool {
    path == "assets"
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if predicate signature is fixed as &bool"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod managed_backends_tests {
    use super::{ManagedBackends, TargetBackend, TargetPlatform};

    /// A macOS-only open scaffolds the Apple project and leaves no `android`
    /// managed backend behind; an Android open the reverse.
    #[test]
    fn a_platform_selects_only_the_backend_it_builds_with() {
        for platform in [
            TargetPlatform::MacOS,
            TargetPlatform::IOS,
            TargetPlatform::IOSSimulator,
        ] {
            let selected = ManagedBackends::for_platform(platform);
            assert!(selected.apple(), "{platform:?} builds with Apple");
            assert!(!selected.android(), "{platform:?} leaves Android alone");
        }
        let selected = ManagedBackends::for_platform(TargetPlatform::Android);
        assert!(selected.android());
        assert!(!selected.apple());
    }

    /// The backends generated on demand (GTK4, hydrolysis, `WinUI`, ESP32) are
    /// not initialised by `Project::open`, so their platforms select nothing.
    #[test]
    fn platforms_without_a_managed_native_backend_select_none() {
        for platform in [
            TargetPlatform::Linux,
            TargetPlatform::Windows,
            TargetPlatform::Web,
            TargetPlatform::Esp32S3,
            TargetPlatform::Esp32C3,
            TargetPlatform::Esp32P4,
        ] {
            assert_eq!(
                ManagedBackends::for_platform(platform),
                ManagedBackends::NONE,
                "{platform:?}"
            );
        }
    }

    #[test]
    fn several_platforms_select_the_union_of_their_backends() {
        assert_eq!(
            ManagedBackends::for_platforms(&[TargetPlatform::IOS, TargetPlatform::IOSSimulator]),
            ManagedBackends::for_platform(TargetPlatform::MacOS)
        );
        assert_eq!(
            ManagedBackends::for_platforms(&[TargetPlatform::MacOS, TargetPlatform::Android]),
            ManagedBackends::ALL
        );
        assert_eq!(ManagedBackends::for_platforms(&[]), ManagedBackends::NONE);
    }

    #[test]
    fn a_backend_selects_itself_when_it_is_managed_natively() {
        assert!(ManagedBackends::for_backend(TargetBackend::Apple).apple());
        assert!(!ManagedBackends::for_backend(TargetBackend::Apple).android());
        assert!(ManagedBackends::for_backend(TargetBackend::Android).android());
        assert!(!ManagedBackends::for_backend(TargetBackend::Android).apple());
        for backend in [
            TargetBackend::Gtk4,
            TargetBackend::Hydrolysis,
            TargetBackend::WinUi,
            TargetBackend::Dew,
        ] {
            assert_eq!(
                ManagedBackends::for_backend(backend),
                ManagedBackends::NONE,
                "{backend:?}"
            );
        }
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    #[test]
    fn local_framework_requirement_is_checked_before_project_io() {
        smol::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let framework_root = directory.path().join("framework");
            let project_root = directory.path().join("consumer");
            smol::fs::create_dir(&framework_root).await.unwrap();
            let mut minimum: cargo_toml::SemVer = env!("CARGO_PKG_VERSION").parse().unwrap();
            minimum.major += 1;
            let mut metadata = toml::toml! {
                [package.metadata.waterui]
                minimum-cli-version = "0.1.4"
                android-min-api-level = 31
            };
            metadata["package"]["metadata"]["waterui"]["minimum-cli-version"] =
                toml::Value::String(minimum.to_string());
            smol::fs::write(
                framework_root.join("Cargo.toml"),
                toml::to_string(&metadata).unwrap(),
            )
            .await
            .unwrap();
            let bundle_identifier =
                BundleIdentifier::try_from("dev.waterui.compatibility").unwrap();
            let options = CreateOptions {
                name: "Compatibility".into(),
                bundle_identifier: bundle_identifier.clone(),
                waterui_path: Some(framework_root),
                channel: None,
                framework_manifest: None,
                framework: None,
                framework_lock: None,
                author: String::new(),
                web: None,
            };
            let machine = crate::toolchain::testing::TestMachine::new();
            let error = Project::create(&real_toolchain_host(&machine), &project_root, options)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("requires waterui-cli >= {minimum}")));
            assert!(error.contains(&format!(
                "cargo install waterui-cli --git {} --locked",
                env!("CARGO_PKG_REPOSITORY")
            )));
            assert!(!project_root.exists());

            smol::fs::create_dir(&project_root).await.unwrap();
            let mut manifest = Manifest::new(Package {
                name: "Compatibility".into(),
                bundle_identifier,
                assets_path: default_assets_path(),
                accessory: false,
                embedded: false,
            });
            manifest.waterui_path = Some("../framework".into());
            manifest.save(&project_root).await.unwrap();
            let error =
                Project::open_for_preview_build(&crate::toolchain::Host::current(), &project_root)
                    .await
                    .unwrap_err()
                    .to_string();
            assert!(error.contains(&format!("requires waterui-cli >= {minimum}")));
            assert!(!project_root.join("Cargo.lock").exists());
        });
    }

    #[test]
    fn failed_channel_selection_preserves_project_files() {
        smol::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let originals = [
                ("Cargo.toml", b"original manifest".as_slice()),
                ("Cargo.lock", b"original dependency lock".as_slice()),
                ("Water.toml", b"original project configuration".as_slice()),
            ];
            for (name, contents) in originals {
                smol::fs::write(root.join(name), contents).await.unwrap();
            }
            let updates = ["Cargo.toml", "Cargo.lock", "Water.toml", "Water.lock"]
                .into_iter()
                .map(|name| (root.join(name), Some(b"invalid selected manifest".to_vec())))
                .collect();
            assert!(
                apply_channel_selection(
                    &Host::current(),
                    root,
                    crate::framework::test_fixtures::stable_framework(),
                    updates
                )
                .await
                .is_err()
            );
            for (name, contents) in originals {
                assert_eq!(smol::fs::read(root.join(name)).await.unwrap(), contents);
            }
            assert!(!root.join("Water.lock").exists());
        });
    }
}

#[cfg(test)]
mod webview_backend_tests {
    use crate::toolchain::Host;

    use super::{
        ResolvedWebViewBackend, TargetBackend, TargetPlatform, resolve_enabled_features,
        resolve_linked_runtime_packages,
    };

    /// An application that links no engine crate uses whatever the platform
    /// bridges, and the bridge is not everywhere: Linux outside GTK has none, so
    /// such a build is refused with an explanation instead of producing a
    /// contentless web view at runtime.
    #[test]
    fn the_platform_bridge_is_the_selection_without_an_engine_crate() {
        assert_eq!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::MacOS, TargetBackend::Hydrolysis)
                .expect("macOS Hydrolysis bridges WKWebView"),
            ResolvedWebViewBackend::System
        );
        assert_eq!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::Linux, TargetBackend::Gtk4)
                .expect("GTK bridges WebKitGTK"),
            ResolvedWebViewBackend::System
        );
        assert!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::Linux, TargetBackend::Hydrolysis)
                .is_err()
        );
        assert!(
            ResolvedWebViewBackend::System
                .validate(TargetPlatform::Windows, TargetBackend::Hydrolysis)
                .is_err()
        );
    }

    #[test]
    fn unsupported_engine_combinations_fail_before_build() {
        assert!(
            ResolvedWebViewBackend::Wpe
                .validate(TargetPlatform::MacOS, TargetBackend::Hydrolysis)
                .is_err()
        );
        assert!(
            ResolvedWebViewBackend::Cef
                .validate(TargetPlatform::Android, TargetBackend::Android)
                .is_err()
        );
        assert_eq!(
            ResolvedWebViewBackend::Cef
                .validate(TargetPlatform::MacOS, TargetBackend::Apple)
                .expect("CEF must compose with the native Apple renderer on macOS"),
            ResolvedWebViewBackend::Cef
        );
    }

    #[test]
    fn cef_is_available_to_every_non_dew_backend_on_desktop_platforms() {
        for backend in [
            TargetBackend::Apple,
            TargetBackend::Android,
            TargetBackend::Gtk4,
            TargetBackend::Hydrolysis,
        ] {
            for platform in [
                TargetPlatform::MacOS,
                TargetPlatform::Linux,
                TargetPlatform::Windows,
            ] {
                assert_eq!(
                    ResolvedWebViewBackend::Cef
                        .validate(platform, backend)
                        .expect("CEF availability must not depend on the WaterUI backend"),
                    ResolvedWebViewBackend::Cef
                );
            }
        }
    }

    #[test]
    fn cef_rejects_dew_and_platforms_without_cef_distributions() {
        for platform in [
            TargetPlatform::MacOS,
            TargetPlatform::Linux,
            TargetPlatform::Windows,
        ] {
            assert!(
                ResolvedWebViewBackend::Cef
                    .validate(platform, TargetBackend::Dew)
                    .is_err()
            );
        }
        for (platform, backend) in [
            (TargetPlatform::Android, TargetBackend::Android),
            (TargetPlatform::IOS, TargetBackend::Apple),
            (TargetPlatform::Web, TargetBackend::Hydrolysis),
        ] {
            assert!(
                ResolvedWebViewBackend::Cef
                    .validate(platform, backend)
                    .is_err()
            );
        }
    }

    /// The engine is read out of the application's own graph, so the examples
    /// are the test: the CEF `WebView` example links `waterui-browser-cef` and
    /// the shared system-`WebView` example links no engine at all. The
    /// examples live in the framework repository — this crate builds against a
    /// enclosing workspace — the test resolves and builds it, so it stays
    /// nightly.
    #[test]
    #[ignore = "reads the enclosing workspace checkout"]
    fn runtime_graph_is_scoped_to_the_selected_application() {
        let repository = crate::pinned_framework::checkout();
        // Pinned, not `Triple::host()`: the answers must be the same on
        // every machine the test runs on, and the `webview` example's
        // `waterui-browser-wpe` link — the `#[target]` table bug from the
        // issue — exists only in the Linux graph, making it the
        // regression's own triple.
        let target: target_lexicon::Triple = "x86_64-unknown-linux-gnu"
            .parse()
            .expect("linux triple parses");
        let chromium = smol::block_on(resolve_linked_runtime_packages(
            &Host::current(),
            repository.join("examples/chromium"),
            &target,
            true,
        ))
        .expect("Chromium example runtime graph must resolve");
        assert!(
            chromium.contains_key("waterui-chromium"),
            "Chromium example graph: {chromium:#?}"
        );
        // The Chromium example links the engine it draws through, and nothing
        // else: no second engine, and no `waterui` facade `webview` feature.
        assert!(
            chromium.contains_key("waterui-browser-cef"),
            "Chromium example graph: {chromium:#?}"
        );
        assert!(
            !chromium.contains_key("waterui-browser-wpe"),
            "Chromium example graph: {chromium:#?}"
        );
        // A Chromium-only application shows no standard `WebView`, so
        // `webview_enabled` must be false for it: the Apple scaffold reads this
        // graph to decide whether to link the `WaterUICefWebView` framework.
        // The `waterui-webview` package is present — `waterui-chromium` links
        // it for the shared asset-server types — so the signal is the `webview`
        // feature, which nothing in this subtree turns on.
        assert!(
            chromium.contains_key("waterui-webview"),
            "waterui-chromium shares the webview asset-server types: {chromium:#?}"
        );
        let chromium_features = smol::block_on(resolve_enabled_features(
            &Host::current(),
            repository.join("examples/chromium"),
            &target,
            true,
        ))
        .expect("Chromium example feature graph must resolve");
        assert!(
            !chromium_features.contains("webview"),
            "a Chromium-only application must not enable the standard WebView \
             component: {chromium_features:#?}"
        );

        let webview = smol::block_on(resolve_linked_runtime_packages(
            &Host::current(),
            repository.join("examples/webview"),
            &target,
            true,
        ))
        .expect("WebView example runtime graph must resolve");
        assert!(
            webview.contains_key("waterui-webview"),
            "WebView example graph: {webview:#?}"
        );
        let webview_features = smol::block_on(resolve_enabled_features(
            &Host::current(),
            repository.join("examples/webview"),
            &target,
            true,
        ))
        .expect("WebView example feature graph must resolve");
        assert!(
            webview_features.contains("webview"),
            "the WebView example enables the facade `webview` feature: {webview_features:#?}"
        );
        assert!(
            !webview.contains_key("waterui-browser-cef"),
            "WebView example graph: {webview:#?}"
        );
        assert!(
            !webview.contains_key("waterui-chromium"),
            "WebView example graph: {webview:#?}"
        );
        // The issue's own case: the `wpe` link lives behind
        // `cfg(target_os = "linux")` in the example's manifest, so only a
        // graph resolved for the Linux triple can carry it.
        assert!(
            webview.contains_key("waterui-browser-wpe"),
            "WebView example Linux graph links wpe — the issue's own case: {webview:#?}"
        );

        let cef_webview = smol::block_on(resolve_linked_runtime_packages(
            &Host::current(),
            repository.join("examples/webview-cef"),
            &target,
            true,
        ))
        .expect("CEF WebView example runtime graph must resolve");
        assert!(
            cef_webview.contains_key("waterui-browser-cef"),
            "CEF WebView example graph: {cef_webview:#?}"
        );
        assert!(
            !cef_webview.contains_key("waterui-browser-wpe"),
            "CEF WebView example graph: {cef_webview:#?}"
        );
    }

    /// The `map` capability — the Apple `MapKit` bridge's `-DWATERUI_MAP`, and
    /// the FFI's `map` feature — is read off the application's own graph, the
    /// same way the browser engine is. `waterui-map` is a component crate an
    /// application depends on directly; no facade feature announces it any
    /// more, so linking it is what the capability has to see. The examples
    /// live in this repository — the test resolves them off the enclosing
    /// workspace checkout and builds them, so it stays nightly.
    #[test]
    #[ignore = "reads the enclosing workspace checkout"]
    fn the_map_capability_is_read_from_the_application_graph() {
        let repository = crate::pinned_framework::checkout();
        // Pinned, not `Triple::host()`: an Android triple also exercises
        // the serving set the generated Android table renders for.
        let target: target_lexicon::Triple = "aarch64-linux-android"
            .parse()
            .expect("android triple parses");

        let map = smol::block_on(resolve_linked_runtime_packages(
            &Host::current(),
            repository.join("examples/map"),
            &target,
            true,
        ))
        .expect("map example runtime graph must resolve");
        assert!(
            map.contains_key("waterui-map"),
            "map example graph: {map:#?}"
        );

        let webview = smol::block_on(resolve_linked_runtime_packages(
            &Host::current(),
            repository.join("examples/webview"),
            &target,
            true,
        ))
        .expect("WebView example runtime graph must resolve");
        assert!(
            !webview.contains_key("waterui-map"),
            "an application that shows no map must not carry the map stack: {webview:#?}"
        );
    }
}

/// A host that runs the real toolchain but whose Water home is
/// `machine`'s — a project opened or created for a real-cargo test writes
/// nothing under `~/.water`.
#[cfg(test)]
fn real_toolchain_host(machine: &crate::toolchain::testing::TestMachine) -> Host {
    crate::toolchain::testing::real_toolchain_host(machine.home())
}

#[cfg(test)]
mod target_graph_tests {
    use std::path::{Path, PathBuf};
    use std::str::FromStr as _;

    use target_lexicon::Triple;

    use crate::platform::{TargetBackend, TargetPlatform};
    use crate::project::ResolvedWebViewBackend;
    use crate::toolchain::testing::TestMachine;

    use super::{ManagedBackends, Manifest, Project};

    const LINUX: &str = "x86_64-unknown-linux-gnu";
    const WINDOWS: &str = "x86_64-pc-windows-msvc";

    /// A `cargo metadata` response for the one-package fixture at `root`,
    /// shaped the way `cargo metadata --no-deps` prints it: the layout
    /// `resolve_cargo_layout` needs plus the root-package row `cargo_tree`
    /// matches by manifest path.
    fn cargo_metadata_json(root: &Path, workspace_root: &Path) -> String {
        let canonical = dunce::canonicalize(root).expect("fixture root canonicalizes");
        let workspace_root =
            dunce::canonicalize(workspace_root).expect("workspace root canonicalizes");
        let manifest_path = canonical.join("Cargo.toml");
        let package_id = format!(
            "path+file:///{}#demo_app@0.1.0",
            canonical.display().to_string().replace('\\', "/")
        );
        serde_json::json!({
            "packages": [{
                "name": "demo_app",
                "version": "0.1.0",
                "id": package_id,
                "license": null,
                "license_file": null,
                "description": null,
                "source": null,
                "dependencies": [],
                "manifest_path": manifest_path.to_string_lossy(),
                "categories": [],
                "keywords": [],
                "readme": null,
                "repository": null,
                "homepage": null,
                "documentation": null,
                "edition": "2021",
                "links": null,
                "default_run": null,
                "rust_version": null,
                "metadata": null,
                "features": {},
                "targets": [{
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "demo_app",
                    "src_path": canonical.join("src/lib.rs").to_string_lossy(),
                    "edition": "2021",
                    "doc": true,
                    "doctest": true,
                    "test": true
                }],
                "publish": null,
                "authors": []
            }],
            "workspace_members": [package_id],
            "workspace_default_members": [package_id],
            "resolve": null,
            "target_directory": workspace_root.join("target").to_string_lossy(),
            "version": 1,
            "workspace_root": workspace_root.to_string_lossy(),
            "metadata": null
        })
        .to_string()
    }

    /// A project `Project::open` accepts — real manifests on the scratch
    /// filesystem — with the fake `cargo` installed and `cargo metadata`
    /// answered. `cargo tree --target <triple>` answers are the caller's to
    /// stage, one `CARGO_TREE_<triple>` response per triple.
    fn fixture(machine: &TestMachine) -> PathBuf {
        let root = machine.root().join("app");
        std::fs::create_dir_all(root.join("src")).expect("fixture dirs");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo_app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("fixture Cargo.toml");
        std::fs::write(root.join("src/lib.rs"), "").expect("fixture lib");
        let mut manifest = Manifest::parse(
            "[package]\nname = \"Demo App\"\nbundle_identifier = \"dev.waterui.demo\"\n",
        )
        .expect("fixture Water.toml parses");
        manifest.framework = Some(crate::framework::test_fixtures::stable_framework());
        smol::block_on(manifest.save(&root)).expect("fixture Water.toml");

        machine.install("cargo");
        machine.install("rustc");
        machine.respond(
            "RUSTC_VERBOSE",
            "rustc 1.99.0 (waterui-test)\n\
             binary: rustc\n\
             host: x86_64-unknown-linux-gnu\n\
             release: 1.99.0\n\
             LLVM version: 20.1.0\n",
        );
        machine.respond("CARGO_METADATA", &cargo_metadata_json(&root, &root));
        root
    }

    /// A workspace fixture: `ws/Cargo.toml` carries `[workspace]` over the
    /// `app` and `dep` members, so the persisted entry's key records three
    /// manifests — the project's own, the path package's, and the
    /// workspace root's — each in its own file an edit can touch alone.
    fn workspace_fixture(machine: &TestMachine) -> (PathBuf, PathBuf, PathBuf) {
        let ws = machine.root().join("ws");
        let app = ws.join("app");
        let dep = ws.join("dep");
        for dir in [app.join("src"), dep.join("src")] {
            std::fs::create_dir_all(&dir).expect("fixture dirs");
            std::fs::write(dir.join("lib.rs"), "").expect("fixture lib");
        }
        std::fs::write(
            app.join("Cargo.toml"),
            "[package]\nname = \"demo_app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("app manifest");
        std::fs::write(
            dep.join("Cargo.toml"),
            "[package]\nname = \"pathdep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("dep manifest");
        std::fs::write(
            ws.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"dep\"]\nresolver = \"2\"\n",
        )
        .expect("workspace manifest");
        std::fs::write(ws.join("Cargo.lock"), "# this file is @generated\n")
            .expect("workspace lockfile");
        let mut manifest = Manifest::parse(
            "[package]\nname = \"Demo App\"\nbundle_identifier = \"dev.waterui.demo\"\n",
        )
        .expect("fixture Water.toml parses");
        manifest.framework = Some(crate::framework::test_fixtures::stable_framework());
        smol::block_on(manifest.save(&app)).expect("fixture Water.toml");

        machine.install("cargo");
        machine.install("rustc");
        machine.respond(
            "RUSTC_VERBOSE",
            "rustc 1.99.0 (waterui-test)\n\
             binary: rustc\n\
             host: x86_64-unknown-linux-gnu\n\
             release: 1.99.0\n\
             LLVM version: 20.1.0\n",
        );
        machine.respond("CARGO_METADATA", &cargo_metadata_json(&app, &ws));
        (ws, app, dep)
    }

    /// Open the fixture project on `host` and ask its linux graph for the
    /// linked browser engine — the one-call resolve the persisted-layer
    /// test counts `cargo tree` invocations through.
    async fn resolve_graph_answer(host: &crate::toolchain::Host, app_root: &Path, linux: &Triple) {
        Project::open(host, app_root, ManagedBackends::NONE)
            .await
            .expect("the project opens on the fixture machine")
            .linked_browser_engine(linux)
            .await
            .expect("the linux graph resolves");
    }

    /// One project, two build targets, one question: the graph a build is
    /// planned from is the graph `cargo tree --target <triple>` resolves
    /// for the build's own target — and each triple's resolve is cached
    /// under its own triple, so a repeat asks nothing and one target's
    /// answer never serves another.
    #[test]
    fn the_runtime_graph_resolves_per_build_target() {
        smol::block_on(async {
            let machine = TestMachine::new();
            let root = fixture(&machine);

            let linux = Triple::from_str(LINUX).expect("linux triple");
            let windows = Triple::from_str(WINDOWS).expect("windows triple");
            // The tree's path rows name real files — the persisted entry
            // hashes every one, so an unreadable one is an error now.
            let app = dunce::canonicalize(&root).expect("fixture root canonicalizes");
            machine.respond(
                &format!("CARGO_TREE_{LINUX}"),
                &format!(
                    "demo_app v0.1.0 ({})\n\
                     waterui-browser-wpe v0.4.1 \
                     (registry+https://github.com/rust-lang/crates.io-index)\n",
                    app.display()
                ),
            );
            machine.respond(
                &format!("CARGO_TREE_{WINDOWS}"),
                &format!("demo_app v0.1.0 ({})\n", app.display()),
            );

            let log = machine.file("invocations.log", "");
            let host = machine.host([("WATERUI_FAKE_LOG", log.as_os_str())]);
            let project = Project::open(&host, &root, ManagedBackends::NONE)
                .await
                .expect("the project opens on the fixture machine");

            assert_eq!(
                project
                    .linked_browser_engine(&linux)
                    .await
                    .expect("linux graph resolves"),
                Some(ResolvedWebViewBackend::Wpe),
            );
            assert_eq!(
                project
                    .linked_browser_engine(&windows)
                    .await
                    .expect("windows graph resolves"),
                None,
            );
            // The cache is per triple: re-asking for the same target
            // resolves nothing again.
            project
                .linked_browser_engine(&linux)
                .await
                .expect("the cached linux graph still answers");

            let invocations = std::fs::read_to_string(&log).expect("invocation log");
            for target in [&linux, &windows] {
                assert_eq!(
                    invocations
                        .lines()
                        .filter(|line| line.starts_with("cargo tree"))
                        .filter(|line| line.contains(&format!("--target {target}")))
                        .count(),
                    1,
                    "exactly one `cargo tree --target` for {target}: {invocations}"
                );
            }
        });
    }

    /// Each OS's section reads its own serving set's graphs: the Hydrolysis
    /// manifest's per-OS tables take Linux's `wpe` answer where macOS's
    /// takes none — the per-target split the disagreement error names.
    #[test]
    fn native_browser_answers_resolve_per_os() {
        smol::block_on(async {
            let machine = TestMachine::new();
            let root = fixture(&machine);

            let macos = TargetPlatform::MacOS.triple();
            let app = dunce::canonicalize(&root).expect("fixture root canonicalizes");
            for target in crate::platform::NativeOs::Linux.serving_triples() {
                machine.respond(
                    &format!("CARGO_TREE_{target}"),
                    &format!(
                        "demo_app v0.1.0 ({})\n\
                         waterui-webview v0.4.1 \
                         (registry+https://github.com/rust-lang/crates.io-index)\n\
                         waterui-browser-wpe v0.4.1 \
                         (registry+https://github.com/rust-lang/crates.io-index)\n\
                         waterui-webview feature \"webview\"\n",
                        app.display()
                    ),
                );
            }
            machine.respond(
                &format!("CARGO_TREE_{macos}"),
                &format!(
                    "demo_app v0.1.0 ({})\n\
                     waterui-webview v0.4.1 \
                     (registry+https://github.com/rust-lang/crates.io-index)\n\
                     waterui-webview feature \"webview\"\n",
                    app.display()
                ),
            );

            let host = machine.host::<&'static str, &'static str>([]);
            let project = Project::open(&host, &root, ManagedBackends::NONE)
                .await
                .expect("the project opens on the fixture machine");

            let section = |os: crate::platform::NativeOs| super::GraphSection {
                manifest: "the generated Hydrolysis launcher manifest",
                table: os.cfg(),
                remedy: "the dependency graph answers differently per target, so that \
                         section needs a per-target split inside its own OS",
            };
            let linux = project
                .native_browser_answers(
                    crate::platform::NativeOs::Linux,
                    section(crate::platform::NativeOs::Linux),
                )
                .await
                .expect("linux answers resolve");
            assert_eq!(linux.engine, Some(ResolvedWebViewBackend::Wpe));
            assert!(linux.webview_enabled);

            let macos_answers = project
                .native_browser_answers(
                    crate::platform::NativeOs::MacOs,
                    section(crate::platform::NativeOs::MacOs),
                )
                .await
                .expect("macOS answers resolve");
            assert_eq!(macos_answers.engine, None);
            assert!(macos_answers.webview_enabled);
        });
    }

    /// A generated manifest's serving set must answer every triple it
    /// serves the same — the section cannot express a per-target
    /// difference — so a graph that resolves two ways fails, naming the
    /// triples and the differing answers.
    #[test]
    fn a_serving_set_whose_graphs_disagree_fails_naming_the_targets() {
        smol::block_on(async {
            let machine = TestMachine::new();
            let root = fixture(&machine);

            let linux = Triple::from_str(LINUX).expect("linux triple");
            let windows = Triple::from_str(WINDOWS).expect("windows triple");
            // The tree's path rows name real files — the persisted entry
            // hashes every one, so an unreadable one is an error now.
            let app = dunce::canonicalize(&root).expect("fixture root canonicalizes");
            machine.respond(
                &format!("CARGO_TREE_{LINUX}"),
                &format!(
                    "demo_app v0.1.0 ({})\n\
                     waterui-browser-wpe v0.4.1 \
                     (registry+https://github.com/rust-lang/crates.io-index)\n",
                    app.display()
                ),
            );
            machine.respond(
                &format!("CARGO_TREE_{WINDOWS}"),
                &format!("demo_app v0.1.0 ({})\n", app.display()),
            );

            let host = machine.host::<&'static str, &'static str>([]);
            let project = Project::open(&host, &root, ManagedBackends::NONE)
                .await
                .expect("the project opens on the fixture machine");

            let error = project
                .linked_browser_engine_for(
                    &[linux, windows],
                    super::GraphSection {
                        manifest: "the generated Hydrolysis manifest",
                        table: crate::platform::NativeOs::Linux.cfg(),
                        remedy: "the dependency graph answers differently per target, so \
                                 that section needs a per-target split inside its own OS",
                    },
                )
                .await
                .expect_err("the serving set's graphs disagree")
                .to_string();
            assert!(error.contains(LINUX), "{error}");
            assert!(error.contains(WINDOWS), "{error}");
            assert!(error.contains("wpe"), "{error}");
            assert!(
                error.contains("the generated Hydrolysis manifest"),
                "the error names the generated manifest: {error}"
            );
            assert!(
                error.contains("cfg(target_os = \"linux\")"),
                "the error names the table: {error}"
            );
            assert!(
                error.contains("per-target split"),
                "the error states the remedy: {error}"
            );
        });
    }

    /// The open `water clean` performs — `Project::open` under
    /// managed-backend selection — resolves no dependency graph: rendering
    /// target-derived scaffolds is the build's step, never the open's.
    #[test]
    fn opening_with_managed_backends_resolves_no_dependency_graph() {
        smol::block_on(async {
            let machine = TestMachine::new();
            let root = fixture(&machine);

            let log = machine.file("invocations.log", "");
            let host = machine.host([("WATERUI_FAKE_LOG", log.as_os_str())]);

            // `water clean --backend apple` opens exactly this way.
            Project::open(
                &host,
                &root,
                ManagedBackends::for_backend(TargetBackend::Apple),
            )
            .await
            .expect("a backend-selected open succeeds on the fixture machine");

            let invocations = std::fs::read_to_string(&log).expect("invocation log");
            assert!(
                invocations
                    .lines()
                    .any(|line| line.starts_with("cargo metadata")),
                "the open did run `cargo metadata`: {invocations}"
            );
            assert!(
                invocations
                    .lines()
                    .all(|line| !line.starts_with("cargo tree")),
                "opening with managed backends must not run `cargo tree`: {invocations}"
            );
        });
    }

    /// The persisted layer: a second `Project::open` on the same project
    /// replays the recorded answer without one `cargo tree` invocation —
    /// asserted through the fake tool's invocation log — and every input
    /// the key records re-resolves when it changes: the project manifest,
    /// the lockfile, a path package's manifest, the workspace root
    /// manifest, the `.cargo/config.toml` rustflags and `RUSTFLAGS` in the
    /// host's environment.
    #[test]
    fn a_second_open_replays_the_recorded_answer_and_edits_re_resolve() {
        smol::block_on(async {
            let machine = TestMachine::new();
            let (ws, app_root, dep) = workspace_fixture(&machine);
            let linux = Triple::from_str(LINUX).expect("linux triple");
            let app = dunce::canonicalize(&app_root).expect("app canonicalizes");
            let dep = dunce::canonicalize(&dep).expect("dep canonicalizes");
            machine.respond(
                &format!("CARGO_TREE_{LINUX}"),
                &format!(
                    "demo_app v0.1.0 ({})\npathdep v0.1.0 ({})\n",
                    app.display(),
                    dep.display()
                ),
            );

            let log = machine.file("tree-invocations.log", "");
            let host = machine.host([("WATERUI_FAKE_LOG", log.as_os_str())]);
            let tree_calls = |log: &Path| {
                std::fs::read_to_string(log)
                    .expect("invocation log")
                    .lines()
                    .filter(|line| line.starts_with("cargo tree"))
                    .count()
            };

            resolve_graph_answer(&host, &app_root, &linux).await;
            let mut expected = 1;
            // The second open answers from the recorded entry alone.
            resolve_graph_answer(&host, &app_root, &linux).await;
            assert_eq!(
                tree_calls(&log),
                expected,
                "the persisted answer replays without a resolve"
            );

            for edited in [
                app.join("Cargo.toml"), // the project manifest
                ws.join("Cargo.lock"),  // the lockfile
                dep.join("Cargo.toml"), // a path package's manifest
                ws.join("Cargo.toml"),  // the workspace root manifest
            ] {
                let text = std::fs::read_to_string(&edited).expect("the input reads");
                std::fs::write(&edited, format!("{text}\n# edited\n")).expect("the input writes");
                expected += 1;
                resolve_graph_answer(&host, &app_root, &linux).await;
                assert_eq!(
                    tree_calls(&log),
                    expected,
                    "{} changed: the resolve re-ran",
                    edited.display()
                );
            }

            // The `.cargo` rustflags chain enters the key too — a config
            // file appearing in it is a new input.
            std::fs::create_dir_all(ws.join(".cargo")).expect("cargo config dir");
            std::fs::write(
                ws.join(".cargo/config.toml"),
                "[build]\nrustflags = [\"--cfg\", \"fixture_flag\"]\n",
            )
            .expect("cargo config");
            expected += 1;
            resolve_graph_answer(&host, &app_root, &linux).await;
            assert_eq!(tree_calls(&log), expected, "cargo config rustflags");

            // And `RUSTFLAGS` the host environment declares.
            let rustflags_host = machine.host([
                ("WATERUI_FAKE_LOG", log.as_os_str()),
                ("RUSTFLAGS", std::ffi::OsStr::new("--cfg fixture_env")),
            ]);
            expected += 1;
            resolve_graph_answer(&rustflags_host, &app_root, &linux).await;
            assert_eq!(tree_calls(&log), expected, "RUSTFLAGS in the host env");
        });
    }
}

#[cfg(test)]
mod scaffold_tests {
    use std::path::{Path, PathBuf};

    use super::{BundleIdentifier, CreateOptions, ManagedBackends, Project, real_toolchain_host};

    /// `water create "1573 App"` derives `1573_app` — a name Cargo rejects
    /// as a package name because it cannot start with a digit. The
    /// derivation refuses it before any scaffold output, naming the derived
    /// name and the rule rather than inventing a mangled name.
    #[test]
    fn a_digit_led_display_name_derives_no_crate_name() {
        let options = CreateOptions {
            name: "1573 App".to_string(),
            bundle_identifier: BundleIdentifier::try_from("dev.waterui.app1573")
                .expect("bundle identifier"),
            waterui_path: None,
            channel: None,
            framework_manifest: None,
            framework: None,
            framework_lock: None,
            author: "water test".to_string(),
            web: None,
        };
        let error = options
            .crate_name()
            .expect_err("the derived name is rejected");
        let message = error.to_string();
        assert!(
            message.contains("1573_app") && message.contains("digit"),
            "the error names the derived name and Cargo's rule: {message}"
        );
    }

    /// The documented `assets!` workflow requires the assets root to exist: the
    /// planner walks it recursively, so a missing directory fails the first
    /// `assets!` call. `water create` must therefore produce it, tracked, and at
    /// exactly the path the generated `Water.toml` declares.
    #[test]
    fn create_scaffolds_the_assets_directory_declared_by_the_manifest() {
        let machine = crate::toolchain::testing::TestMachine::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("water-example");

        let project = smol::block_on(Project::create(
            &real_toolchain_host(&machine),
            &root,
            CreateOptions {
                name: "Water Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                    .expect("bundle identifier"),
                waterui_path: None,
                channel: None,
                framework_manifest: None,
                // A channel resolution would fetch the newest release from
                // GitHub; a unit test resolves a fixture in place instead.
                framework: Some(crate::framework::test_fixtures::stable_framework()),
                framework_lock: None,
                author: "Lexo Liu".to_string(),
                web: None,
            },
        ))
        .expect("project creation must succeed");

        let assets = project.assets_dir();
        assert!(
            assets.is_dir(),
            "the assets root {} must exist after `water create`",
            assets.display()
        );
        assert_eq!(
            assets,
            root.join(project.assets_path()),
            "the scaffolded directory must be the one the manifest declares"
        );
        assert!(
            assets.join("README.md").is_file(),
            "a tracked file keeps the assets directory present in git"
        );
    }

    /// Generated crate names carry the project-root tag that keeps a shared
    /// Cargo target directory unambiguous; the names packaged binaries ship
    /// under drop it — a checkout path must never appear in a shipped
    /// executable name.
    #[test]
    fn shipped_binary_names_drop_the_project_root_tag() {
        let machine = crate::toolchain::testing::TestMachine::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("water-example");
        let project = smol::block_on(Project::create(
            &real_toolchain_host(&machine),
            &root,
            CreateOptions {
                name: "Water Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                    .expect("bundle identifier"),
                waterui_path: None,
                channel: None,
                framework_manifest: None,
                framework: Some(crate::framework::test_fixtures::stable_framework()),
                framework_lock: None,
                author: "Lexo Liu".to_string(),
                web: None,
            },
        ))
        .expect("project creation must succeed");

        for (shipped, tagged) in [
            (project.gtk4_binary_name(), project.gtk_backend_crate_name()),
            (
                project.hydrolysis_binary_name(),
                project.hydrolysis_backend_crate_name(),
            ),
            (
                project.winui_binary_name(),
                project.winui_backend_crate_name(),
            ),
            (
                project.esp32_binary_name(),
                project.esp32_backend_crate_name(),
            ),
        ] {
            assert!(
                tagged.as_str().starts_with(&format!("{shipped}-")),
                "the build name must be the shipped name plus the tag: {tagged}"
            );
            assert_eq!(
                tagged.as_str().len() - shipped.as_str().len(),
                9,
                "the tag is a dash plus eight hex digits: {tagged}"
            );
        }
    }

    /// The `waterui-ffi` feature table every vendored stub carries.
    const FFI_FEATURES: &[&str] = &[
        "android-jni",
        "c-api",
        "chromium",
        "dev",
        "gpu",
        "inspector",
        "map",
        "media",
        "video",
        "webview",
        "webview-cef",
    ];

    /// Stage the canonical framework checkout the `apple_backend` arm of
    /// [`create_project`] names in `Water.toml`: the `waterui` facade as root
    /// package, `waterui-ffi` at `ffi`, `waterui-apple` at `backends/apple`.
    fn stage_waterui_checkout(root: &Path, vendor_dir: &Path) {
        let mut waterui_manifest = toml_edit::DocumentMut::new();
        waterui_manifest["package"]["name"] = toml_edit::value("waterui");
        waterui_manifest["package"]["version"] = toml_edit::value("0.4.1");
        waterui_manifest["package"]["edition"] = toml_edit::value("2021");
        for feature in ["dynamic_linking", "media"] {
            waterui_manifest["features"][feature] = toml_edit::value(toml_edit::Array::new());
        }
        waterui_manifest["workspace"]["members"] =
            toml_edit::value(toml_edit::Array::from_iter(["ffi", "backends/apple"]));
        std::fs::create_dir_all(vendor_dir.join("src")).expect("waterui stub dir");
        std::fs::write(vendor_dir.join("Cargo.toml"), waterui_manifest.to_string())
            .expect("waterui checkout manifest");
        std::fs::write(vendor_dir.join("src/lib.rs"), "").expect("waterui lib");
        crate::framework::test_fixtures::write_vendor_stub(
            &vendor_dir.join("ffi"),
            "waterui-ffi",
            FFI_FEATURES,
        );
        crate::framework::test_fixtures::write_vendor_stub(
            &vendor_dir.join("backends/apple"),
            "waterui-apple",
            &["map", "media", "webview"],
        );
        let water_toml = root.join("Water.toml");
        let mut water_document: toml_edit::DocumentMut = std::fs::read_to_string(&water_toml)
            .expect("Water.toml exists")
            .parse()
            .expect("Water.toml parses");
        water_document["waterui_path"] = toml_edit::value(vendor_dir.to_string_lossy().as_ref());
        std::fs::write(&water_toml, water_document.to_string())
            .expect("name the vendored framework checkout");
    }

    /// A remote-channel project whose framework pins resolve without a
    /// network: `waterui` and `waterui-ffi` ride `[patch.crates-io]` onto
    /// vendor stubs (the only source `[patch]` can redirect offline), and
    /// `waterui-apple` rides the canonical `backends/apple` slot of the
    /// vendored checkout `waterui_path` names — a path dependency — so the
    /// ffi companion's feature-table probe resolves entirely locally.
    /// `apple_backend` stages that checkout and names it in `Water.toml` for
    /// the opens that select the Apple backend.
    fn create_project(
        host: &crate::toolchain::Host,
        root: &Path,
        vendor_dir: &Path,
        apple_backend: bool,
    ) -> Project {
        let project = smol::block_on(Project::create(
            host,
            root,
            CreateOptions {
                name: "Water Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                    .expect("bundle identifier"),
                waterui_path: None,
                channel: None,
                framework_manifest: None,
                framework: Some(crate::framework::test_fixtures::stable_framework()),
                framework_lock: None,
                author: "Lexo Liu".to_string(),
                web: None,
            },
        ))
        .expect("project creation must succeed");

        if apple_backend {
            stage_waterui_checkout(root, vendor_dir);
        } else {
            crate::framework::test_fixtures::write_vendor_stub(
                &vendor_dir.join("waterui"),
                "waterui",
                &["dynamic_linking", "media"],
            );
            crate::framework::test_fixtures::write_vendor_stub(
                &vendor_dir.join("waterui-ffi"),
                "waterui-ffi",
                FFI_FEATURES,
            );
        }
        let manifest_path = root.join("Cargo.toml");
        let mut document: toml_edit::DocumentMut = std::fs::read_to_string(&manifest_path)
            .expect("project Cargo.toml exists")
            .parse()
            .expect("project Cargo.toml parses");
        let patch_targets: [(String, PathBuf); 2] = if apple_backend {
            [
                ("waterui".to_string(), vendor_dir.to_path_buf()),
                ("waterui-ffi".to_string(), vendor_dir.join("ffi")),
            ]
        } else {
            [
                ("waterui".to_string(), vendor_dir.join("waterui")),
                ("waterui-ffi".to_string(), vendor_dir.join("waterui-ffi")),
            ]
        };
        for (name, dir) in patch_targets {
            document["patch"]["crates-io"][name]["path"] =
                toml_edit::value(dir.to_string_lossy().as_ref());
        }
        std::fs::write(&manifest_path, document.to_string()).expect("write the patch table");

        // `Project::open` resolves the project's layout with `cargo metadata
        // --locked`; a plain offline resolve records the patched sources in
        // the lock first.
        cargo_metadata::MetadataCommand::new()
            .manifest_path(&manifest_path)
            .other_options(vec!["--offline".to_string()])
            .exec()
            .expect("offline metadata resolves the patched project");

        project
    }

    /// Opening a project for one platform scaffolds the managed backend
    /// that platform builds with and nothing else: a macOS open must not
    /// leave an Android project behind, and an Android open no Apple project.
    #[test]
    fn opening_a_project_scaffolds_only_the_platforms_managed_backend() {
        use crate::android::backend::AndroidBackend;
        use crate::apple::backend::AppleBackend;
        use crate::platform::TargetPlatform;

        let machine = crate::toolchain::testing::TestMachine::new();

        for (platform, apple_expected) in [
            (TargetPlatform::MacOS, true),
            (TargetPlatform::Android, false),
        ] {
            let dir = tempfile::tempdir().expect("temp dir");
            let root = dir.path().join("water-example");
            create_project(
                &real_toolchain_host(&machine),
                &root,
                dir.path(),
                apple_expected,
            );

            let project = smol::block_on(Project::open(
                &real_toolchain_host(&machine),
                &root,
                ManagedBackends::for_platform(platform),
            ))
            .expect("opening the project must succeed");

            let apple_path = project.backend_path::<AppleBackend>();
            let android_path = project.backend_path::<AndroidBackend>();
            assert_eq!(
                project.apple_backend().is_some(),
                apple_expected,
                "{platform:?}: apple backend"
            );
            assert_eq!(
                project.android_backend().is_some(),
                !apple_expected,
                "{platform:?}: android backend"
            );
            assert_eq!(
                apple_path.exists(),
                apple_expected,
                "{platform:?}: {}",
                apple_path.display()
            );
            assert_eq!(
                android_path.exists(),
                !apple_expected,
                "{platform:?}: {}",
                android_path.display()
            );
        }
    }

    /// An ffi companion a previous render left behind — a manifest naming a
    /// `waterui-apple` path dependency that no longer resolves plus the
    /// entry-owning bin file — is not touched by `Project::open`: rendering
    /// target-derived scaffolds is no open's job. The build path's render
    /// replaces it wholesale before anything reads the manifest, and the
    /// open itself must not fail on the stale manifest.
    #[test]
    fn open_leaves_a_stale_ffi_companion_for_the_builds_render() {
        use crate::platform::TargetPlatform;

        let machine = crate::toolchain::testing::TestMachine::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("water-example");
        create_project(&real_toolchain_host(&machine), &root, dir.path(), false);

        // A `waterui_path` checkout carrying the pieces the render
        // resolves: `waterui` at the root, `waterui-ffi` at `ffi` under
        // `[workspace.dependencies]`, and the `backends/apple` member slot
        // `waterui-apple` resolves through.
        let waterui_root = dir.path().join("waterui");
        let mut waterui_manifest = toml_edit::DocumentMut::new();
        waterui_manifest["package"]["name"] = toml_edit::value("waterui");
        waterui_manifest["package"]["version"] = toml_edit::value("0.4.1");
        waterui_manifest["package"]["edition"] = toml_edit::value("2021");
        for feature in ["dynamic_linking", "media"] {
            waterui_manifest["features"][feature] = toml_edit::value(toml_edit::Array::new());
        }
        waterui_manifest["workspace"]["members"] =
            toml_edit::value(toml_edit::Array::from_iter(["ffi", "backends/apple"]));
        waterui_manifest["workspace"]["dependencies"]["waterui-ffi"]["path"] =
            toml_edit::value("ffi");
        std::fs::create_dir_all(waterui_root.join("src")).expect("waterui root dir");
        std::fs::write(
            waterui_root.join("Cargo.toml"),
            waterui_manifest.to_string(),
        )
        .expect("waterui root manifest");
        std::fs::write(waterui_root.join("src/lib.rs"), "").expect("waterui lib");
        crate::framework::test_fixtures::write_vendor_stub(
            &waterui_root.join("ffi"),
            "waterui-ffi",
            &[],
        );
        crate::framework::test_fixtures::write_vendor_stub(
            &waterui_root.join("backends/apple"),
            "waterui-apple",
            &["map", "media", "webview"],
        );
        // The stale companion's `waterui-apple` dependency names a slot that
        // does not exist — nothing may resolve the stale manifest at all.
        let missing_apple = waterui_root.join("backends/old-apple");
        let water_toml = root.join("Water.toml");
        let mut document: toml_edit::DocumentMut = std::fs::read_to_string(&water_toml)
            .expect("Water.toml exists")
            .parse()
            .expect("Water.toml parses");
        document["waterui_path"] = toml_edit::value(waterui_root.to_string_lossy().as_ref());
        std::fs::write(&water_toml, document.to_string()).expect("name the framework checkout");

        // Shape the build cache before seeding, on the machine the open
        // runs on: `ensure_project_build_cache` records the project root
        // and this CLI's commit in its metadata and wipes any cache
        // directory whose metadata does not match, so only a companion
        // left inside a shaped cache survives to the open.
        let ffi_dir = smol::block_on(crate::water_dir::ensure_project_build_cache(
            &real_toolchain_host(&machine),
            &root,
        ))
        .expect("build cache dir")
        .join("ffi");

        // The stale companion: a manifest carrying a `waterui-apple` path
        // dependency that no longer resolves plus the entry file, so a
        // `cargo metadata` on it would fail.
        std::fs::create_dir_all(ffi_dir.join("src/bin")).expect("stale ffi dir");
        let mut stale_ffi = toml_edit::DocumentMut::new();
        stale_ffi["package"]["name"] = toml_edit::value("water-example-ffi");
        stale_ffi["package"]["version"] = toml_edit::value("0.1.0");
        stale_ffi["package"]["edition"] = toml_edit::value("2021");
        stale_ffi["dependencies"]["waterui-apple"]["path"] =
            toml_edit::value(missing_apple.to_string_lossy().as_ref());
        let stale_manifest = stale_ffi.to_string();
        std::fs::write(ffi_dir.join("Cargo.toml"), &stale_manifest)
            .expect("seed the stale ffi manifest");
        std::fs::write(ffi_dir.join("src/lib.rs"), "").expect("seed the stale ffi lib");
        std::fs::write(ffi_dir.join("src/bin/waterui-apple-main.rs"), "")
            .expect("seed the stale apple entry file");

        let project = smol::block_on(Project::open(
            &real_toolchain_host(&machine),
            &root,
            ManagedBackends::for_platform(TargetPlatform::Android),
        ))
        .expect("the open must not read the stale companion's unresolvable manifest");

        // The open leaves the companion as it found it — the render is the
        // build path's step.
        let on_disk = std::fs::read_to_string(project.ffi_crate_path().join("Cargo.toml"))
            .expect("the companion manifest still reads");
        assert_eq!(
            on_disk, stale_manifest,
            "the open must leave the stale companion alone"
        );

        smol::block_on(project.scaffold_ffi_companion())
            .expect("the build path's render rewrites the stale companion");

        let rendered = std::fs::read_to_string(project.ffi_crate_path().join("Cargo.toml"))
            .expect("the re-rendered ffi manifest");
        let manifest = rendered
            .parse::<toml::Table>()
            .expect("the re-rendered manifest parses");
        let entry_file = project
            .ffi_crate_path()
            .join("src/bin/waterui-apple-main.rs");
        if cfg!(target_os = "macos") {
            // A host that can produce an Apple build renders the Apple
            // pieces — the manifest is a function of the project, not of
            // the invocation that rendered it.
            assert!(rendered.contains("waterui-apple"), "{rendered}");
            assert!(entry_file.exists(), "the Apple entry file is rendered");
        } else {
            assert!(!rendered.contains("waterui-apple"), "{rendered}");
            assert!(
                manifest
                    .get("bin")
                    .and_then(toml::Value::as_array)
                    .is_none_or(Vec::is_empty),
                "a companion rendered off macOS declares no entry-owning bin"
            );
            assert!(
                !entry_file.exists(),
                "a companion rendered off macOS removes the stale entry file"
            );
        }
    }

    /// The companion's Apple pieces exist only where an Apple build can
    /// run: an apple-selected render on a host that cannot produce one
    /// emits no `waterui-apple` — the manifest it writes must resolve on
    /// the host that rendered it.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn apple_selected_companion_carries_no_apple_pieces_off_macos() {
        let machine = crate::toolchain::testing::TestMachine::new();
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("water-example");
        let project = create_project(&real_toolchain_host(&machine), &root, dir.path(), true);

        smol::block_on(project.scaffold_ffi_companion())
            .expect("an apple-selected scaffold must succeed");

        let rendered = std::fs::read_to_string(project.ffi_crate_path().join("Cargo.toml"))
            .expect("the rendered ffi manifest");
        assert!(!rendered.contains("waterui-apple"), "{rendered}");
        assert!(
            !project
                .ffi_crate_path()
                .join("src/bin/waterui-apple-main.rs")
                .exists(),
            "a companion rendered off macOS renders no apple entry file"
        );
    }

    /// Packaged executables stage under the project's own managed backend
    /// directory — `dist/<platform>/<profile>` below `backend_path` — so
    /// two projects sharing a crate name, most often two worktrees of one
    /// project, never write the same shipped path the way the shared Cargo
    /// profile directory made them.
    #[test]
    fn same_named_projects_stage_packaged_binaries_under_their_own_backends() {
        let machine = crate::toolchain::testing::TestMachine::new();
        let host = real_toolchain_host(&machine);
        let dir = tempfile::tempdir().expect("temp dir");
        let create = |root: &Path| {
            smol::block_on(Project::create(
                &host,
                root,
                CreateOptions {
                    name: "Water Example".to_string(),
                    bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                        .expect("bundle identifier"),
                    waterui_path: None,
                    channel: None,
                    framework_manifest: None,
                    framework: Some(crate::framework::test_fixtures::stable_framework()),
                    framework_lock: None,
                    author: "Lexo Liu".to_string(),
                    web: None,
                },
            ))
            .expect("project creation must succeed")
        };
        let first = create(&dir.path().join("one/demo"));
        let second = create(&dir.path().join("two/demo"));

        let staged = |project: &Project| {
            crate::platforming::packaging::dist_dir(
                &project.backend_path::<crate::hydrolysis::backend::HydrolysisBackend>(),
                "linux",
                Some("release"),
            )
            .join(project.hydrolysis_binary_name().as_str())
        };
        let first_staged = staged(&first);
        let second_staged = staged(&second);

        assert_ne!(
            first_staged, second_staged,
            "same-named projects must not stage the same shipped path"
        );
        for (project, staged) in [(&first, &first_staged), (&second, &second_staged)] {
            assert!(
                staged.starts_with(
                    project.backend_path::<crate::hydrolysis::backend::HydrolysisBackend>()
                ),
                "{} must live under the project's own managed backend directory",
                staged.display()
            );
        }
    }
}

#[cfg(test)]
mod local_patch_tests {
    use std::path::{Path, PathBuf};

    use super::{BundleIdentifier, CreateOptions, FailToCreateProject, Project, ProjectDraft};
    use crate::{framework::test_fixtures::write_local_checkout, toolchain::testing::TestMachine};

    const WATER_TOML: &str = "waterui_path = \"../waterui\"\n\n[package]\nname = \"App\"\nbundle_identifier = \"dev.waterui.app\"\n";

    /// A checkout beside a project directory `app`, returning the project.
    fn checkout_and_project(directory: &Path) -> std::path::PathBuf {
        let checkout = directory.join("waterui");
        std::fs::create_dir_all(&checkout).expect("checkout dir");
        std::fs::write(
            checkout.join("Cargo.toml"),
            include_str!("../../tests/fixtures/local_checkout_patches.toml"),
        )
        .expect("checkout manifest");
        let app = directory.join("app");
        std::fs::create_dir_all(&app).expect("project dir");
        app
    }

    /// A project on a `waterui_path` keeps the checkout's entries in its
    /// `[patch]` tables current every time it opens — the entries recorded as
    /// copied earlier are replaced, so a dropped one disappears and a moved one
    /// follows — while the project's own entries stay as written, and a
    /// project already in line is left byte-for-byte alone.
    #[test]
    fn a_local_checkout_project_follows_the_checkout_and_keeps_its_own_patches() {
        let directory = tempfile::tempdir().expect("temp dir");
        let app = checkout_and_project(directory.path());
        let cargo_path = app.join("Cargo.toml");
        std::fs::write(
            &cargo_path,
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nwaterui = { path = \"../waterui\" }\n\n[patch.crates-io]\nwaterui-core = { path = \"../elsewhere/core\" }\nstale = { path = \"../elsewhere/stale\" }\nnami-derive = { git = \"https://github.com/water-rs/nami\", rev = \"1ffe641ad7e18d341a7cb6d8bcb326d76fb7cf03\" }\n",
        )
        .expect("project manifest");
        let water_path = app.join("Water.toml");
        std::fs::write(
            &water_path,
            format!(
                "{WATER_TOML}\n[waterui_patches.crates-io.waterui-core]\npath = \"../elsewhere/core\"\n\n[waterui_patches.crates-io.stale]\npath = \"../elsewhere/stale\"\n"
            ),
        )
        .expect("water manifest");
        let written = super::Manifest::open(&water_path);
        let written = smol::block_on(written)
            .expect("water manifest parses")
            .waterui_patches;

        smol::block_on(Project::refresh_local_patches(
            &app,
            Path::new("../waterui"),
            &written,
        ))
        .expect("tables refresh");
        let refreshed = std::fs::read_to_string(&cargo_path).expect("refreshed manifest");
        let manifest = cargo_toml::Manifest::from_str(&refreshed).expect("manifest parses");
        let crates_io = &manifest.patch["crates-io"];
        let cargo_toml::Dependency::Detailed(core) = &crates_io["waterui-core"] else {
            panic!("the core patch is a path dependency");
        };
        assert_eq!(core.path.as_deref(), Some("../waterui/core"));
        assert!(!crates_io.contains_key("stale"));
        assert!(crates_io.contains_key("vello"));
        assert!(
            refreshed.contains(
                "nami-derive = { git = \"https://github.com/water-rs/nami\", rev = \"1ffe641ad7e18d341a7cb6d8bcb326d76fb7cf03\" }"
            ),
            "the project's own entry keeps its spelling:\n{refreshed}"
        );
        assert!(refreshed.starts_with("[package]"));
        let recorded = smol::block_on(super::Manifest::open(&water_path))
            .expect("water manifest parses")
            .waterui_patches;
        assert_eq!(
            recorded,
            crate::templates::local_framework_patches(&app, Path::new("../waterui"))
                .expect("checkout tables"),
            "Water.toml records exactly the checkout's copy"
        );
        let recorded_text = std::fs::read_to_string(&water_path).expect("water manifest");
        assert!(
            recorded_text.starts_with(WATER_TOML)
                && recorded_text.contains("\n[waterui_patches.crates-io.waterui-core]\npath = "),
            "the record is spliced in as one table per entry:\n{recorded_text}"
        );

        smol::block_on(Project::refresh_local_patches(
            &app,
            Path::new("../waterui"),
            &recorded,
        ))
        .expect("second refresh");
        assert_eq!(
            std::fs::read_to_string(&cargo_path).expect("manifest after the second refresh"),
            refreshed
        );
        assert_eq!(
            std::fs::read_to_string(&water_path).expect("water manifest after the second refresh"),
            recorded_text
        );
    }

    /// A project entry for a crate the checkout patches to somewhere else
    /// overrides the checkout's: it stays as written, the checkout's entry is
    /// not copied beside it nor recorded, and dropping the override later
    /// brings the checkout's entry back.
    #[test]
    fn a_project_entry_overrides_the_checkouts_for_the_same_crate() {
        let directory = tempfile::tempdir().expect("temp dir");
        let app = checkout_and_project(directory.path());
        let cargo_path = app.join("Cargo.toml");
        let header = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n";
        let manifest = format!(
            "{header}\n[patch.crates-io]\nvello = {{ git = \"https://github.com/linebender/vello\", rev = \"project-rev\" }}\n"
        );
        std::fs::write(&cargo_path, &manifest).expect("project manifest");
        let water_path = app.join("Water.toml");
        std::fs::write(&water_path, WATER_TOML).expect("water manifest");

        smol::block_on(Project::refresh_local_patches(
            &app,
            Path::new("../waterui"),
            &cargo_toml::PatchSet::new(),
        ))
        .expect("an override refreshes");

        let refreshed = std::fs::read_to_string(&cargo_path).expect("refreshed manifest");
        let crates_io = &cargo_toml::Manifest::from_str(&refreshed)
            .expect("manifest parses")
            .patch["crates-io"];
        let cargo_toml::Dependency::Detailed(vello) = &crates_io["vello"] else {
            panic!("the vello patch is a git dependency");
        };
        assert_eq!(vello.rev.as_deref(), Some("project-rev"));
        assert!(crates_io.contains_key("waterui-core"), "the rest is copied");
        let recorded = smol::block_on(super::Manifest::open(&water_path))
            .expect("water manifest parses")
            .waterui_patches;
        assert!(
            !recorded["crates-io"].contains_key("vello"),
            "the overridden entry was not copied, so it is not recorded"
        );

        // Dropping the override hands the crate back to the checkout.
        let mut document: toml_edit::DocumentMut = refreshed.parse().expect("manifest parses");
        document["patch"]["crates-io"]
            .as_table_like_mut()
            .expect("crates-io table")
            .remove("vello");
        std::fs::write(&cargo_path, document.to_string()).expect("override dropped");
        smol::block_on(Project::refresh_local_patches(
            &app,
            Path::new("../waterui"),
            &recorded,
        ))
        .expect("second refresh");
        let crates_io = cargo_toml::Manifest::from_path(&cargo_path)
            .expect("manifest parses")
            .patch
            .remove("crates-io")
            .expect("crates-io table");
        let cargo_toml::Dependency::Detailed(vello) = &crates_io["vello"] else {
            panic!("the vello patch is a git dependency");
        };
        assert_eq!(
            vello.git.as_deref(),
            Some("https://github.com/lexoliu/vello")
        );
    }

    /// A project inside the checkout's own workspace — every example in this
    /// repository — is already governed by the checkout's tables, so nothing is
    /// copied into the member manifest, where Cargo would ignore it and warn on
    /// every build.
    #[test]
    fn a_member_of_the_checkouts_workspace_keeps_its_manifest() {
        let directory = tempfile::tempdir().expect("temp dir");
        let checkout = directory.path().join("waterui");
        let app = checkout.join("examples/app");
        std::fs::create_dir_all(&app).expect("project dir");
        std::fs::write(
            checkout.join("Cargo.toml"),
            include_str!("../../tests/fixtures/local_checkout_patches.toml"),
        )
        .expect("checkout manifest");
        let manifest = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nwaterui = { path = \"../..\" }\n";
        let cargo_path = app.join("Cargo.toml");
        std::fs::write(&cargo_path, manifest).expect("project manifest");

        smol::block_on(Project::refresh_local_patches(
            &app,
            Path::new("../.."),
            &cargo_toml::PatchSet::new(),
        ))
        .expect("tables refresh");

        assert_eq!(
            std::fs::read_to_string(&cargo_path).expect("manifest after the refresh"),
            manifest
        );
    }

    /// A project inside someone else's workspace cannot carry the tables at all:
    /// Cargo reads them from that workspace root. Saying which manifest they
    /// belong in beats writing a copy that is read by nobody.
    #[test]
    fn a_member_of_a_foreign_workspace_is_told_where_the_tables_belong() {
        let directory = tempfile::tempdir().expect("temp dir");
        let checkout = directory.path().join("waterui");
        std::fs::create_dir_all(&checkout).expect("checkout dir");
        std::fs::write(
            checkout.join("Cargo.toml"),
            include_str!("../../tests/fixtures/local_checkout_patches.toml"),
        )
        .expect("checkout manifest");
        let workspace = directory.path().join("their-workspace");
        let app = workspace.join("app");
        std::fs::create_dir_all(&app).expect("project dir");
        std::fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\n",
        )
        .expect("workspace manifest");
        let manifest = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nwaterui = { path = \"../../waterui\" }\n";
        let cargo_path = app.join("Cargo.toml");
        std::fs::write(&cargo_path, manifest).expect("project manifest");

        let error = smol::block_on(Project::refresh_local_patches(
            &app,
            Path::new("../../waterui"),
            &cargo_toml::PatchSet::new(),
        ))
        .expect_err("a copy here would be ignored");

        let message = error.to_string();
        assert!(message.contains("their-workspace"), "{message}");
        assert_eq!(
            std::fs::read_to_string(&cargo_path).expect("manifest after the refusal"),
            manifest
        );
    }

    fn local_create_options(waterui_path: PathBuf) -> CreateOptions {
        CreateOptions {
            name: "Water Example".to_string(),
            bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                .expect("bundle identifier"),
            waterui_path: Some(waterui_path),
            channel: None,
            framework_manifest: None,
            framework: None,
            framework_lock: None,
            author: "water test".to_string(),
            web: None,
        }
    }

    fn local_checkout(machine: &TestMachine) -> PathBuf {
        let checkout = machine.root().join("waterui");
        write_local_checkout(&checkout);
        checkout
    }

    #[test]
    fn a_failing_ffi_scaffold_fails_create_and_leaves_nothing_behind() {
        let machine = TestMachine::new();
        machine.install("cargo");
        machine.install("git");
        let host = machine.host::<&str, &str>([]);
        let checkout = local_checkout(&machine);
        let root = machine.root().join("water-example");

        let draft = smol::block_on(ProjectDraft::create(
            &host,
            &root,
            local_create_options(checkout),
        ))
        .expect("the initial project scaffold succeeds");
        assert!(root.join("Water.toml").is_file());
        let cache = crate::water_dir::build_cache_container_for_on(&host, &root)
            .expect("cache container path");
        assert!(
            cache.starts_with(machine.home()),
            "managed cache must use the test host's home: {}",
            cache.display()
        );

        let error = smol::block_on(draft.finish()).expect_err("the FFI scaffold must fail");
        assert!(
            matches!(error, FailToCreateProject::ScaffoldGeneratedCrates(_)),
            "{error}"
        );
        let message = error.to_string();
        assert!(
            message.contains("Failed to scaffold the project's generated crates"),
            "{message}"
        );
        assert!(
            message.contains("could not scaffold the Apple/Android FFI companion crate"),
            "{message}"
        );
        assert!(
            message.contains("`cargo metadata` exited with an error"),
            "{message}"
        );
        assert!(
            !root.exists(),
            "failed project root remains: {}",
            root.display()
        );
        assert!(
            !cache.exists(),
            "failed project cache remains: {}",
            cache.display()
        );
    }

    #[test]
    fn create_removes_what_it_wrote_when_a_later_step_fails() {
        let machine = TestMachine::new();
        machine.install("cargo");
        machine.install("git");
        let host = machine.host::<&str, &str>([]);
        let checkout = local_checkout(&machine);
        let root = machine.root().join("water-example");
        std::fs::write(machine.home().join(".water"), "not a directory")
            .expect("block the Water home directory");

        let error = smol::block_on(ProjectDraft::create(
            &host,
            &root,
            local_create_options(checkout),
        ))
        .expect_err("the build-cache setup must fail");
        assert!(
            matches!(error, FailToCreateProject::BuildCache(_)),
            "{error}"
        );
        assert!(
            !root.exists(),
            "partial project root remains: {}",
            root.display()
        );
    }

    #[test]
    fn discarding_a_draft_removes_the_project_and_returns_the_error() {
        let machine = TestMachine::new();
        machine.install("cargo");
        machine.install("git");
        let host = machine.host::<&str, &str>([]);
        let checkout = local_checkout(&machine);
        let root = machine.root().join("water-example");

        let draft = smol::block_on(ProjectDraft::create(
            &host,
            &root,
            local_create_options(checkout),
        ))
        .expect("project draft creation");
        let cache = crate::water_dir::build_cache_container_for_on(&host, &root)
            .expect("cache container path");
        let error = smol::block_on(draft.discard(eyre::eyre!("frontend failed")));
        assert_eq!(error.to_string(), "frontend failed");
        assert!(
            !root.exists(),
            "discarded project root remains: {}",
            root.display()
        );
        assert!(
            !cache.exists(),
            "discarded project cache remains: {}",
            cache.display()
        );
    }
}

#[cfg(test)]
mod embedded_declaration_tests {
    use super::Manifest;

    fn parse(toml: &str) -> Manifest {
        toml::from_str(toml).expect("manifest parses")
    }

    /// `[package] embedded = true` declares the crate a library a host app
    /// embeds (water-rs/cli#223); absent the key the crate owns the app
    /// entry as before.
    #[test]
    fn embedded_defaults_to_the_entry_owning_mode() {
        let manifest = parse(
            r#"
                [package]
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"
            "#,
        );
        assert!(!manifest.package.embedded);

        let embedded = parse(
            r#"
                [package]
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"
                embedded = true
            "#,
        );
        assert!(embedded.package.embedded);
    }

    /// `embedded` serializes back out only when set — `Water.toml` stays
    /// quiet about the default.
    #[test]
    fn embedded_serializes_only_when_true() {
        let manifest = parse(
            r#"
                [package]
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"
            "#,
        );
        assert!(
            !toml::to_string(&manifest.package)
                .expect("package serializes")
                .contains("embedded")
        );

        let embedded = parse(
            r#"
                [package]
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"
                embedded = true
            "#,
        );
        assert!(
            toml::to_string(&embedded.package)
                .expect("package serializes")
                .contains("embedded = true")
        );
    }
}

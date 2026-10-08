//! Build system

mod native_links;
pub(crate) use native_links::NativeLink;

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::Stdio,
};

use eyre::{Context as _, bail};
use futures_util::StreamExt as _;
use smol::{io::AsyncReadExt as _, process::Command, unblock};
use target_lexicon::{Environment, OperatingSystem, Triple};
use tracing::{info, warn};

use crate::project::Project;
use crate::utils::{run_command, std_output_enabled};

/// Get the dynamic library extension for a target triple.
#[must_use]
pub const fn lib_extension_for_triple(triple: &Triple) -> &'static str {
    match triple.operating_system {
        OperatingSystem::Darwin(_)
        | OperatingSystem::MacOSX { .. }
        | OperatingSystem::IOS(_)
        | OperatingSystem::TvOS(_)
        | OperatingSystem::WatchOS(_)
        | OperatingSystem::VisionOS(_) => "dylib",
        OperatingSystem::Windows => "dll",
        // Linux, Android, and most other Unix-like targets use .so.
        _ => "so",
    }
}

/// Whether `triple` targets an Apple platform — the set whose `-l` link flag
/// and `@rpath` install-name conventions [`RustDynamicLibraries::resolve`]
/// stages for.
const fn is_apple_triple(triple: &Triple) -> bool {
    matches!(
        triple.operating_system,
        OperatingSystem::Darwin(_)
            | OperatingSystem::MacOSX { .. }
            | OperatingSystem::IOS(_)
            | OperatingSystem::TvOS(_)
            | OperatingSystem::WatchOS(_)
            | OperatingSystem::VisionOS(_)
    )
}

/// The rustup toolchain a project's builds run under: the one its own
/// directory selects, whatever directory the generated crate compiles in.
///
/// # Errors
/// Returns an error when rustup resolves no toolchain for the project.
pub async fn project_toolchain(project: &Project) -> eyre::Result<String> {
    Ok(crate::toolchain::rust::project_rustup_toolchain(project.root()).await?)
}

/// Resolve the Rust standard-library directory for a target triple under
/// `toolchain`, the rustup toolchain the libraries were built with.
///
/// # Errors
/// Returns an error if rustc cannot resolve an existing target library directory.
pub async fn rust_target_libdir(triple: &Triple, toolchain: &str) -> eyre::Result<PathBuf> {
    let target = triple.to_string();
    let host = crate::toolchain::Host::current().with_env("RUSTUP_TOOLCHAIN", toolchain);
    let output = host
        .run(
            "rustc",
            ["--print", "target-libdir", "--target", target.as_str()],
        )
        .await?;
    let libdir = output.trim();
    if libdir.is_empty() {
        bail!("`rustc --print target-libdir --target {target}` returned an empty path");
    }
    let path = PathBuf::from(libdir);
    if !path.is_dir() {
        bail!(
            "Rust target libdir does not exist for dynamic linking: {}",
            path.display()
        );
    }
    Ok(path)
}

/// The Cargo target a build selects.
///
/// A crate-type override only has meaning for the library target, so carrying the
/// target kind in the type keeps `cargo rustc --crate-type` from ever reaching a
/// binary build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CargoTarget<'a> {
    /// The crate's library target.
    Lib,
    /// One named binary target.
    Binary(&'a str),
}

impl CargoTarget<'_> {
    /// The cargo-level args selecting this target, plus the `--crate-type`
    /// unit selector when a lib override is set. The flag must reach cargo
    /// itself — passed after `--` it would land on rustc, where
    /// `--crate-type` accumulates and the manifest's other crate types would
    /// still emit and link.
    fn cargo_args(self, crate_type_override: Option<&str>) -> Vec<String> {
        let mut args: Vec<String> = match self {
            Self::Lib => vec!["--lib".to_owned()],
            Self::Binary(name) => vec!["--bin".to_owned(), name.to_owned()],
        };
        if let Some(crate_type) = crate_type_override {
            args.push("--crate-type".to_owned());
            args.push(crate_type.to_owned());
        }
        args
    }

    const fn accepts_crate_type_override(self) -> bool {
        matches!(self, Self::Lib)
    }

    /// Whether a `compiler-artifact` message's target is the one this build
    /// selected.
    fn matches(&self, target: &cargo_metadata::Target) -> bool {
        use cargo_metadata::TargetKind;
        match self {
            Self::Binary(name) => {
                target.name.as_str() == *name && target.kind.contains(&TargetKind::Bin)
            }
            Self::Lib => target.kind.iter().any(|kind| {
                matches!(
                    kind,
                    TargetKind::Lib
                        | TargetKind::RLib
                        | TargetKind::DyLib
                        | TargetKind::CDyLib
                        | TargetKind::StaticLib
                        | TargetKind::ProcMacro
                )
            }),
        }
    }
}

/// The outcome of one Cargo invocation: the profile directory everything
/// landed under and the artifact Cargo reported for the selected target.
#[derive(Debug)]
pub struct BuiltTarget {
    /// `<target>/<triple>/<profile>` — dependency artifacts and staged
    /// runtime libraries resolve from this directory.
    pub profile_dir: PathBuf,
    /// The final artifact Cargo reported writing for the selected target —
    /// its own `compiler-artifact` message, not a name reconstructed under
    /// the profile root. For a `--bin` build this is the variant-stable
    /// `deps/<name>-<marker>` hard link of the reported `executable`, made
    /// while the binary artifact lock was held.
    pub artifact: PathBuf,
    /// The `waterui-dylib` dynamic library Cargo reported, when this build
    /// produced one.
    pub shared_runtime: Option<PathBuf>,
    /// The library artifact this build's dependency graph produced for the
    /// project crate — the `deps/` rlib, staticlib, or dylib whose symbol
    /// table carries the `waterui_meta_*` statics and `waterui_preview_*`
    /// exports `crate::artifact_symbols::ArtifactSymbols` reads. `None` when
    /// no project is attached or Cargo reported no library unit for it.
    pub app_library: Option<PathBuf>,
    /// The `executable` Cargo's `compiler-artifact` message reported for the
    /// selected binary target — the unhashed `<profile>/<name>` uplift.
    /// `None` when this build selected a library target, which emits no
    /// executable. Launchers execute this path, never the `deps/` artifact:
    /// the staged shared runtime sits beside the uplift in the profile
    /// directory, so `@executable_path`/`$ORIGIN` resolves it.
    pub executable: Option<SharedExecutable>,
    /// The entry `[[bin]]` an Apple build produces beside the companion
    /// library — `Some` only on a non-embedded Apple build. Its own
    /// `BuiltTarget` carries the reported `artifact` — the marked
    /// `deps/<name>-<marker>` link — which packaging reads, so it never
    /// reconstructs `<profile>/<name>` itself.
    pub entry_binary: Option<Box<Self>>,
    /// The CEF helper `[[bin]]` the generated hydrolysis backend crate
    /// builds next to the application binary — `Some` only on a hydrolysis
    /// build whose project declared one. Its own `BuiltTarget` carries the
    /// helper's `artifact` path, so packaging never reconstructs it from
    /// this target's artifact directory.
    pub cef_helper: Option<Box<Self>>,
}

impl BuiltTarget {
    /// The `executable` Cargo reported for this build's binary target — the
    /// `<profile>/<name>` uplift a launcher executes so the staged shared
    /// runtime resolves beside it.
    ///
    /// How long the uplift keeps this run's bytes depends on the artifact
    /// lock this value still holds — see [`SharedExecutable`] and
    /// [`RustBuild::with_artifact_lock_scope`].
    ///
    /// # Errors
    /// Returns an error when this build selected no binary target or Cargo
    /// reported no executable for it.
    pub fn executable(&self) -> eyre::Result<&Path> {
        self.executable
            .as_ref()
            .map(SharedExecutable::path)
            .ok_or_else(|| {
                eyre::eyre!(
                    "Cargo reported no executable for the build in {}",
                    self.profile_dir.display()
                )
            })
    }

    /// Return the shared `WaterUI` runtime Cargo reported for this build.
    ///
    /// # Errors
    /// Returns an error when this build did not produce a shared runtime.
    pub fn shared_runtime(&self) -> eyre::Result<&Path> {
        self.shared_runtime.as_deref().ok_or_else(|| {
            eyre::eyre!(
                "Cargo reported no `waterui-dylib` dynamic library for the build in {}; the shared WaterUI runtime was not built",
                self.profile_dir.display()
            )
        })
    }

    /// Symbols of this build's project-crate library artifact.
    ///
    /// The artifact is the one the build just emitted, so the symbols are
    /// ground truth for what compiled: macro-emitted `waterui_meta_*` statics
    /// and `waterui_preview_*` exports enumerate here. A build with no project
    /// attached — a fixture or a support crate — reads empty.
    ///
    /// # Errors
    /// Returns an error when the reported artifact cannot be read or parsed.
    pub fn app_symbols(&self) -> eyre::Result<crate::artifact_symbols::ArtifactSymbols> {
        self.app_library.as_ref().map_or_else(
            || Ok(crate::artifact_symbols::ArtifactSymbols::empty()),
            |library| crate::artifact_symbols::ArtifactSymbols::read(library),
        )
    }
}

/// The `<profile>/<name>` uplift a `--bin` build's `executable` report
/// names, carried together with the artifact lock that bounds how long the
/// shared slot keeps this build's bytes.
///
/// Keeping the reported path cannot keep the lock by accident: `lock` is
/// `Some` only while the launcher that executes this path may still spawn
/// or `exec` it — the fd closes on `exec`, releasing the lock exactly when
/// the new image loads. A build nothing launches through the shared uplift
/// — the Apple entry `[[bin]]`, the CEF helper — reports the path with
/// `lock: None`, so a `BuiltTarget` held for packaging or for a whole run
/// session blocks no other project's same-named build.
#[derive(Debug)]
pub struct SharedExecutable {
    path: PathBuf,
    _lock: Option<ArtifactLock>,
}

impl SharedExecutable {
    /// An `executable` report carrying no artifact lock — the bundled
    /// executable and ESP32 firmware paths platform packaging hands out
    /// itself, which no same-named build can re-uplift under it.
    #[must_use]
    pub const fn unlocked(path: PathBuf) -> Self {
        Self { path, _lock: None }
    }

    /// The `<profile>/<name>` uplift path itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The guard holding fs4's exclusive lock on a `<profile>/<name>` uplift
/// slot: while it lives, a same-named build cannot re-uplift the shared
/// path a launcher is about to exec.
///
/// The fd is closed-on-exec, so an `exec`ing launcher releases the lock in
/// the kernel's image swap; elsewhere dropping the guard closes it — which
/// is why the guard lives inside [`SharedExecutable`] rather than as a bare
/// `File` a caller could keep past the launch.
#[derive(Debug)]
pub struct ArtifactLock {
    _file: std::fs::File,
}

/// How long `build_inner` keeps a `--bin` build's artifact lock, resolved
/// against which path the binary is launched through.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArtifactLockScope {
    /// Until the launched image is mapped: the returned [`BuiltTarget`]
    /// carries the lock inside its `executable`, so the shared
    /// `<profile>/<name>` uplift keeps this build's bytes until the
    /// launcher's child is spawned — or the `exec` happened. The scope for
    /// every binary the CLI execs in place: the TUI launcher, the
    /// Hydrolysis and Apple previews, the MCP proxy.
    #[default]
    UntilLaunch,
    /// Until the `deps/<name>-<marker>` link exists: the lock is released
    /// as `build_inner` ends, for binaries nothing execs through the shared
    /// uplift — the Apple entry `[[bin]]` and the CEF helper, whose
    /// packaging reads the marked `deps/` artifact. Their lock file lives
    /// in the shared profile directory and is the same for every project;
    /// a `water run` holding it across packaging and launch would block a
    /// `water build` of another same-named binary until the run exits.
    UntilMarked,
}

/// Selects how Rust dependencies are linked into a native application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustLinkage {
    /// Link the `WaterUI` runtime into the application archive.
    Static,
    /// Link the application and loadable modules against one shared `WaterUI` runtime.
    SharedRuntime,
}

/// Configure a Cargo invocation that compiles one of `WaterUI`'s generated crates.
///
/// Incremental compilation is off for every one of these builds, unconditionally.
/// `-C incremental` is part of a unit's profile, the profile feeds Cargo's `-C metadata`,
/// and `-C metadata` is mangled into every symbol name. Two builds in the same flow that
/// disagree about incremental therefore produce runtimes whose symbols cannot resolve
/// against each other: a preview support app built one way and a preview module built the
/// other share a `libwaterui_dylib.dylib` filename and roughly 33,000 mismatched symbols,
/// and the module fails to `dlopen` on a missing generic instantiation.
///
/// The choice is unconditional precisely so it cannot depend on an environmental accident
/// such as whether a machine has `sccache` installed. Little is given up: every generated
/// backend builds into one shared target directory where Cargo already reuses each unit's
/// compiled artifact across backends and feature variants — while an `sccache` entry,
/// which requires incremental to be off, covers what that sharing cannot.
pub fn configure_generated_crate_compilation(command: &mut Command) {
    command.env("CARGO_INCREMENTAL", "0");
}

/// Prepend the managed tool directories (`~/.water/tools/<name>/<version>`)
/// to the build's `PATH` so build scripts resolve a pinned `dxc`, JDK, and
/// friends by name — the user never edits `PATH`. A no-op when nothing is
/// installed (or the paths cannot join), so ambient `PATH` passes through.
fn with_managed_tools_path(command: &mut Command) {
    if let Some((key, value)) =
        crate::toolchain::managed_tool::managed_tools_path_env(&crate::toolchain::Host::current())
    {
        command.env(key, value);
    }
}

/// A shared Rust library staged with an artifact: the build's own output to
/// copy, and the file name the copy must carry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedDynamicLibrary {
    /// The file this build produced.
    source: PathBuf,
    /// The name the staged copy carries.
    staged_name: OsString,
}

impl StagedDynamicLibrary {
    /// Stage `source` under the file name it already has — the behavior for a
    /// library the artifact records no dynamic dependency on.
    fn reported(source: PathBuf) -> eyre::Result<Self> {
        let staged_name = source.file_name().map(ToOwned::to_owned).ok_or_else(|| {
            eyre::eyre!(
                "Dynamic library path has no file name: {}",
                source.display()
            )
        })?;
        Ok(Self {
            source,
            staged_name,
        })
    }

    /// Stage `source` under `needed_name` — the basename of the dynamic
    /// dependency the packaged artifact records.
    fn needed(needed_name: &str, source: PathBuf) -> Self {
        Self {
            source,
            staged_name: OsString::from(needed_file_name(needed_name)),
        }
    }
}

/// Dynamic Rust libraries required by a shared-runtime development build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustDynamicLibraries {
    waterui: StagedDynamicLibrary,
    standard_library: StagedDynamicLibrary,
    triple: Triple,
}

impl RustDynamicLibraries {
    /// Resolve the shared `WaterUI` runtime and target Rust standard library.
    ///
    /// Each library is staged under the name the artifact's own dynamic
    /// section records: a `dylib` unit from a git or registry source compiles
    /// as `deps/libwaterui_dylib-<metadata>.so`, and `DT_NEEDED` (the PE
    /// import descriptor / `LC_LOAD_DYLIB` on the other platforms) records
    /// that hashed name — not the unhashed `<profile>/libwaterui_dylib.so`
    /// alias Cargo uplifts and its artifact report names. The loader only
    /// ever resolves the recorded name, so a needed library that is not found
    /// under it is a packaging error naming what was searched for and where
    /// (water-rs/cli#184). An artifact that records no matching dependency —
    /// a statically linked runtime — keeps the reported path's own name.
    ///
    /// The prebuilt `libstd` comes from the toolchain `project` selects — the
    /// one [`RustBuild`] compiled the runtime under — so the runtime and the
    /// shipped standard library agree; a runtime built under one toolchain
    /// and shipped with another's `libstd` fails at launch on the missing
    /// library hash.
    ///
    /// # Errors
    /// Returns an error when a needed dynamic library is absent, ambiguous, or
    /// the artifact's dynamic dependencies cannot be read.
    pub async fn resolve(
        built: &BuiltTarget,
        triple: &Triple,
        project: &Project,
    ) -> eyre::Result<Self> {
        let reported = built.shared_runtime()?.to_path_buf();
        let lib_dir = &built.profile_dir;
        let deps_dir = lib_dir.join("deps");
        let needed = unblock({
            let artifact = built.artifact.clone();
            move || needed_shared_libraries(&artifact)
        })
        .await
        .map_err(|error| {
            eyre::eyre!(
                "failed to read the dynamic dependencies of {}: {error}",
                built.artifact.display()
            )
        })?;

        let waterui = match needed
            .iter()
            .find(|name| needed_library_matches(name, "waterui_dylib"))
        {
            Some(name) => {
                let source = needed_library_source(
                    name,
                    &[deps_dir.clone(), lib_dir.clone()],
                    &built.artifact,
                )?;
                StagedDynamicLibrary::needed(name, source)
            }
            None => StagedDynamicLibrary::reported(reported)?,
        };

        // A `-Zbuild-std` build publishes its freshly compiled `libstd` into
        // the profile's `deps/` directory via the rustc wrapper; that copy —
        // not the toolchain's prebuilt one — is what the build linked against,
        // so it is the one that has to ship. The needed-name lookups below
        // search the same two directories in that order; the prefix scan is
        // the fallback for an artifact that records no `libstd` at all (a
        // Mach-O binary, whose `libstd` dependency is the runtime dylib's own).
        let needed_std = needed.iter().find(|name| is_rust_standard_library(name));
        let standard_library = match needed_std {
            Some(name) if deps_dir.join(needed_file_name(name)).is_file() => {
                StagedDynamicLibrary::needed(name, deps_dir.join(needed_file_name(name)))
            }
            Some(name) => {
                let toolchain = project_toolchain(project).await?;
                let target_libdir = rust_target_libdir(triple, &toolchain).await?;
                StagedDynamicLibrary::needed(
                    name,
                    needed_library_source(
                        name,
                        &[deps_dir.clone(), target_libdir],
                        &built.artifact,
                    )?,
                )
            }
            None => {
                let resolution_triple = triple.clone();
                let staged = unblock(move || {
                    resolve_rust_standard_library_in(&deps_dir, &resolution_triple)
                })
                .await;
                match staged {
                    Ok(path) => StagedDynamicLibrary::reported(path)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let toolchain = project_toolchain(project).await?;
                        let target_libdir = rust_target_libdir(triple, &toolchain).await?;
                        let resolution_triple = triple.clone();
                        let path = unblock(move || {
                            resolve_rust_standard_library_in(&target_libdir, &resolution_triple)
                        })
                        .await?;
                        StagedDynamicLibrary::reported(path)?
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        };

        Ok(Self {
            waterui,
            standard_library,
            triple: triple.clone(),
        })
    }

    /// Shared `WaterUI` runtime path.
    #[must_use]
    pub fn waterui(&self) -> &Path {
        &self.waterui.source
    }

    /// The file name the artifact's dynamic section records for the
    /// shared `WaterUI` runtime — `libwaterui_dylib-<metadata>.dylib` for
    /// a dynamically linked build, the reported path's own name when the
    /// artifact records no such dependency.
    #[must_use]
    pub fn waterui_staged_name(&self) -> &OsStr {
        &self.waterui.staged_name
    }

    /// The path the canonical `libwaterui_dylib.dylib` occupies once
    /// [`stage_apple_canonical`](Self::stage_apple_canonical) copies it into
    /// `destination` — the file the `@rpath/libwaterui_dylib.dylib` install
    /// name resolves to. Apple-only:
    /// the reported artifact path may be a hashed `deps/` name or already
    /// the canonical unhashed one, and staging must honor both (water-rs/cli#197,
    /// water-rs/cli#291). On other triples the staged copy already carries
    /// the recorded name, so this is its own path.
    #[must_use]
    pub fn apple_canonical_waterui(&self, destination: &Path) -> PathBuf {
        if is_apple_triple(&self.triple) {
            destination.join("libwaterui_dylib.dylib")
        } else {
            destination.join(&self.waterui.staged_name)
        }
    }

    /// Copy the resolved `waterui_dylib` into `destination` under the
    /// canonical Apple name [`apple_canonical_waterui`](Self::apple_canonical_waterui)
    /// returns, alongside
    /// the recorded name [`stage`](Self::stage) writes.
    ///
    /// The reported artifact can already occupy the canonical destination —
    /// an unhashed `deps/` dylib name is already the canonical one — and
    /// then staging is a no-op, never a copy: `copy_file` deletes its
    /// destination before reflinking, so copying the reported path over
    /// itself would destroy the runtime it stages.
    ///
    /// # Errors
    /// Returns an error if the copy fails, or if the reported runtime
    /// cannot be validated at the canonical destination.
    pub async fn stage_apple_canonical(&self, destination: &Path) -> eyre::Result<PathBuf> {
        let staged = self.apple_canonical_waterui(destination);
        if self.waterui.source == staged {
            smol::fs::metadata(&self.waterui.source)
                .await
                .wrap_err_with(|| {
                    format!(
                        "Failed to validate the reported shared WaterUI runtime {}",
                        self.waterui.source.display()
                    )
                })?;
            return Ok(staged);
        }
        crate::utils::copy_file_if_changed(&self.waterui.source, &staged)
            .await
            .wrap_err_with(|| {
                format!(
                    "Failed to stage {} to {}",
                    self.waterui.source.display(),
                    staged.display()
                )
            })?;
        Ok(staged)
    }

    /// Target Rust standard-library dynamic library path.
    #[must_use]
    pub fn standard_library(&self) -> &Path {
        &self.standard_library.source
    }

    /// Iterate over every library that must be staged with the application.
    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        [&self.waterui, &self.standard_library]
            .into_iter()
            .map(|library| library.source.as_path())
    }

    /// Copy all required dynamic libraries into a runtime search directory.
    ///
    /// Staging goes through the reflinking copy so a shared runtime that every build
    /// output needs a copy of costs one set of extents instead of one full copy per
    /// destination. A copy-on-write clone is also the only sharing that is safe here:
    /// these staged libraries are rewritten in place later (`install_name_tool`), so
    /// hard links would corrupt the Cargo artifact they were linked to.
    ///
    /// # Errors
    /// Returns an error when the destination cannot be created or a library cannot be copied.
    pub async fn stage(&self, destination: &Path) -> eyre::Result<()> {
        smol::fs::create_dir_all(destination).await?;
        // A resolved source can already live inside the destination — the
        // hashed `deps/` dylib staged beside a binary that lives there too —
        // so the staged-copy cleanup must leave sources alone and the copy
        // must not rewrite a library over itself.
        let libraries = [&self.waterui, &self.standard_library];
        let sources: Vec<PathBuf> = libraries
            .iter()
            .map(|library| library.source.clone())
            .collect();
        Self::remove_staged_except(destination, &self.triple, &sources).await?;
        for library in &libraries {
            let staged = destination.join(&library.staged_name);
            if library.source == staged {
                continue;
            }
            crate::utils::copy_file_if_changed(&library.source, &staged)
                .await
                .wrap_err_with(|| {
                    format!(
                        "Failed to stage {} to {}",
                        library.source.display(),
                        staged.display()
                    )
                })?;
        }
        Ok(())
    }

    /// Remove shared-runtime libraries left by an earlier development build.
    ///
    /// # Errors
    /// Returns an error when the destination cannot be read or a matching library cannot be removed.
    pub async fn remove_staged(destination: &Path, triple: &Triple) -> eyre::Result<()> {
        Self::remove_staged_except(destination, triple, &[]).await
    }

    /// `keep` holds library paths that must survive: when a resolved source
    /// already lives in `destination`, deleting it would remove the very
    /// library being staged.
    async fn remove_staged_except(
        destination: &Path,
        triple: &Triple,
        keep: &[PathBuf],
    ) -> eyre::Result<()> {
        if !destination.is_dir() {
            return Ok(());
        }

        let extension = lib_extension_for_triple(triple);
        let standard_library_prefix = if triple.operating_system == OperatingSystem::Windows {
            "std-"
        } else {
            "libstd-"
        };
        let mut entries = smol::fs::read_dir(destination).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            if keep.contains(&entry.path()) {
                continue;
            }
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            let has_dynamic_extension =
                entry.path().extension().and_then(|value| value.to_str()) == Some(extension);
            // A Windows shared-runtime staging also ships the dxc runtime
            // pair wgpu's DirectX 12 backend `LoadLibrary`s; static packaging
            // clears them here so the dist directory is left clean.
            let is_shader_compiler = triple.operating_system == OperatingSystem::Windows
                && matches!(file_name.as_ref(), "dxcompiler.dll" | "dxil.dll");
            // `waterui_dylib` matches with or without a `-<metadata>` suffix
            // so a superseded hashed staging is removed with the unhashed one.
            if has_dynamic_extension
                && (is_waterui_dylib_file_name(&file_name)
                    || is_shader_compiler
                    || file_name.starts_with(standard_library_prefix))
            {
                smol::fs::remove_file(entry.path()).await?;
            }
        }
        Ok(())
    }
}

/// The shader-compiler runtime libraries a Windows binary `LoadLibrary`s by
/// name, resolved beside the `dxc` tool that ships them.
const DXC_RUNTIME_LIBRARIES: [&str; 2] = ["dxcompiler.dll", "dxil.dll"];

/// Resolve the `dxc` runtime pair to the copies installed beside the `dxc`
/// executable (on `PATH` or under the managed tool directory).
async fn resolve_dxc_runtime() -> eyre::Result<Vec<PathBuf>> {
    let host = crate::toolchain::Host::current();
    let dxc = crate::toolchain::dxc::Dxc
        .path(&host)
        .await
        .ok_or_else(|| {
            eyre::eyre!(
                "the dxc tool is not installed; run `water doctor` to install it, then build again"
            )
        })?;
    let dxc_dir = dxc.parent().ok_or_else(|| {
        eyre::eyre!(
            "the resolved dxc path {} has no parent directory",
            dxc.display()
        )
    })?;
    resolve_dxc_runtime_in(dxc_dir)
        .map_err(|error| eyre::eyre!("{}; run `water doctor` to reinstall dxc", error))
}

/// Collect [`DXC_RUNTIME_LIBRARIES`] from `dxc_dir`; a missing directory or
/// library is `NotFound`.
fn resolve_dxc_runtime_in(dxc_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    DXC_RUNTIME_LIBRARIES
        .iter()
        .map(|name| {
            let path = dxc_dir.join(name);
            path.is_file().then_some(path).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "the dxc installation at {} is missing {name}",
                        dxc_dir.display()
                    ),
                )
            })
        })
        .collect()
}

/// Stage the DirectX Shader Compiler runtime into `destination` for a Windows
/// binary whose wgpu DirectX 12 backend `LoadLibrary`s `dxcompiler.dll` and
/// `dxil.dll` by name at run time.
///
/// Windows resolves those names against the executable's directory before
/// `PATH`, so a staged copy always wins over a same-named library elsewhere
/// on `PATH` — and makes the binary self-contained on a machine without the
/// `dxc` tool installed.
///
/// # Errors
/// Returns an error when `dxc` is not installed, a runtime library is missing
/// beside it, or the destination cannot be created or written.
pub async fn stage_dxc_runtime(destination: &Path) -> eyre::Result<()> {
    smol::fs::create_dir_all(destination).await?;
    for source in resolve_dxc_runtime().await? {
        let file_name = source.file_name().ok_or_else(|| {
            eyre::eyre!("dxc runtime path has no file name: {}", source.display())
        })?;
        crate::utils::copy_file_if_changed(&source, &destination.join(file_name))
            .await
            .wrap_err_with(|| {
                format!(
                    "Failed to stage {} to {}",
                    source.display(),
                    destination.display()
                )
            })?;
    }
    Ok(())
}

/// The file name a recorded dynamic dependency carries: the basename for a
/// Mach-O install name like `@rpath/libwaterui_dylib.dylib`, the name itself
/// for a `DT_NEEDED` or PE import entry.
fn needed_file_name(recorded_name: &str) -> &str {
    recorded_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(recorded_name)
}

/// Whether `file_name` — an already validated library name, `lib` prefix and
/// platform extension included — names `waterui_dylib`, optionally carrying a
/// `-<metadata>` hash.
fn is_waterui_dylib_file_name(file_name: &str) -> bool {
    let Some((stem, _)) = file_name.rsplit_once('.') else {
        return false;
    };
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    stem == "waterui_dylib" || stem.starts_with("waterui_dylib-")
}

/// Whether a recorded dynamic dependency names the `crate_name` library —
/// `libwaterui_dylib-0123abcd.so`, `waterui_dylib.dll`, or the Mach-O install
/// name `@rpath/libwaterui_dylib.dylib` for `waterui_dylib`.
fn needed_library_matches(recorded_name: &str, crate_name: &str) -> bool {
    let stem = needed_file_name(recorded_name).to_ascii_lowercase();
    let Some((stem, _)) = stem.rsplit_once('.') else {
        return false;
    };
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    stem == crate_name || stem.starts_with(&format!("{crate_name}-"))
}

/// Whether a recorded dynamic dependency names the Rust standard library —
/// `libstd-<hash>.so`/`libstd-<hash>.dylib`/`std-<hash>.dll`.
fn is_rust_standard_library(recorded_name: &str) -> bool {
    let stem = needed_file_name(recorded_name).to_ascii_lowercase();
    let Some((stem, _)) = stem.rsplit_once('.') else {
        return false;
    };
    stem.strip_prefix("lib").unwrap_or(stem).starts_with("std-")
}

/// The build output a recorded needed library name resolves to: the file of
/// that exact name in one of `search_dirs`, or an error naming the needed
/// name, the directories searched, and the artifact that records it.
fn needed_library_source(
    needed_name: &str,
    search_dirs: &[PathBuf],
    artifact: &Path,
) -> eyre::Result<PathBuf> {
    let file_name = needed_file_name(needed_name);
    for dir in search_dirs {
        let candidate = dir.join(file_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(eyre::eyre!(
        "{} records a dynamic dependency on `{needed_name}` but no file named {file_name} was found in {}",
        artifact.display(),
        search_dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The shared-library names `artifact` records — [`needed_shared_libraries`]
/// off the blocking pool. A static archive or other unlinked artifact yields
/// an empty list.
async fn needed_libraries_of(artifact: &Path) -> Result<Vec<String>, RustBuildError> {
    let path = artifact.to_path_buf();
    unblock(move || needed_shared_libraries(&path))
        .await
        .map_err(|error| {
            RustBuildError::FailToBuildRustLibrary(io::Error::other(format!(
                "failed to read the dynamic dependencies of {}: {error}",
                artifact.display()
            )))
        })
}

/// The shared libraries an artifact's dynamic section records.
///
/// `DT_NEEDED` on ELF, the import descriptors on PE, the `LC_*_DYLIB` load
/// commands on Mach-O — the names the platform loader searches for when the
/// artifact runs. Mach-O entries carry the dylib's recorded install name
/// (typically an `@rpath/` path); `needed_file_name` reduces any entry to
/// the file name the loader resolves against its search paths. An artifact
/// that records no dynamic dependencies — a static archive, an object file —
/// yields an empty list.
///
/// # Errors
/// Returns an error when `path` cannot be read or its dynamic records are
/// malformed.
pub fn needed_shared_libraries(path: &Path) -> std::io::Result<Vec<String>> {
    let data = std::fs::read(path)?;
    let invalid =
        |error: object::read::Error| io::Error::new(io::ErrorKind::InvalidData, error.to_string());
    match object::FileKind::parse(&*data).map_err(invalid)? {
        object::FileKind::Elf32 => {
            elf_needed_libraries::<object::elf::FileHeader32<object::Endianness>>(&data)
        }
        object::FileKind::Elf64 => {
            elf_needed_libraries::<object::elf::FileHeader64<object::Endianness>>(&data)
        }
        object::FileKind::Pe32 => pe_needed_libraries::<object::pe::ImageNtHeaders32>(&data),
        object::FileKind::Pe64 => pe_needed_libraries::<object::pe::ImageNtHeaders64>(&data),
        object::FileKind::MachO32 => {
            macho_needed_libraries::<object::macho::MachHeader32<object::Endianness>>(&data)
        }
        object::FileKind::MachO64 => {
            macho_needed_libraries::<object::macho::MachHeader64<object::Endianness>>(&data)
        }
        // A static archive or any other artifact records no dynamic
        // dependencies; the caller resolves libraries as before.
        _ => Ok(Vec::new()),
    }
}

/// `DT_NEEDED` entries an ELF image's dynamic section records.
fn elf_needed_libraries<Elf>(data: &[u8]) -> std::io::Result<Vec<String>>
where
    Elf: object::read::elf::FileHeader<Endian = object::Endianness>,
{
    use object::read::elf::{Dyn as _, ElfFile};

    let invalid =
        |error: object::read::Error| io::Error::new(io::ErrorKind::InvalidData, error.to_string());
    let file = ElfFile::<Elf>::parse(data).map_err(invalid)?;
    let endian = file.endian();
    let sections = file.elf_section_table();
    let Some((dyns, strings_index)) = sections.dynamic(endian, data).map_err(invalid)? else {
        return Ok(Vec::new());
    };
    let strings = sections
        .strings(endian, data, strings_index)
        .map_err(invalid)?;
    Ok(dyns
        .iter()
        .filter(|d| d.tag32(endian) == Some(object::elf::DT_NEEDED))
        .filter_map(|d| d.string(endian, strings).ok())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect())
}

/// The DLL names a PE image's import descriptors record.
fn pe_needed_libraries<Pe>(data: &[u8]) -> std::io::Result<Vec<String>>
where
    Pe: object::read::pe::ImageNtHeaders,
{
    use object::LittleEndian;

    let invalid =
        |error: object::read::Error| io::Error::new(io::ErrorKind::InvalidData, error.to_string());
    let file = object::read::pe::PeFile::<Pe>::parse(data).map_err(invalid)?;
    let Some(import_table) = file.import_table().map_err(invalid)? else {
        return Ok(Vec::new());
    };
    let mut names = Vec::new();
    let mut descriptors = import_table.descriptors().map_err(invalid)?;
    while let Some(descriptor) = descriptors.next().map_err(invalid)? {
        let name = import_table
            .name(descriptor.name.get(LittleEndian))
            .map_err(invalid)?;
        names.push(String::from_utf8_lossy(name).into_owned());
    }
    Ok(names)
}

/// The names a Mach-O image's `LC_*_DYLIB` load commands record — its own
/// `LC_ID_DYLIB` identity is a separate load command and never included.
fn macho_needed_libraries<Mach>(data: &[u8]) -> std::io::Result<Vec<String>>
where
    Mach: object::read::macho::MachHeader,
{
    let invalid =
        |error: object::read::Error| io::Error::new(io::ErrorKind::InvalidData, error.to_string());
    let file = object::read::macho::MachOFile::<Mach>::parse(data).map_err(invalid)?;
    let endian = file.endian();
    let mut commands = file.macho_load_commands().map_err(invalid)?;
    let mut names = Vec::new();
    while let Some(command) = commands.next().map_err(invalid)? {
        if let Some(dylib) = command.dylib().map_err(invalid)? {
            let name = command.string(endian, dylib.dylib.name).map_err(invalid)?;
            names.push(String::from_utf8_lossy(name).into_owned());
        }
    }
    Ok(names)
}

/// The unhashed profile-root spelling a `dylib` unit uplifts to — the name a
/// path-sourced unit carries; git- and registry-sourced units record hashed
/// `deps/` names instead (cli#184).
#[cfg(test)]
fn dynamic_library_file_name(crate_name: &str, triple: &Triple) -> String {
    if triple.operating_system == OperatingSystem::Windows {
        format!("{crate_name}.dll")
    } else {
        format!("lib{crate_name}.{}", lib_extension_for_triple(triple))
    }
}

/// Find the dynamic standard library a directory holds for `triple`.
///
/// A missing directory or an empty match set is `NotFound`; several
/// candidates is an error — the caller cannot tell which `libstd` the build
/// actually linked.
fn resolve_rust_standard_library_in(libdir: &Path, triple: &Triple) -> std::io::Result<PathBuf> {
    let (prefix, extension) = if triple.operating_system == OperatingSystem::Windows {
        ("std-", "dll")
    } else {
        ("libstd-", lib_extension_for_triple(triple))
    };
    let entries = match std::fs::read_dir(libdir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} does not exist", libdir.display()),
            ));
        }
        Err(error) => return Err(error),
    };
    let mut matches = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(prefix)
                        && path.extension().and_then(|extension| extension.to_str())
                            == Some(extension)
                })
        })
        .collect::<Vec<_>>();
    matches.sort_unstable();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Rust target libdir {} contains no dynamic standard library for {triple}",
                libdir.display()
            ),
        )),
        _ => Err(std::io::Error::other(format!(
            "Rust target libdir {} contains multiple dynamic standard libraries for {triple}: {}",
            libdir.display(),
            matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Represents a Rust build for a specific target triple.
#[derive(Debug, Clone)]
pub struct RustBuild {
    path: PathBuf,
    triple: Triple,
    project: Option<Project>,
    /// Explicit Cargo target directory for cross-project artifact reuse.
    target_dir: Option<PathBuf>,
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<PathBuf>,
    /// Cargo features to enable.
    features: Vec<String>,
    /// Override the final crate type built by `cargo rustc`.
    crate_type_override: Option<String>,
    /// Extra rustc flags to append via `RUSTFLAGS`.
    rustc_flags: Vec<String>,
    /// Rustc flags that apply to the final crate only, via `cargo rustc -- <flags>`.
    ///
    /// `RUSTFLAGS` is hashed into every dependency unit's fingerprint, so a flag that
    /// only matters when linking the final artifact — an `-rpath` link argument, say —
    /// must not go through [`Self::with_rustc_flag`]: two builds sharing one target
    /// directory that disagree about `RUSTFLAGS` invalidate each other's entire
    /// dependency graph. Trailing `cargo rustc` arguments reach only the selected
    /// target's own compilation and leave dependency fingerprints alone.
    final_rustc_args: Vec<String>,
    /// rustup toolchain name (a nightly) when this build compiles the standard
    /// library from source via `-Zbuild-std`.
    ///
    /// Cargo only ever emits the `rlib` half of a source-built `std`, so a
    /// shared-runtime build on a target whose prebuilt `libstd` is unusable —
    /// Android's is 4 KB-aligned, which 16 KB-page devices reject — runs Cargo
    /// under the `water` rustc wrapper, which adds the `dylib` crate type to
    /// the `std` unit and hands the produced `.so` to every dependent.
    build_std_toolchain: Option<String>,
    /// Extra environment variables to set for the cargo build process.
    envs: Vec<(String, OsString)>,
    /// Sink compile progress is reported to while cargo runs.
    progress: Option<BuildProgress>,
    /// How long a `--bin` build's artifact lock outlives `build_inner`.
    artifact_lock_scope: ArtifactLockScope,
}

/// The optimization/debug-info trade-off a Cargo build selects.
///
/// The variants are realized on top of the workspace's declared `dev` and
/// `release` profiles through `CARGO_PROFILE_*` overrides, so they work on
/// user projects and generated crates alike without manifest changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildProfile {
    /// The `dev` profile as declared: unoptimized, with debug info.
    #[default]
    Debug,
    /// The `dev` profile lifted to a light optimization level with full debug
    /// info — the development default for self-drawn backends, whose
    /// per-frame cost sits in rendering dependencies rather than in app code.
    Optimized,
    /// The `release` profile at full speed optimization, without debug info.
    Release,
    /// The `release` profile at full speed optimization, with debug info and
    /// symbols kept so a profiler can symbolicate the recording.
    Profiling,
}

impl BuildProfile {
    /// Whether the build uses Cargo's `release` profile — artifacts land in
    /// the `release/` profile directory and `cargo` gets `--release`.
    #[must_use]
    pub const fn is_release(self) -> bool {
        matches!(self, Self::Release | Self::Profiling)
    }

    /// Whether the profile keeps the development-run shape: the `include_web!`
    /// dev server may serve mounts and the artifact packages as debuggable.
    #[must_use]
    pub const fn is_development(self) -> bool {
        !self.is_release()
    }

    /// `CARGO_PROFILE_*` overrides realizing this profile on the workspace's
    /// declared `dev`/`release` profiles.
    ///
    /// These compose with `profile.*.package."*"` overrides a manifest may
    /// declare: the env sets the profile's base value, so generated crates —
    /// whose `dev` profile already lifts dependencies to `opt-level 2` — keep
    /// that dependency optimization while the base rises to cover the root
    /// crate and the per-unit debug-assertion switches the override table
    /// does not mention.
    ///
    /// A development build links the shared Rust runtime, a `dylib` crate,
    /// and a `dylib` links the toolchain's prebuilt `std`, which carries the
    /// `panic_unwind` runtime: under the packaging profile's `panic = "abort"`
    /// rustc refuses the link ("the linked panic runtime `panic_unwind` is
    /// not compiled with this crate's panic strategy `abort`"), and under its
    /// `lto = true` it refuses to prefer dynamic linking at all. The release
    /// profiles therefore unwind without LTO here; the packaging build keeps
    /// the manifest's `abort` and LTO.
    fn development_envs(self) -> Vec<(String, OsString)> {
        let entries: &[(&str, &str)] = match self {
            Self::Debug => &[],
            Self::Optimized => &[
                ("CARGO_PROFILE_DEV_OPT_LEVEL", "1"),
                ("CARGO_PROFILE_DEV_DEBUG", "true"),
                ("CARGO_PROFILE_DEV_DEBUG_ASSERTIONS", "false"),
                ("CARGO_PROFILE_DEV_OVERFLOW_CHECKS", "false"),
            ],
            Self::Release => &[
                ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3"),
                ("CARGO_PROFILE_RELEASE_PANIC", "unwind"),
                ("CARGO_PROFILE_RELEASE_LTO", "off"),
            ],
            Self::Profiling => &[
                ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3"),
                ("CARGO_PROFILE_RELEASE_PANIC", "unwind"),
                ("CARGO_PROFILE_RELEASE_LTO", "off"),
                ("CARGO_PROFILE_RELEASE_DEBUG", "true"),
                ("CARGO_PROFILE_RELEASE_STRIP", "none"),
            ],
        };
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), OsString::from(*value)))
            .collect()
    }
}

/// Options for building Rust libraries.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    profile: BuildProfile,
    output_dir: Option<std::path::PathBuf>,
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<std::path::PathBuf>,
    /// Optional target triple override.
    target_triple: Option<Triple>,
    /// Rust runtime linkage used by the final native application.
    linkage: RustLinkage,
    /// Whether the built app will `dlopen` `WaterUI` modules — a preview
    /// support app — and therefore must package the shared Rust runtime
    /// instead of linking it in, even on a platform that otherwise forces
    /// static linkage.
    dynamic_module_loading: bool,
    /// Whether `include_web!` mounts are dev-server-served and skipped when
    /// the build stages assets (Hydrolysis stages at build time).
    dev_server: bool,
    /// `CARGO_PROFILE_*` overrides applied to the cargo invocation.
    cargo_envs: Vec<(String, OsString)>,
    /// Sink compile progress is reported to while cargo runs.
    progress: Option<BuildProgress>,
}

impl BuildOptions {
    /// Create options for a development build that uses the shared Rust runtime.
    ///
    /// Development runs want wall-clock speed: `Release` and `Profiling` force
    /// `opt-level 3` rather than the size-optimized `opt-level "z"` the
    /// packaging profile declares, and `Optimized`/`Profiling`/`Release` all
    /// carry `CARGO_PROFILE_*` overrides the cargo invocation applies.
    #[must_use]
    pub fn development(profile: BuildProfile) -> Self {
        Self {
            profile,
            output_dir: None,
            sccache_path: None,
            target_triple: None,
            linkage: RustLinkage::SharedRuntime,
            dynamic_module_loading: false,
            dev_server: false,
            cargo_envs: profile.development_envs(),
            progress: None,
        }
    }

    /// Link the Rust runtime in, whatever the caller asked for.
    ///
    /// A platform whose loader cannot accept the toolchain's prebuilt runtime
    /// says so here rather than at the link step, so that the target directory
    /// and the staged libraries agree with what is actually built.
    #[must_use]
    pub fn with_static_runtime(mut self) -> Self {
        self.linkage = RustLinkage::Static;
        // No shared runtime to link, so the manifest's panic strategy and LTO
        // stand.
        self.cargo_envs.retain(|(key, _)| {
            key != "CARGO_PROFILE_RELEASE_PANIC" && key != "CARGO_PROFILE_RELEASE_LTO"
        });
        self
    }

    /// Create options for a self-contained package build.
    ///
    /// A packaged artifact builds under the profile the workspace declares —
    /// no `CARGO_PROFILE_*` overrides: the release profile's size tuning
    /// (`opt-level "z"`, symbol stripping) is the shipped configuration.
    #[must_use]
    pub const fn packaging(profile: BuildProfile) -> Self {
        Self {
            profile,
            output_dir: None,
            sccache_path: None,
            target_triple: None,
            linkage: RustLinkage::Static,
            dynamic_module_loading: false,
            dev_server: false,
            cargo_envs: Vec::new(),
            progress: None,
        }
    }

    /// Whether the build uses Cargo's `release` profile.
    #[must_use]
    pub const fn is_release(&self) -> bool {
        self.profile.is_release()
    }

    /// The selected build profile.
    #[must_use]
    pub const fn profile(&self) -> BuildProfile {
        self.profile
    }

    /// `CARGO_PROFILE_*` overrides the cargo invocation applies.
    #[must_use]
    pub fn cargo_envs(&self) -> &[(String, OsString)] {
        &self.cargo_envs
    }

    /// Mark web mounts as dev-server-served for asset staging this build does.
    #[must_use]
    pub const fn with_dev_server(mut self, dev_server: bool) -> Self {
        self.dev_server = dev_server;
        self
    }

    /// Whether web mounts are dev-server-served and skipped during staging.
    #[must_use]
    pub const fn uses_dev_server(&self) -> bool {
        self.dev_server
    }

    /// Get the output directory, if specified
    #[must_use]
    pub fn output_dir(&self) -> Option<&std::path::Path> {
        self.output_dir.as_deref()
    }

    /// Set the output directory where built libraries should be copied
    #[must_use]
    pub fn with_output_dir(mut self, output_dir: impl Into<std::path::PathBuf>) -> Self {
        self.output_dir = Some(output_dir.into());
        self
    }

    /// Get the sccache path, if configured
    #[must_use]
    pub fn sccache_path(&self) -> Option<&std::path::Path> {
        self.sccache_path.as_deref()
    }

    /// Set the sccache path for compilation caching.
    ///
    /// When set, `RUSTC_WRAPPER` will be configured to use sccache,
    /// which can significantly improve build times by caching compiled artifacts.
    #[must_use]
    pub fn with_sccache(mut self, sccache_path: impl Into<std::path::PathBuf>) -> Self {
        self.sccache_path = Some(sccache_path.into());
        self
    }

    /// Get the explicit target triple override, if configured.
    #[must_use]
    pub const fn target_triple(&self) -> Option<&Triple> {
        self.target_triple.as_ref()
    }

    /// Override the target triple used for compilation.
    #[must_use]
    pub fn with_target_triple(mut self, target_triple: Triple) -> Self {
        self.target_triple = Some(target_triple);
        self
    }

    /// Get the selected Rust runtime linkage.
    #[must_use]
    pub const fn linkage(&self) -> RustLinkage {
        self.linkage
    }

    /// Mark the built app as a host for `dlopen`'d `WaterUI` modules.
    ///
    /// A preview support app resolves a pushed module's framework symbols
    /// against the runtime it already has open, so the shared runtime has to
    /// ship in the package rather than be linked into the app alone.
    #[must_use]
    pub const fn with_dynamic_module_loading(mut self) -> Self {
        self.dynamic_module_loading = true;
        self
    }

    /// Whether the built app hosts dynamically loaded `WaterUI` modules.
    #[must_use]
    pub const fn loads_dynamic_modules(&self) -> bool {
        self.dynamic_module_loading
    }

    /// Attach a compile-progress sink every cargo invocation this build
    /// performs reports to.
    #[must_use]
    pub fn with_progress(mut self, progress: BuildProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    /// The compile-progress sink, when one is attached.
    #[must_use]
    pub const fn progress(&self) -> Option<&BuildProgress> {
        self.progress.as_ref()
    }
}

/// Errors that can occur during the Rust build process.
#[derive(Debug, thiserror::Error)]
pub enum RustBuildError {
    /// Failed to execute cargo build.
    #[error("Failed to execute cargo build: {0}")]
    FailToExecuteCargoBuild(std::io::Error),

    /// Cargo executed but failed to build the Rust library.
    #[error("Failed to build Rust library: {0}")]
    FailToBuildRustLibrary(std::io::Error),
}

/// Cargo's compile-phase progress: one event per status line cargo writes to
/// stderr.
///
/// A cold build reports nothing to a captured pipe for its whole duration, so
/// `water run` and `water build` attach a [`BuildProgress`] sink that keeps
/// the compile visibly alive on every terminal. Events are parsed from
/// cargo's own output, never generated by a timer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileEvent {
    /// A `name vversion` status line: one crate unit moved through cargo's
    /// pipeline. `phase` is cargo's status word — `Compiling`, `Checking`,
    /// `Fresh`, `Downloading`, `Downloaded` or `Doc-tests`.
    Unit {
        /// Cargo's status word.
        phase: &'static str,
        /// The crate the status line names.
        name: String,
        /// The crate's version, when the status line carries one.
        version: Option<String>,
    },
    /// `Finished ...` — cargo's closing status line.
    Finished(String),
    /// Any other line — index and lock status, warnings, diagnostics,
    /// build-script output.
    Line(String),
}

/// The sink a cargo build reports its [`CompileEvent`]s into.
///
/// The terminal attaches one per build; events arrive on the task draining
/// cargo's stderr, so a sink must stay cheap.
#[derive(Clone)]
pub struct BuildProgress {
    report: std::sync::Arc<dyn Fn(CompileEvent) + Send + Sync>,
    /// Whether the sink renders every line live. When it does, a build
    /// failure report can tail the captured output instead of re-dumping what
    /// the user already watched scroll by.
    shows_all_lines: bool,
}

impl std::fmt::Debug for BuildProgress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BuildProgress(..)")
    }
}

impl BuildProgress {
    /// A sink that renders each event through `report`.
    #[must_use]
    pub fn new(report: impl Fn(CompileEvent) + Send + Sync + 'static) -> Self {
        Self {
            report: std::sync::Arc::new(report),
            shows_all_lines: false,
        }
    }

    /// Mark the sink as rendering every line live, including
    /// [`CompileEvent::Line`] diagnostics.
    #[must_use]
    pub const fn showing_all_lines(mut self) -> Self {
        self.shows_all_lines = true;
        self
    }

    /// Whether the sink renders every line live.
    #[must_use]
    pub const fn shows_all_lines(&self) -> bool {
        self.shows_all_lines
    }

    fn report(&self, event: CompileEvent) {
        (self.report)(event);
    }
}

/// Cargo status words whose line names one crate unit: `phase name vversion`.
const CARGO_UNIT_PHASES: &[&str] = &[
    "Compiling",
    "Checking",
    "Fresh",
    "Downloading",
    "Downloaded",
    "Doc-tests",
];

/// Classify one line of cargo's stderr into a [`CompileEvent`].
///
/// Cargo emits ANSI-colored status lines whenever color is forced — by the
/// `CARGO_TERM_COLOR` this module sets for terminal output, or by the user's
/// own `[term] color` configuration — so the line is classified on its
/// stripped text. Text-carrying events keep the raw line: an interactive sink
/// renders cargo's colors, and the piped and JSON renderers strip on emit.
fn classify_compile_line(line: &str) -> CompileEvent {
    let raw = line.trim();
    let stripped = console::strip_ansi_codes(raw);
    let text = stripped.trim();
    for phase in CARGO_UNIT_PHASES {
        let Some(rest) = text
            .strip_prefix(phase)
            .and_then(|rest| rest.strip_prefix(' '))
        else {
            continue;
        };
        // A unit line names `name vversion`; `Downloaded 12 crates` and
        // `Doc-tests foo` are status text, not a unit.
        let Some((name, version)) = rest.split_once(" v") else {
            return CompileEvent::Line(raw.to_owned());
        };
        let version = version.split([' ', '(']).next().unwrap_or_default();
        return CompileEvent::Unit {
            phase,
            name: name.to_owned(),
            version: (!version.is_empty()).then(|| version.to_owned()),
        };
    }
    if text.starts_with("Finished ") {
        return CompileEvent::Finished(raw.to_owned());
    }
    CompileEvent::Line(raw.to_owned())
}

/// Spawn a configured command with piped stdio, drain both streams to their
/// ends, and report cargo's stderr status lines to `progress`.
///
/// The returned [`std::process::Output`] is exactly what `output()` produces:
/// pipes are always drained and collected in full, so failure reporting and
/// retry detection see the same captured text whether or not a sink is
/// attached. When no sink is attached and the CLI's output passthrough is
/// enabled, raw stderr chunks echo to the terminal as they arrive — the
/// historical `Stdio::inherit` behavior. Stdout is collected silently: a
/// `--message-format=json` caller parses it as a protocol stream, so it is
/// never mirrored.
pub(crate) async fn command_output_with_progress(
    command: &mut Command,
    progress: Option<BuildProgress>,
) -> io::Result<std::process::Output> {
    let mut child = command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout_pipe = child.stdout.take().expect("stdout is piped");
    let stderr_pipe = child.stderr.take().expect("stderr is piped");

    // Raw chunk echo reproduces `Stdio::inherit` for a build carrying no
    // progress sink; a sink renders the parsed events itself.
    let echo = progress.is_none() && std_output_enabled();
    // The drains run as their own tasks: inlined into this future their read
    // buffers alone would push it past clippy's `large_futures` threshold.
    let stdout_task = smol::spawn(drain_pipe(stdout_pipe));
    let stderr_task = smol::spawn(drain_cargo_stderr(stderr_pipe, progress, echo));
    let status = child.status().await?;
    let stdout = stdout_task.await?;
    let stderr = stderr_task.await?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Drain a piped child stream to EOF, collecting every byte.
async fn drain_pipe(mut reader: impl smol::io::AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut collected = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..read]);
    }
    Ok(collected)
}

/// Drain cargo's piped stderr: collect every byte, echo raw chunks when
/// passthrough is enabled, and report each completed line's [`CompileEvent`]
/// to `progress` as it arrives.
async fn drain_cargo_stderr(
    mut reader: impl smol::io::AsyncRead + Unpin,
    progress: Option<BuildProgress>,
    echo: bool,
) -> io::Result<Vec<u8>> {
    let mut collected = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..read]);
        if echo {
            let _ = io::stderr().write_all(&chunk[..read]);
            let _ = io::stderr().flush();
        }
        if let Some(sink) = &progress {
            pending.extend_from_slice(&chunk[..read]);
            // A line feed is never a UTF-8 continuation byte, so scanning raw
            // bytes for line boundaries and decoding only complete lines
            // cannot corrupt a multibyte character straddling a chunk.
            while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=newline).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end();
                if !line.trim().is_empty() {
                    sink.report(classify_compile_line(line));
                }
            }
        }
    }
    if let Some(sink) = &progress {
        let tail = String::from_utf8_lossy(&pending);
        let tail = tail.trim_end();
        if !tail.trim().is_empty() {
            sink.report(classify_compile_line(tail));
        }
    }
    Ok(collected)
}

impl RustBuild {
    /// Create a new rust build for the given path and target triple.
    pub fn new(path: impl AsRef<Path>, triple: Triple) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            triple,
            project: None,
            target_dir: None,
            sccache_path: None,
            features: Vec::new(),
            crate_type_override: None,
            rustc_flags: Vec::new(),
            final_rustc_args: Vec::new(),
            build_std_toolchain: None,
            envs: Vec::new(),
            progress: None,
            artifact_lock_scope: ArtifactLockScope::default(),
        }
    }

    /// Build on behalf of `project`: its framework prepares the crate, and
    /// cargo runs under the rustup toolchain the project's own directory
    /// selects — the generated crate sits in the build cache, outside the
    /// project tree, where rustup would fall back to its default toolchain and
    /// link the runtime against a `libstd` the project's toolchain does not
    /// have.
    pub(crate) fn with_project(mut self, project: &Project) -> Self {
        self.project = Some(project.clone());
        self
    }

    /// Use an explicit Cargo target directory.
    #[must_use]
    pub fn with_target_dir(mut self, target_dir: impl Into<PathBuf>) -> Self {
        self.target_dir = Some(target_dir.into());
        self
    }

    /// Set the sccache path for compilation caching.
    ///
    /// When set, `RUSTC_WRAPPER` will be configured to use sccache,
    /// which can significantly improve incremental build times.
    #[must_use]
    pub fn with_sccache(mut self, sccache_path: PathBuf) -> Self {
        self.sccache_path = Some(sccache_path);
        self
    }

    /// Add a Cargo feature to enable during the build.
    ///
    /// Features are passed to cargo via `--features`.
    #[must_use]
    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.features.push(feature.into());
        self
    }

    /// Add multiple Cargo features to enable during the build.
    #[must_use]
    pub fn with_features(mut self, features: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.features.extend(features.into_iter().map(Into::into));
        self
    }

    /// Cargo features this build passes via `--features`.
    #[must_use]
    pub fn features(&self) -> &[String] {
        &self.features
    }

    /// Add a rustc flag to the build via `RUSTFLAGS`.
    #[must_use]
    pub fn with_rustc_flag(mut self, flag: impl Into<String>) -> Self {
        self.rustc_flags.push(flag.into());
        self
    }

    /// Add a rustc flag that applies to the final crate only.
    ///
    /// The flag is passed as a trailing `cargo rustc` argument instead of through
    /// `RUSTFLAGS`, so dependency unit fingerprints stay identical across builds that
    /// differ only in how their final artifact links. See the field documentation on
    /// `final_rustc_args` for why link arguments must take this route.
    #[must_use]
    pub fn with_final_rustc_arg(mut self, flag: impl Into<String>) -> Self {
        self.final_rustc_args.push(flag.into());
        self
    }

    /// Build the Rust standard library from source with `-Zbuild-std` on the
    /// named toolchain (a nightly with `rust-src`), sharing one `libstd`
    /// dylib across the graph.
    ///
    /// The build runs Cargo under the `water` rustc wrapper
    /// ([`crate::rustc_wrapper`]): Cargo strips `dylib` from `std`'s crate
    /// types under `-Zbuild-std`, and the wrapper restores it so the produced
    /// `libstd-*.so` carries the same strict version hash as the rlib every
    /// dependent is compiled against. The wrapper also publishes the dylib
    /// into the profile's `deps/` directory, where
    /// [`RustDynamicLibraries::resolve`] finds it before the toolchain's
    /// prebuilt copy.
    #[must_use]
    pub fn with_build_std(mut self, toolchain: impl Into<String>) -> Self {
        self.build_std_toolchain = Some(toolchain.into());
        self
    }

    /// Prefer dynamic Rust dependencies and emit loader search paths for them.
    #[must_use]
    pub fn with_preferred_dynamic_linking(self) -> Self {
        self.with_rustc_flag("-Cprefer-dynamic")
            .with_rustc_flag("-Crpath")
    }

    /// Configure this build for the selected Rust runtime linkage.
    ///
    /// A shared-runtime development build enables the project's `dev` feature (which
    /// resolves the shared `waterui-dylib` runtime), prefers dynamic linking, and —
    /// when the platform's loader needs them — embeds loader search paths into the
    /// final artifact only. A static packaging build needs none of this.
    ///
    /// A platform generally needs more than one: the binary is run both where it was
    /// built, with the runtime staged beside it, and from inside a packaged artifact
    /// that puts the runtime somewhere else. Each path becomes its own `-rpath`, and
    /// the loader tries them in order.
    #[must_use]
    pub fn with_linkage(
        self,
        linkage: RustLinkage,
        development_feature: &str,
        loader_search_paths: &[&str],
    ) -> Self {
        if linkage == RustLinkage::Static {
            return self;
        }
        let build = self
            .with_feature(development_feature)
            .with_preferred_dynamic_linking();
        loader_search_paths.iter().fold(build, |build, path| {
            build.with_final_rustc_arg(format!("-Clink-arg=-Wl,-rpath,{path}"))
        })
    }

    /// Override the library crate type passed to `rustc`.
    #[must_use]
    pub fn with_crate_type_override(mut self, crate_type: impl Into<String>) -> Self {
        self.crate_type_override = Some(crate_type.into());
        self
    }

    /// Add an environment variable for the cargo build process.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Add multiple environment variables for the cargo build process.
    #[must_use]
    pub fn with_envs(mut self, envs: impl IntoIterator<Item = (String, OsString)>) -> Self {
        self.envs.extend(envs);
        self
    }

    /// Attach a compile-progress sink the cargo invocation reports to.
    ///
    /// Each [`CompileEvent`] is parsed from cargo's own stderr stream, so the
    /// report is driven by build output rather than a timer.
    #[must_use]
    pub fn with_progress(mut self, progress: BuildProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    /// How long a `--bin` build's artifact lock outlives `build_inner`.
    ///
    /// The default, [`ArtifactLockScope::UntilLaunch`], hands the lock to
    /// whoever executes the shared `<profile>/<name>` uplift the returned
    /// [`BuiltTarget::executable`] names. [`ArtifactLockScope::UntilMarked`]
    /// releases it where the `deps/<name>-<marker>` link exists — the scope
    /// for binaries nothing execs through the uplift, like the Apple entry
    /// `[[bin]]` and the CEF helper, whose lock file sits in the shared
    /// profile directory and would block every other project's same-named
    /// build for as long as a `water run` kept it.
    #[must_use]
    pub const fn with_artifact_lock_scope(mut self, scope: ArtifactLockScope) -> Self {
        self.artifact_lock_scope = scope;
        self
    }

    /// Get the target triple for this build.
    #[must_use]
    pub const fn triple(&self) -> &Triple {
        &self.triple
    }

    /// Build rust library in development mode.
    ///
    /// Will produce debug symbols and less optimizations for faster builds.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn dev_build(&self) -> Result<BuiltTarget, RustBuildError> {
        Box::pin(self.build_lib(false)).await
    }

    /// Build rust library in release mode.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn release_build(&self) -> Result<BuiltTarget, RustBuildError> {
        Box::pin(self.build_lib(true)).await
    }

    /// Build the crate's library target.
    ///
    /// The returned [`BuiltTarget`] carries the profile directory plus the
    /// artifact Cargo reported. A crate emitting several library crate types
    /// needs [`Self::with_crate_type_override`] to say which one is wanted —
    /// the build fails rather than guess.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn build_lib(&self, release: bool) -> Result<BuiltTarget, RustBuildError> {
        Box::pin(self.build_inner(release, CargoTarget::Lib, self.lib_artifact_extension())).await
    }

    /// Build a dynamic library (cdylib) and return Cargo's reported build result.
    ///
    /// The path is Cargo's own `compiler-artifact` report, so the returned file
    /// is the one this build wrote even when another project's identically
    /// named crate shares the target directory.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If the library was not found after building.
    pub async fn build_dylib(&self, release: bool) -> Result<BuiltTarget, RustBuildError> {
        Box::pin(self.build_inner(
            release,
            CargoTarget::Lib,
            Some(lib_extension_for_triple(&self.triple)),
        ))
        .await
    }

    /// Build a static library (staticlib) and return Cargo's reported build result.
    ///
    /// The manifest's declared crate types all emit — no `--crate-type`
    /// selection — so the dependency units carry the same fingerprints a
    /// sibling binary build in this target directory computes; a narrowed
    /// build would compile the whole dependency graph a second time for the
    /// entry binary that follows. The `.a`/`.lib` is picked out of Cargo's
    /// report by extension.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If the library was not found after building.
    pub async fn build_staticlib(&self, release: bool) -> Result<BuiltTarget, RustBuildError> {
        Box::pin(self.build_inner(
            release,
            CargoTarget::Lib,
            crate_type_artifact_extension("staticlib", &self.triple),
        ))
        .await
    }

    /// Builds one named binary and returns its full output path.
    ///
    /// The path is `deps/<name>-<marker>` — a hard link, taken under the
    /// binary artifact lock, to the `executable` this run's
    /// `compiler-artifact` message reported. The unhashed `<profile>/<name>`
    /// uplift the message names is shared by every same-named unit in the
    /// target directory, so it is only trusted while the lock is held; the
    /// marked link stays this variant's bytes afterwards. See
    /// `binary_artifact_lock` and `marked_binary_artifact`.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo fails or the expected binary is missing.
    pub async fn build_binary(
        &self,
        binary_name: &str,
        release: bool,
    ) -> Result<BuiltTarget, RustBuildError> {
        Box::pin(self.build_inner(release, CargoTarget::Binary(binary_name), None)).await
    }

    /// Compute the expected dylib output path without building.
    ///
    /// This uses `cargo metadata` to resolve the target directory to avoid assuming
    /// a fixed `target/` path.
    ///
    /// # Errors
    /// Returns an error if Cargo metadata cannot be read.
    pub async fn dylib_path(
        &self,
        crate_name: &str,
        release: bool,
    ) -> Result<PathBuf, RustBuildError> {
        let lib_dir = self.lib_output_dir(release).await?;
        let lib_name = crate_name.replace('-', "_");
        let ext = lib_extension_for_triple(&self.triple);
        Ok(lib_dir.join(format!("lib{lib_name}.{ext}")))
    }

    /// Return target directory path
    async fn build_inner(
        &self,
        release: bool,
        cargo_target: CargoTarget<'_>,
        artifact_extension: Option<&'static str>,
    ) -> Result<BuiltTarget, RustBuildError> {
        // Held until `build_inner` returns: the shared lease keeps the build
        // cache garbage collector from dropping the target mid-compile.
        let _target_lease = self.shared_target_lease().await?;
        let profile_dir = self.lib_output_dir(release).await?;
        // The shared-target lease is a *shared* lease — concurrent `water`
        // builds in this target directory are expected. For a `--bin` unit,
        // hold an exclusive per-binary lock from before Cargo's invocation,
        // covering at least the `deps/<name>-<marker>` relink; where it
        // ends is `artifact_lock_scope`'s call — inside the returned
        // `BuiltTarget`'s `executable` for a binary a launcher execs
        // through the shared `<profile>/<name>` spelling, or dropped once
        // the marked link exists for an uplift nobody execs (#2073).
        let artifact_lock = match cargo_target {
            CargoTarget::Binary(name) => Some(binary_artifact_lock(&profile_dir, name).await?),
            CargoTarget::Lib => None,
        };
        let user_rustflags = self
            .user_rustflags(&self.project_cargo_config_files()?)
            .await?;
        let mut output = self
            .cargo_build_output(release, cargo_target, &user_rustflags)
            .await?;

        if !output.status.success() {
            let mut combined = combined_build_output(&output);

            // Handle stale CMake generator caches (e.g. Unix Makefiles vs Ninja)
            // by cleaning crate-local CMake build dirs and retrying once.
            if should_retry_after_cmake_generator_mismatch(&combined)
                && self.clean_stale_cmake_build_dirs().await?
            {
                output = self
                    .cargo_build_output(release, cargo_target, &user_rustflags)
                    .await?;
                combined = combined_build_output(&output);
            }

            if !output.status.success() && should_auto_install_meson(&combined) {
                match ensure_meson_installed_for_build().await {
                    Ok(()) => {
                        output = self
                            .cargo_build_output(release, cargo_target, &user_rustflags)
                            .await?;
                    }
                    Err(install_err) => {
                        return Err(RustBuildError::FailToBuildRustLibrary(
                            std::io::Error::other(format!(
                                "Cargo build failed and meson appears missing.\n\
Automatic meson installation failed: {install_err}\n\n{}",
                                self.failure_report(&combined)
                            )),
                        ));
                    }
                }
            }
        }

        if !output.status.success() {
            let combined = combined_build_output(&output);
            return Err(RustBuildError::FailToBuildRustLibrary(
                std::io::Error::other(format!(
                    "Cargo build failed:\n{}",
                    self.failure_report(&combined)
                )),
            ));
        }

        let mut artifact =
            reported_artifact(&output.stdout, &self.path, cargo_target, artifact_extension)?;

        // A dependency's final `dylib`/`cdylib` artifact uplifts to an
        // unhashed name (`deps/libwaterui_dylib.so`), so one filename serves
        // every same-named package sharing this target — last writer wins.
        // A `fresh` unit emits nothing yet still reports that path, which can
        // leave a different source's bytes where `water run` expects its own
        // runtime; rebuild_stale_shared_dylibs repairs that from the
        // dep-info written alongside.
        self.rebuild_stale_shared_dylibs(
            &mut output,
            &mut artifact,
            release,
            cargo_target,
            artifact_extension,
            &user_rustflags,
        )
        .await?;
        // Relink under the still-held artifact lock: the `executable`
        // report names the shared uplift, so the variant-stable path has
        // to be taken while no same-named build can overwrite it. The
        // reported uplift path is kept on `executable` — the launcher
        // spelling whose directory holds the staged shared runtime.
        let executable = matches!(cargo_target, CargoTarget::Binary(_)).then(|| artifact.clone());
        let artifact = match cargo_target {
            CargoTarget::Binary(name) => {
                let marker = self.artifact_marker(release, cargo_target, &user_rustflags);
                marked_binary_artifact(&artifact, &profile_dir, name, &marker, &self.triple).await?
            }
            CargoTarget::Lib => artifact,
        };
        // The `deps/<name>-<marker>` link exists now. An uplift nobody
        // launches — the Apple entry `[[bin]]`, the CEF helper — releases
        // its lock here rather than letting a `BuiltTarget` a `water run`
        // holds through packaging and launch block every other project's
        // same-named build; the lock file is per-name in the shared
        // profile directory.
        let artifact_lock = match self.artifact_lock_scope {
            ArtifactLockScope::UntilLaunch => artifact_lock,
            ArtifactLockScope::UntilMarked => {
                drop(artifact_lock);
                None
            }
        };

        let shared_runtime = reported_shared_runtime(&output.stdout)?;
        let app_library = match self.project.as_ref() {
            Some(project) => {
                app_library_artifact(&output.stdout, &project.root().join("Cargo.toml"))?
            }
            None => None,
        };
        Ok(BuiltTarget {
            profile_dir,
            artifact,
            executable: executable.map(|path| SharedExecutable {
                path,
                _lock: artifact_lock,
            }),
            entry_binary: None,
            shared_runtime,
            app_library,
            cef_helper: None,
        })
    }

    /// Rebuild shared dylib units whose `fresh` reports left another
    /// source's bytes at the unhashed artifact name.
    ///
    /// The dep-info `.d` written alongside an uplifted artifact records the
    /// producing sources; when they are not this unit's — or when no
    /// dep-info exists to say — the package is cleaned so the rebuild emits
    /// this source's artifact. The rebuild compiles the cleaned package
    /// anew, so a unit it still reports `fresh` in the same state is a
    /// cache this CLI cannot repair by rebuilding, reported instead of
    /// retried. `artifact` is reselected from the rebuild's output.
    async fn rebuild_stale_shared_dylibs(
        &self,
        output: &mut std::process::Output,
        artifact: &mut PathBuf,
        release: bool,
        cargo_target: CargoTarget<'_>,
        artifact_extension: Option<&'static str>,
        user_rustflags: &[String],
    ) -> Result<(), RustBuildError> {
        let needed = needed_libraries_of(artifact).await?;
        let stale = stale_shared_dylib_packages(&output.stdout, &needed).await?;
        if stale.is_empty() {
            return Ok(());
        }
        let target_dir = self.target_directory().await?;
        for unit in &stale {
            warn!(
                package = unit.package,
                artifact = %unit.artifact.display(),
                "discarding a shared dylib unit and rebuilding it: {}",
                unit.reason
            );
            clean_cargo_package(&self.path, &unit.package, &target_dir).await?;
        }
        *output = self
            .cargo_build_output(release, cargo_target, user_rustflags)
            .await?;
        if !output.status.success() {
            let combined = combined_build_output(output);
            return Err(RustBuildError::FailToBuildRustLibrary(
                std::io::Error::other(format!(
                    "Cargo build failed:\n{}",
                    self.failure_report(&combined)
                )),
            ));
        }
        *artifact =
            reported_artifact(&output.stdout, &self.path, cargo_target, artifact_extension)?;
        let needed = needed_libraries_of(artifact).await?;
        let unrecovered = stale_shared_dylib_packages(&output.stdout, &needed).await?;
        if !unrecovered.is_empty() {
            return Err(unrecoverable_shared_dylib_error(&unrecovered, &target_dir));
        }
        Ok(())
    }

    /// The artifact extension this build's `--crate-type` override produces,
    /// when one is set and the type has a known file shape.
    fn lib_artifact_extension(&self) -> Option<&'static str> {
        self.crate_type_override
            .as_deref()
            .and_then(|crate_type| crate_type_artifact_extension(crate_type, &self.triple))
    }

    /// The text a build failure report embeds: the whole captured output, or
    /// only its tail when the attached sink already rendered every line live.
    fn failure_report(&self, combined: &str) -> String {
        if self
            .progress
            .as_ref()
            .is_some_and(BuildProgress::shows_all_lines)
        {
            output_tail(combined)
        } else {
            combined.to_owned()
        }
    }

    async fn clean_stale_cmake_build_dirs(&self) -> Result<bool, RustBuildError> {
        let target_dir = self.target_directory().await?;
        let triple = self.triple.to_string();

        let removed = unblock(move || {
            let mut removed = 0usize;
            removed +=
                remove_cmake_build_dirs_in(&target_dir.join(&triple).join("debug").join("build"))?;
            removed += remove_cmake_build_dirs_in(
                &target_dir.join(&triple).join("release").join("build"),
            )?;
            Ok::<usize, std::io::Error>(removed)
        })
        .await
        .map_err(|error| {
            RustBuildError::FailToBuildRustLibrary(std::io::Error::other(format!(
                "Failed to clean stale CMake cache: {error}"
            )))
        })?;

        Ok(removed > 0)
    }

    /// The `--config` files restoring the project's Cargo-config hierarchy
    /// on this build, in Cargo's precedence order. Empty when the crate
    /// builds inside the project, where discovery already reaches its
    /// config files.
    fn project_cargo_config_files(&self) -> Result<Vec<PathBuf>, RustBuildError> {
        let Some(project) = self.project.as_ref() else {
            return Ok(Vec::new());
        };
        crate::toolchain::cargo_project_config::cargo_config_files(project.root(), &self.path)
            .map_err(|error| {
                RustBuildError::FailToBuildRustLibrary(io::Error::other(error.to_string()))
            })
    }

    /// The rustflags Cargo resolves for this build before the CLI's own
    /// `rustc_flags`, over the environment the spawned cargo actually sees:
    /// this process's variables overlaid by `self.envs`, last entry per key
    /// winning — the same order `Command::env` applies.
    ///
    /// [`cargo_config2::Config::rustflags`] resolves Cargo's precedence
    /// (`CARGO_ENCODED_RUSTFLAGS`, `RUSTFLAGS`, the `target` table's
    /// `target.<triple>` / `CARGO_TARGET_<triple>_RUSTFLAGS` / matching
    /// `target.<cfg>` keys, or `build.rustflags`), evaluating `target.<cfg>`
    /// keys against `rustc --print cfg` for the toolchain this build runs
    /// under. The `--config` files Cargo also sees have no resolver channel,
    /// so they are laid down as a `.cargo/config.toml` chain the discovery
    /// walk reaches at `--config` depth.
    async fn user_rustflags(
        &self,
        cargo_config_files: &[PathBuf],
    ) -> Result<Vec<String>, RustBuildError> {
        let mut host = crate::toolchain::Host::current().with_cwd(&self.path);
        for (key, value) in &self.envs {
            host = host.with_env(key, value);
        }
        // Cargo's `$CARGO_HOME` resolution: absolute, anchored at the
        // working directory when relative, `~/.cargo` when unset.
        let cargo_home = host
            .env(std::ffi::OsStr::new("CARGO_HOME"))
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .map(|home| {
                if home.is_absolute() {
                    home
                } else {
                    self.path.join(home)
                }
            })
            .or_else(|| host.home_dir().map(|home| home.join(".cargo")));
        let crate_dir = if cargo_config_files.is_empty() {
            self.path.clone()
        } else {
            crate::toolchain::cargo_project_config::config_chain_dir(&self.path, cargo_config_files)
                .map_err(rustflags_resolution_error)?
        };
        let options = cargo_config2::ResolveOptions::default()
            .env(
                host.envs()
                    .map(|(k, v)| (k.to_os_string(), v.to_os_string())),
            )
            .rustc(self.toolchain_rustc().await?)
            .cargo_home(cargo_home);
        let config = cargo_config2::Config::load_with_options(&crate_dir, options)
            .map_err(rustflags_resolution_error)?;
        let flags = config
            .rustflags(self.triple.to_string().as_str())
            .map_err(rustflags_resolution_error)?;
        Ok(flags.map(|flags| flags.flags).unwrap_or_default())
    }

    /// `rustc` resolving to the toolchain the cargo invocation runs under:
    /// `rustup run` for the `-Zbuild-std` nightly or the project's toolchain,
    /// else the bare `rustc` shim — the same resolution cargo's own `rustc`
    /// applies in this directory.
    async fn toolchain_rustc(&self) -> Result<cargo_config2::PathAndArgs, RustBuildError> {
        let toolchain = if let Some(toolchain) = &self.build_std_toolchain {
            Some(toolchain.clone())
        } else if let Some(project) = &self.project {
            Some(
                project_toolchain(project)
                    .await
                    .map_err(rustflags_resolution_error)?,
            )
        } else {
            None
        };
        Ok(toolchain.map_or_else(
            || cargo_config2::PathAndArgs::new("rustc"),
            |toolchain| {
                let mut rustc = cargo_config2::PathAndArgs::new("rustup");
                rustc.args(["run", toolchain.as_str(), "rustc"]);
                rustc
            },
        ))
    }

    /// The union of the user's resolved rustflags and this build's own
    /// `rustc_flags`, handed to cargo through `CARGO_ENCODED_RUSTFLAGS`.
    /// `user_rustflags` is the caller's resolved set — the same value
    /// the artifact marker records, so it is computed once per build.
    ///
    /// Cargo resolves rustflags from mutually exclusive sources — an
    /// `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS` value discards every
    /// config-file flag, so the CLI's own flags cannot join the user's
    /// set that way. Instead resolve the union Cargo itself would pick
    /// and hand it back through `CARGO_ENCODED_RUSTFLAGS`: Cargo's own
    /// precedence decides what applies, the CLI's flags append on top,
    /// the encoded form survives spaces in either set, and the whole
    /// value lands in every unit fingerprint Cargo computes.
    fn apply_rustflags(
        &self,
        cmd: &mut Command,
        user_rustflags: &[String],
    ) -> Result<(), RustBuildError> {
        if self.rustc_flags.is_empty() {
            return Ok(());
        }
        let mut flags = cargo_config2::Flags::default();
        for flag in user_rustflags.iter().chain(&self.rustc_flags) {
            flags.push(flag.clone());
        }
        cmd.env(
            "CARGO_ENCODED_RUSTFLAGS",
            flags.encode().map_err(rustflags_resolution_error)?,
        );
        Ok(())
    }

    async fn prepare_framework_build(&self) -> Result<bool, RustBuildError> {
        let framework = self.project.as_ref().and_then(|project| {
            project
                .manifest()
                .framework
                .as_ref()
                .map(|framework| (project, framework))
        });
        if let Some((project, framework)) = framework {
            framework
                .prepare_build(project, &self.path, &self.features)
                .await
                .map_err(|error| {
                    RustBuildError::FailToBuildRustLibrary(std::io::Error::other(error.to_string()))
                })?;
        }
        Ok(framework.is_some())
    }

    /// Environment every cargo compilation gets before caller-provided
    /// `self.envs` (which therefore still overrides it): the managed tools'
    /// `PATH` entry, and the framework's deployment-target floor when
    /// `self.triple` names an Apple platform — the triple is the compilation
    /// target whether the build crosses or the host is the Apple target
    /// itself.
    fn apply_default_envs(&self, cmd: &mut Command) {
        with_managed_tools_path(cmd);
        if let Some((key, value)) =
            crate::apple::platform::apple_deployment_target_env(&self.triple)
        {
            cmd.env(key, value);
        }
    }

    async fn cargo_build_output(
        &self,
        release: bool,
        cargo_target: CargoTarget<'_>,
        user_rustflags: &[String],
    ) -> Result<std::process::Output, RustBuildError> {
        let framework = self.prepare_framework_build().await?;
        let crate_type_override = if cargo_target.accepts_crate_type_override() {
            self.crate_type_override.as_deref()
        } else {
            None
        };
        let mut cmd = Command::new("cargo");
        // A `--bin` unit always builds through `cargo rustc`: link flags for
        // the final crate (`final_rustc_args`) can then be passed after `--`,
        // where they reach only the selected unit's compilation and leave
        // dependency fingerprints identical across link variants.
        let cargo_subcommand = if crate_type_override.is_some()
            || !self.final_rustc_args.is_empty()
            || matches!(cargo_target, CargoTarget::Binary(_))
        {
            "rustc"
        } else {
            "build"
        };
        let mut cmd = cmd.arg(cargo_subcommand);
        if self.build_std_toolchain.is_some() {
            // `-Zbuild-std-features` replaces Cargo's default std feature set
            // — `panic-unwind,backtrace,default` (cargo's `standard_lib.rs`)
            // — so all three are listed back explicitly; `default` keeps each
            // std-workspace crate's own defaults, notably `compiler_builtins`'s
            // `arch` routines. `compiler-builtins-c` then links the NDK's
            // prebuilt compiler-rt archive — on aarch64 that provides the LSE
            // outline-atomics helpers (`__aarch64_ldadd4_acq_rel` & friends)
            // that NDK-compiled C objects reference, which otherwise stay
            // undefined and make `dlopen` reject the libraries.
            cmd = cmd.arg("-Zbuild-std=std,panic_abort");
            cmd =
                cmd.arg("-Zbuild-std-features=panic-unwind,backtrace,default,compiler-builtins-c");
        }
        let mut cmd = cmd
            .arg("--message-format=json-render-diagnostics")
            .args(cargo_target.cargo_args(crate_type_override))
            .args(["--target", self.triple.to_string().as_str()])
            .args(framework.then_some("--locked"))
            .current_dir(&self.path);

        // A managed crate builds outside the project, so Cargo's config
        // discovery never reaches `<project>/.cargo/config.toml`.
        let cargo_config_files = self.project_cargo_config_files()?;
        cmd = cmd.args(
            cargo_config_files
                .iter()
                .flat_map(|path| [OsString::from("--config"), path.clone().into_os_string()]),
        );

        if let Some(config) = self.debug_assertions_config_arg(release) {
            cmd = cmd.arg("--config").arg(config);
        }

        if let Some(target_dir) = &self.target_dir {
            cmd = cmd.arg("--target-dir").arg(target_dir);
        }
        self.apply_default_envs(cmd);
        // Apply extra environment variables (caller-provided values override defaults).
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }
        let mut cmd = self.with_project_toolchain_env(cmd).await?;

        self.apply_rustflags(cmd, user_rustflags)?;

        configure_generated_crate_compilation(cmd);

        // Use sccache as rustc wrapper if configured
        if let Some(sccache_path) = &self.sccache_path {
            crate::toolchain::sccache::configure_compilation_cache(cmd, sccache_path)
                .await
                .map_err(|error| {
                    RustBuildError::FailToBuildRustLibrary(std::io::Error::other(error.to_string()))
                })?;
        }

        // A `-Zbuild-std` build runs the `water` binary itself as
        // `RUSTC_WRAPPER`, chained in front of sccache when one is configured,
        // so the wrapper can add the `dylib` crate type Cargo strips from the
        // `std` unit and publish the produced `libstd-*.so` into `deps/`.
        // This must come after the sccache block above to win `RUSTC_WRAPPER`.
        if self.build_std_toolchain.is_some() {
            cmd = self.with_build_std_envs(cmd, release).await?;
        }

        // Set target-scoped bindgen clang args for simulator builds.
        //
        // Using the global `BINDGEN_EXTRA_CLANG_ARGS` leaks the simulator SDK into
        // host-side build scripts (for example `coreaudio-sys`), which then try to
        // parse host frameworks against the simulator SDK and fail. Bindgen supports
        // target-qualified env vars, so scope the override to the actual Cargo target.
        if self.triple.environment == Environment::Sim
            && let Some(clang_args) = self.bindgen_clang_args_for_simulator().await
        {
            let bindgen_target_key = format!(
                "BINDGEN_EXTRA_CLANG_ARGS_{}",
                self.triple.to_string().replace('-', "_")
            );
            cmd = cmd.env(bindgen_target_key, clang_args);
        }

        if release {
            cmd = cmd.arg("--release");
        }

        // Add cargo features if specified
        if !self.features.is_empty() {
            cmd = cmd.args(["--features", &self.features.join(",")]);
        }

        let trailing_args = self.trailing_rustc_args(release, cargo_target, user_rustflags);
        if !trailing_args.is_empty() {
            cmd = cmd.arg("--").args(trailing_args);
        }

        // Piped stdio strips rustc diagnostics of their colors; when the
        // terminal renders them — through the progress sink or the raw
        // passthrough echo — restore cargo's coloring unless the caller
        // configured it explicitly.
        if std_output_enabled()
            && std::env::var_os("CARGO_TERM_COLOR").is_none()
            && !self.envs.iter().any(|(key, _)| key == "CARGO_TERM_COLOR")
        {
            cmd.env("CARGO_TERM_COLOR", "always");
        }

        command_output_with_progress(cmd, self.progress.clone())
            .await
            .map_err(RustBuildError::FailToExecuteCargoBuild)
    }

    /// The `--config` argument re-enabling `debug_assertions` for the
    /// project's own package on a development-profile build whose profile
    /// disables them (Optimized).
    ///
    /// The framework gates its `waterui_meta_*` statics and
    /// `waterui_preview_*` exports on `debug_assertions` so release binaries
    /// stay free of them, but those symbols are how the CLI discovers mounts
    /// and previews — so the Optimized development profile would compile the
    /// project's own library artifact without them. Scoping the flag back on
    /// for the project package only leaves dependencies on the profile's
    /// setting; a release build keeps the gate.
    fn debug_assertions_config_arg(&self, release: bool) -> Option<String> {
        if release {
            return None;
        }
        self.project.as_ref().map(|project| {
            format!(
                "profile.dev.package.{}.debug-assertions=true",
                project.crate_name()
            )
        })
    }

    /// The arguments after `cargo rustc --`: this build's trailing rustc
    /// arguments — a `--crate-type` override on the library target, any
    /// `final_rustc_args`, and for a `--bin` unit the build's marker as a
    /// `--cfg`. The marker keeps each build variant its own Cargo unit:
    /// Cargo 1.99's unit metadata covers features, rustflags and trailing
    /// arguments, but not the manifest path or the process environment —
    /// a same-named crate in a second directory reports `fresh` on its
    /// first build and its `executable` names the first crate's bytes.
    /// The marker must not rename or redirect the artifact: Cargo's
    /// freshness check reads the output at `deps/<name>-<unit metadata>`,
    /// so `-Cextra-filename` or `--emit link` leave that file missing
    /// (`FailedToReadMetadata`) and the unit dirty on every run (#2073).
    fn trailing_rustc_args(
        &self,
        release: bool,
        cargo_target: CargoTarget<'_>,
        user_rustflags: &[String],
    ) -> Vec<String> {
        let mut args = Vec::new();
        // A `--crate-type` override only has meaning for the library target;
        // on a `--bin` unit it would fight rustc's own `bin` crate type.
        if cargo_target.accepts_crate_type_override()
            && let Some(crate_type) = &self.crate_type_override
        {
            args.push("--crate-type".to_owned());
            args.push(crate_type.clone());
        }
        args.extend(self.final_rustc_args.iter().cloned());
        if let CargoTarget::Binary(_) = cargo_target {
            let marker = self.artifact_marker(release, cargo_target, user_rustflags);
            args.extend(["--cfg".to_owned(), format!("water_build_marker_{marker}")]);
        }
        args
    }

    /// Run cargo under the rustup toolchain the project's own directory
    /// selects, so a crate generated outside the project tree (the build
    /// cache) compiles with the same toolchain as the project instead of
    /// rustup's default for that directory. A `-Zbuild-std` build names its
    /// own nightly through [`Self::with_build_std_envs`] instead.
    async fn with_project_toolchain_env<'a>(
        &self,
        cmd: &'a mut Command,
    ) -> Result<&'a mut Command, RustBuildError> {
        if self.build_std_toolchain.is_some() {
            return Ok(cmd);
        }
        let Some(project) = &self.project else {
            return Ok(cmd);
        };
        let toolchain = project_toolchain(project).await.map_err(|error| {
            RustBuildError::FailToBuildRustLibrary(std::io::Error::other(error.to_string()))
        })?;
        Ok(cmd.env("RUSTUP_TOOLCHAIN", toolchain))
    }

    /// Point a `-Zbuild-std` cargo invocation at the nightly toolchain and at
    /// this binary as `RUSTC_WRAPPER`, chained in front of sccache when one is
    /// configured.
    async fn with_build_std_envs<'a>(
        &self,
        cmd: &'a mut Command,
        release: bool,
    ) -> Result<&'a mut Command, RustBuildError> {
        let Some(toolchain) = &self.build_std_toolchain else {
            return Ok(cmd);
        };
        let publish_dir = self.lib_output_dir(release).await?.join("deps");
        let cmd = cmd
            .env("RUSTUP_TOOLCHAIN", toolchain)
            .env(
                "RUSTC_WRAPPER",
                crate::toolchain::Host::current_exe()
                    .map_err(RustBuildError::FailToExecuteCargoBuild)?,
            )
            .env(crate::workflows::rustc_wrapper::WRAPPER_MODE_ENV, "1")
            .env(
                crate::workflows::rustc_wrapper::BUILD_STD_TARGET_ENV,
                self.triple.to_string(),
            )
            .env(
                crate::workflows::rustc_wrapper::BUILD_STD_DYLIB_DIR_ENV,
                publish_dir,
            );
        if let Some(sccache_path) = &self.sccache_path {
            cmd.env(
                crate::workflows::rustc_wrapper::WRAPPER_CHAIN_ENV,
                sccache_path,
            );
        }
        // A workspace wrapper replaces `RUSTC_WRAPPER` on workspace-member
        // units — the support app's ffi crate and the generated module crate
        // are exactly the link-emitting members that need the `std` dylib
        // extern. Without it they would link `std` statically while the deps
        // link dynamically: two panic runtimes in one process.
        cmd.env_remove("RUSTC_WORKSPACE_WRAPPER");
        cmd.env_remove("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER");
        Ok(cmd)
    }

    /// Resolve the Cargo library artifact directory for this build target and profile.
    ///
    /// # Errors
    /// Returns an error if Cargo metadata cannot be read for this build target.
    pub async fn lib_output_dir(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        let target_directory = self.target_directory().await?;
        Ok(target_directory
            .join(self.triple.to_string())
            .join(if release { "release" } else { "debug" }))
    }

    async fn target_directory(&self) -> Result<PathBuf, RustBuildError> {
        if let Some(target_dir) = &self.target_dir {
            return Ok(target_dir.clone());
        }

        let build_path = self.path.clone();
        let metadata = unblock(move || {
            cargo_metadata::MetadataCommand::new()
                .no_deps()
                .current_dir(build_path)
                .exec()
                .map_err(|e| {
                    RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e,
                    ))
                })
        })
        .await?;
        Ok(metadata.target_directory.as_std_path().to_path_buf())
    }

    /// Take a shared lease on the shared Cargo target when this build
    /// compiles into it (`target/shared`, `target/toolchain-*` and the root
    /// itself are all inside it), so the build-cache garbage collector
    /// cannot drop the tree while Cargo runs. The returned file is the
    /// guard: holding it holds the lease.
    async fn shared_target_lease(&self) -> Result<Option<std::fs::File>, RustBuildError> {
        let Some(target_dir) = &self.target_dir else {
            return Ok(None);
        };
        // A pure path lookup decides containment: a build whose target lives
        // outside the shared root must not touch `~/.water` at all — the
        // ensure would stamp the shared target's metadata for a build that
        // never enters it, and that write races other processes on Windows.
        let shared_root = crate::water_dir::shared_target_dir_path()
            .await
            .map_err(|error| {
                RustBuildError::FailToBuildRustLibrary(std::io::Error::other(format!(
                    "Could not resolve the shared Cargo target: {error}"
                )))
            })?;
        if !target_dir.starts_with(&shared_root) {
            return Ok(None);
        }
        let shared_root = crate::water_dir::shared_target_dir()
            .await
            .map_err(|error| {
                RustBuildError::FailToBuildRustLibrary(std::io::Error::other(format!(
                    "Could not resolve the shared Cargo target: {error}"
                )))
            })?;
        crate::water_dir::lease_shared_target_dir(&shared_root)
            .await
            .map(Some)
            .map_err(|error| {
                RustBuildError::FailToBuildRustLibrary(std::io::Error::other(format!(
                    "Could not lease the shared Cargo target {}: {error}",
                    shared_root.display()
                )))
            })
    }

    /// The `--cfg` value distinguishing one build variant's `--bin` unit:
    /// a hash of the crate directory, the target and the profile, the
    /// binary's name, and every compiler-shaping option this build carries
    /// — features, the resolved rustflags set, environment overrides and
    /// the trailing `cargo rustc` arguments. Two builds of the same crate
    /// that differ in any of those never share the marker, and neither do
    /// two same-named crates in different directories: Cargo 1.99's unit
    /// metadata covers features, rustflags and trailing rustc arguments,
    /// but not the manifest path or the process environment — on this
    /// toolchain a same-named crate in a second directory reports `fresh`
    /// on its first build, its `executable` naming the first crate's
    /// bytes. The marker folds those unhashed inputs into the unit so
    /// each variant compiles, fingerprints and uplifts separately, and it
    /// names the variant-stable `deps/<name>-<marker>` path
    /// [`marked_binary_artifact`] relinks to.
    fn artifact_marker(
        &self,
        release: bool,
        cargo_target: CargoTarget<'_>,
        user_rustflags: &[String],
    ) -> String {
        use sha2::Digest as _;
        let mut signature = Vec::new();
        let mut feed = |bytes: &[u8]| {
            signature.extend_from_slice(bytes);
            signature.push(0);
        };
        feed(self.path.as_os_str().as_encoded_bytes());
        feed(self.triple.to_string().as_bytes());
        feed(&[u8::from(release)]);
        if let CargoTarget::Binary(name) = cargo_target {
            feed(name.as_bytes());
        }
        if let Some(crate_type) = &self.crate_type_override {
            feed(crate_type.as_bytes());
        }
        for feature in &self.features {
            feed(feature.as_bytes());
        }
        // The rustflags rustc receives, in application order: the user's
        // resolved set first, then this build's own flags appended.
        for flag in user_rustflags {
            feed(flag.as_bytes());
        }
        for flag in &self.rustc_flags {
            feed(flag.as_bytes());
        }
        for arg in &self.final_rustc_args {
            feed(arg.as_bytes());
        }
        for (key, value) in &self.envs {
            feed(key.as_bytes());
            feed(value.as_encoded_bytes());
        }
        let digest = sha2::Sha256::digest(&signature);
        hex::encode(&digest[..4])
    }

    /// Generate `BINDGEN_EXTRA_CLANG_ARGS` for simulator builds.
    ///
    /// Bindgen has issues with the `*-apple-*-sim` target triples, so we need to
    /// provide explicit clang arguments with a proper target and SDK path.
    async fn bindgen_clang_args_for_simulator(&self) -> Option<String> {
        let (sdk_name, target_os) = match self.triple.operating_system {
            OperatingSystem::IOS(_) => ("iphonesimulator", "ios"),
            OperatingSystem::TvOS(_) => ("appletvsimulator", "tvos"),
            OperatingSystem::WatchOS(_) => ("watchsimulator", "watchos"),
            OperatingSystem::VisionOS(_) => ("xrsimulator", "xros"),
            _ => return None,
        };

        let arch = match self.triple.architecture {
            target_lexicon::Architecture::Aarch64(_) => "arm64",
            target_lexicon::Architecture::X86_64 => "x86_64",
            _ => return None,
        };

        // Get SDK path using xcrun
        let sdk_path = run_command("xcrun", ["--sdk", sdk_name, "--show-sdk-path"])
            .await
            .ok()
            .map(|s| s.trim().to_string())?;

        // Use a reasonable minimum deployment target
        let min_version = if matches!(target_os, "ios" | "tvos") {
            "17.0"
        } else if target_os == "watchos" {
            "10.0"
        } else {
            debug_assert_eq!(
                target_os, "xros",
                "bindgen simulator target_os must be one of ios/tvos/watchos/xros"
            );
            "1.0"
        };

        Some(format!(
            "--target={arch}-apple-{target_os}{min_version}-simulator -isysroot {sdk_path}"
        ))
    }
}

/// The file extension the produced artifact carries for a `--crate-type`
/// value — `None` for a type with no single known file shape.
fn crate_type_artifact_extension(crate_type: &str, triple: &Triple) -> Option<&'static str> {
    match crate_type {
        "lib" | "rlib" => Some("rlib"),
        "staticlib" => Some(if matches!(triple.environment, Environment::Msvc) {
            "lib"
        } else {
            "a"
        }),
        "cdylib" | "dylib" | "proc-macro" => Some(lib_extension_for_triple(triple)),
        _ => None,
    }
}

/// The final artifact Cargo reported for the selected target: the
/// `compiler-artifact` message for `crate_dir`'s manifest, matched by target
/// kind — Cargo's own report of what it wrote, never a name reconstructed
/// under the profile directory.
///
/// Every generated crate builds into one shared per-user Cargo target, so
/// `<profile>/<name>` alone is not evidence the file came from this build.
/// `artifact_extension` disambiguates a library target that emitted several
/// crate types; without one, the build reports exactly one file or this
/// fails rather than guesses.
///
/// # Errors
/// Returns an error when no `compiler-artifact` message for the selected
/// target reports a matching file, or the reported file does not exist.
pub(crate) fn reported_artifact(
    stdout: &[u8],
    crate_dir: &Path,
    cargo_target: CargoTarget<'_>,
    artifact_extension: Option<&'static str>,
) -> Result<PathBuf, RustBuildError> {
    let manifest_path = canonical_manifest_path(crate_dir)?;
    let mut artifacts = Vec::new();
    for artifact in compiler_artifacts(stdout)? {
        if cargo_target.matches(&artifact.target)
            && same_manifest_path(artifact.manifest_path.as_std_path(), &manifest_path)
        {
            artifacts.push(artifact);
        }
    }
    reported_artifact_file(&artifacts, cargo_target, artifact_extension, &manifest_path)
}

/// An exclusive lock on the unhashed `<profile>/<name>` uplift a `--bin`
/// build shares with every other same-named binary unit in this target
/// directory.
///
/// `shared_target_lease` is a *shared* lease — concurrent `water` builds
/// into this target directory are expected — so this lock file takes fs4's
/// exclusive lock from before Cargo's invocation, through the reported
/// executable's relink by [`marked_binary_artifact`], and — under the
/// default [`ArtifactLockScope`] — on inside the returned
/// [`SharedExecutable`] until the launcher that executes the uplift has
/// spawned its child. A second same-named build then uplifts
/// `<profile>/<name>` only after this build's launch read its bytes, and
/// its own `executable` report names its own output. The lock is per
/// binary name: unrelated builds proceed in parallel.
///
/// A contended wait announces which artifact it waits on before blocking:
/// fs4's lock has no timeout, so the message is the only sign the build is
/// queued behind another `water` process rather than compiling.
async fn binary_artifact_lock(
    profile_dir: &Path,
    binary_name: &str,
) -> Result<ArtifactLock, RustBuildError> {
    let lock_path = profile_dir.join(format!(".water-artifact-{binary_name}.lock"));
    let profile_dir = profile_dir.to_path_buf();
    let binary_name = binary_name.to_owned();
    smol::unblock(move || {
        std::fs::create_dir_all(&profile_dir).map_err(RustBuildError::FailToBuildRustLibrary)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(RustBuildError::FailToBuildRustLibrary)?;
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(fs4::TryLockError::WouldBlock) => {
                info!(
                    artifact = binary_name.as_str(),
                    lock = %lock_path.display(),
                    "waiting for the binary artifact lock"
                );
                fs4::FileExt::lock(&file).map_err(RustBuildError::FailToBuildRustLibrary)?;
            }
            Err(fs4::TryLockError::Error(error)) => {
                return Err(RustBuildError::FailToBuildRustLibrary(error));
            }
        }
        Ok(ArtifactLock { _file: file })
    })
    .await
}

/// The variant-stable name of this `--bin` build's artifact: a hard link
/// at `deps/<name>-<marker>` to the unhashed `<profile>/<name>` executable
/// Cargo reported for this run.
///
/// The link — not a copy — must be made while [`binary_artifact_lock`] is
/// held: the uplift name is shared, so a same-named build could re-uplift
/// different bytes over it once the lock is gone. The marked name is
/// stable afterwards: only a build hashing to the same marker rewrites
/// it, and such a build produces the same unit's output. The link goes
/// through a same-directory temporary plus `rename`, so a concurrent
/// reader of the marked path sees either the previous or the new inode —
/// never a partially written one. The hard link is required, with no copy
/// fallback: a copy would publish an `artifact` name that stops tracking
/// the unit's output, so a filesystem that cannot hard-link inside
/// `deps/` fails the build rather than silently break that tracking.
async fn marked_binary_artifact(
    uplift: &Path,
    profile_dir: &Path,
    binary_name: &str,
    marker: &str,
    triple: &Triple,
) -> Result<PathBuf, RustBuildError> {
    let deps_dir = profile_dir.join("deps");
    let suffix = executable_suffix(triple);
    let marked_path = deps_dir.join(format!("{binary_name}-{marker}{suffix}"));
    let staging = deps_dir.join(format!(".{binary_name}-{marker}{suffix}.tmp"));
    smol::fs::create_dir_all(&deps_dir)
        .await
        .map_err(RustBuildError::FailToBuildRustLibrary)?;

    // `rename` may do nothing when source and destination are links to
    // the same file — macOS takes that no-op branch — which would leave
    // the staging name behind on a same-inode relink. `same_file` catches
    // a marked path still linked to the uplift's inode; an uplift Cargo
    // rewrote with identical output keeps the mtime it stamped, so the
    // size+mtime check reads it as already naming this run's output
    // without reading either binary, and keeping the marked path preserves
    // the variant's inode for whatever watches `deps/` (#2073).
    let marked_bytes = marked_path.clone();
    let uplift_bytes = uplift.to_path_buf();
    if same_file(uplift, &marked_path).await?
        || smol::unblock(move || {
            crate::utils::cargo_output_unmodified(&uplift_bytes, &marked_bytes)
        })
        .await
        .map_err(RustBuildError::FailToBuildRustLibrary)?
    {
        remove_staging_link(&staging).await?;
        return Ok(marked_path);
    }

    remove_staging_link(&staging).await?;
    smol::fs::hard_link(uplift, &staging)
        .await
        .map_err(RustBuildError::FailToBuildRustLibrary)?;
    smol::fs::rename(&staging, &marked_path)
        .await
        .map_err(RustBuildError::FailToBuildRustLibrary)?;
    Ok(marked_path)
}

/// Remove `marked_binary_artifact`'s staging name if a previous run left
/// one behind.
async fn remove_staging_link(staging: &Path) -> Result<(), RustBuildError> {
    match smol::fs::remove_file(staging).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RustBuildError::FailToBuildRustLibrary(error)),
    }
}

/// Whether `a` and `b` name the same file — `false` when `b` does not
/// exist, an error for any other stat failure.
async fn same_file(a: &Path, b: &Path) -> Result<bool, RustBuildError> {
    let a = a.to_path_buf();
    let b = b.to_path_buf();
    smol::unblock(move || {
        let b_handle = match same_file::Handle::from_path(&b) {
            Ok(handle) => handle,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(RustBuildError::FailToBuildRustLibrary(error)),
        };
        let a_handle =
            same_file::Handle::from_path(&a).map_err(RustBuildError::FailToBuildRustLibrary)?;
        Ok(a_handle == b_handle)
    })
    .await
}

/// The canonicalized `Cargo.toml` path of `crate_dir` — the spelling
/// `compiler-artifact` messages report `manifest_path` in.
fn canonical_manifest_path(crate_dir: &Path) -> Result<PathBuf, RustBuildError> {
    dunce::canonicalize(crate_dir.join("Cargo.toml")).map_err(|error| {
        RustBuildError::FailToBuildRustLibrary(io::Error::other(format!(
            "failed to canonicalize {}: {error}",
            crate_dir.join("Cargo.toml").display()
        )))
    })
}

/// Every `compiler-artifact` message in a cargo `--message-format=json`
/// stdout stream.
///
/// Cargo's report is the only record of what a build wrote, so a line naming
/// itself `compiler-artifact` that does not deserialize is a hard error
/// carrying the line — silently dropping it degrades into a misleading "no
/// artifact reported" failure downstream. Messages with any other `reason`,
/// and lines that are not cargo messages at all, are ignored.
pub(crate) fn compiler_artifacts(
    stdout: &[u8],
) -> Result<Vec<cargo_metadata::Artifact>, RustBuildError> {
    /// The one field that classifies a cargo message line.
    #[derive(serde::Deserialize)]
    struct Reason {
        reason: String,
    }

    let mut artifacts = Vec::new();
    for (index, line) in stdout.split(|byte| *byte == b'\n').enumerate() {
        let Ok(line) = str::from_utf8(line) else {
            continue;
        };
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let malformed = |error: serde_json::Error| {
            RustBuildError::FailToBuildRustLibrary(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "cargo emitted a malformed `compiler-artifact` message on line {}: {error}\n{line}",
                    index + 1
                ),
            ))
        };
        match serde_json::from_str::<Reason>(line) {
            Ok(Reason { reason }) if reason == "compiler-artifact" => {
                let artifact =
                    serde_json::from_str::<cargo_metadata::Artifact>(line).map_err(malformed)?;
                artifacts.push(artifact);
            }
            // A line that is not readable JSON cannot yield its `reason`
            // field; one that still names itself a `compiler-artifact`
            // carries an unreadable payload — the hard error, never a drop.
            Err(error) if line.contains("\"reason\":\"compiler-artifact\"") => {
                return Err(malformed(error));
            }
            Ok(_) | Err(_) => {}
        }
    }
    Ok(artifacts)
}

fn reported_shared_runtime(stdout: &[u8]) -> Result<Option<PathBuf>, RustBuildError> {
    let mut reported = Vec::new();
    for artifact in compiler_artifacts(stdout)? {
        if artifact_package_name(&artifact.package_id) != "waterui-dylib"
            || !artifact
                .target
                .kind
                .contains(&cargo_metadata::TargetKind::DyLib)
        {
            continue;
        }
        for filename in &artifact.filenames {
            let path = filename.as_std_path();
            if is_dynamic_library(path) {
                reported.push((path.to_path_buf(), artifact.manifest_path.clone()));
            }
        }
    }
    match reported.as_slice() {
        [] => Ok(None),
        [(path, _)] => Ok(Some(path.clone())),
        _ => Err(RustBuildError::FailToBuildRustLibrary(io::Error::other(
            format!(
                "Cargo reported multiple `waterui-dylib` dynamic libraries: {}",
                reported
                    .iter()
                    .map(|(_, manifest)| manifest.as_std_path().display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ))),
    }
}

/// The library artifact `stdout` reports for the crate at `manifest_path` —
/// the `deps/` rlib, staticlib, or dylib whose symbol table
/// `crate::artifact_symbols::ArtifactSymbols` reads.
///
/// The selected build's own `compiler-artifact` stream is the source, so the
/// path is what this invocation's rustc wrote — never a `<profile>`-glob that
/// could name bytes an older configuration left behind. Several library
/// filenames can accompany one artifact (a crate-type list emits an rlib
/// beside a staticlib); the most parseable rank wins — `.rlib`, then a
/// static archive, then a dynamic library — and two equally ranked reports
/// are a hard error rather than a guess.
///
/// # Errors
/// Returns an error when cargo's message stream cannot be parsed or reports
/// two indistinguishable library artifacts for the crate.
pub(crate) fn app_library_artifact(
    stdout: &[u8],
    manifest_path: &Path,
) -> Result<Option<PathBuf>, RustBuildError> {
    let mut candidates: BTreeMap<u8, BTreeSet<PathBuf>> = BTreeMap::new();
    for artifact in compiler_artifacts(stdout)? {
        if !artifact.target.kind.iter().any(|kind| {
            matches!(
                kind,
                cargo_metadata::TargetKind::Lib
                    | cargo_metadata::TargetKind::RLib
                    | cargo_metadata::TargetKind::DyLib
                    | cargo_metadata::TargetKind::CDyLib
                    | cargo_metadata::TargetKind::StaticLib
            )
        }) || !same_manifest_path(artifact.manifest_path.as_std_path(), manifest_path)
        {
            continue;
        }
        for filename in &artifact.filenames {
            let path = filename.as_std_path();
            if let Some(rank) = app_library_extension_rank(path) {
                candidates
                    .entry(rank)
                    .or_default()
                    .insert(path.to_path_buf());
            }
        }
    }
    let Some((_, mut best)) = candidates.into_iter().next() else {
        return Ok(None);
    };
    if best.len() == 1 {
        return Ok(best.pop_first());
    }
    Err(RustBuildError::FailToBuildRustLibrary(io::Error::other(
        format!(
            "Cargo reported multiple library artifacts for {}: {}",
            manifest_path.display(),
            best.iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )))
}

/// The readability rank of a library filename, or `None` for a file that is
/// not a library artifact (`.rmeta`, executables, `.d` dep-info). `.rlib` and
/// `.a` archives read as full object collections; `.so`/`.dylib`/`.dll` are
/// single objects; a bare `.lib`/`foo.dll.lib` can also be a linker import
/// stub beside a `.dll`, so it ranks last.
fn app_library_extension_rank(path: &Path) -> Option<u8> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rlib" | "wasm" => Some(0),
        "a" => Some(1),
        "so" | "dylib" | "dll" => Some(2),
        "lib" => Some(3),
        _ => None,
    }
}

/// Whether a `manifest_path` cargo reported is `expected`, the manifest of
/// the crate this build ran. Cargo reports the path in the spelling its own
/// working directory carried — a verbatim `\\?\` or an 8.3 short-name root on
/// Windows — so a lexical miss canonicalizes the reported path (it exists;
/// cargo just built from it) before deciding.
pub(crate) fn same_manifest_path(reported: &Path, expected: &Path) -> bool {
    reported == expected
        || dunce::canonicalize(reported).is_ok_and(|canonical| canonical == expected)
}

/// Picks the single file the selected target emitted out of its collected
/// `compiler-artifact` messages.
fn reported_artifact_file(
    artifacts: &[cargo_metadata::Artifact],
    cargo_target: CargoTarget<'_>,
    artifact_extension: Option<&'static str>,
    manifest_path: &Path,
) -> Result<PathBuf, RustBuildError> {
    let what = || -> String {
        match cargo_target {
            CargoTarget::Lib => format!("the library target of {}", manifest_path.display()),
            CargoTarget::Binary(name) => {
                format!("binary `{name}` of {}", manifest_path.display())
            }
        }
    };
    let not_found = |detail: String| {
        RustBuildError::FailToBuildRustLibrary(io::Error::new(io::ErrorKind::NotFound, detail))
    };

    let files: Vec<PathBuf> = artifacts
        .iter()
        .flat_map(|artifact| {
            artifact
                .filenames
                .iter()
                .map(|file| file.as_std_path().to_path_buf())
        })
        .collect();
    let artifact = match cargo_target {
        // Cargo re-uplifts the selected `--bin` unit to the unhashed
        // `<profile>/<name>` on every run — `fresh` included — so
        // `executable` names this run's own bytes. That only holds
        // because the marker `--cfg` keeps every variant its own unit:
        // without it a same-named crate at a different path reports
        // `fresh` against the sibling's unit and `executable` names the
        // sibling's bytes.
        CargoTarget::Binary(_) => artifacts
            .iter()
            .find_map(|artifact| artifact.executable.as_ref())
            .map(|path| path.as_std_path().to_path_buf())
            .ok_or_else(|| {
                not_found(format!(
                    "Cargo reported no artifact for {} (reported files: {files:?})",
                    what()
                ))
            })?,
        CargoTarget::Lib => {
            let matching: Vec<&PathBuf> = artifact_extension.map_or_else(
                || files.iter().collect(),
                |extension| {
                    files
                        .iter()
                        .filter(|file| file.extension().is_some_and(|e| *e == *extension))
                        .collect()
                },
            );
            match matching.as_slice() {
                [only] => (*only).clone(),
                _ => {
                    return Err(not_found(artifact_extension.map_or_else(
                        || {
                            format!(
                                "Cargo reported {} artifacts for {} — select one with a crate-type override (reported files: {files:?})",
                                matching.len(),
                                what()
                            )
                        },
                        |extension| {
                            format!(
                                "Cargo reported no `.{extension}` artifact for {} (reported files: {files:?})",
                                what()
                            )
                        },
                    )));
                }
            }
        }
    };
    if !artifact.is_file() {
        return Err(not_found(format!(
            "Cargo reported {} for {} but the file does not exist",
            artifact.display(),
            what()
        )));
    }
    Ok(artifact)
}

/// Why a `fresh` shared dylib unit cannot be trusted as this project's own.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StaleSharedDylibReason {
    /// The dep-info beside the artifact names no source under the unit's
    /// manifest root: another source's build of the same-named package wrote
    /// the file.
    ForeignDepInfo { dep_info: PathBuf },
    /// No dep-info exists beside the artifact or in its unit directory, so
    /// nothing records which sources produced the bytes on disk.
    MissingDepInfo { reported_files: Vec<PathBuf> },
}

impl std::fmt::Display for StaleSharedDylibReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ForeignDepInfo { dep_info } => write!(
                f,
                "its dep-info {} names no source under this unit's manifest root, so another source's build wrote it",
                dep_info.display()
            ),
            Self::MissingDepInfo { reported_files } => write!(
                f,
                "no dep-info was found beside it or in its unit directory, so nothing records which sources produced it (reported files: {reported_files:?})"
            ),
        }
    }
}

/// A `fresh` shared dylib unit whose artifact this build must not trust, and
/// the package whose units are cleaned so the rebuild emits its own.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StaleSharedDylib {
    package: String,
    artifact: PathBuf,
    reason: StaleSharedDylibReason,
}

/// The error a build reports when a cleaned and rebuilt package still comes
/// back `fresh` in a state this CLI cannot trust: rebuilding once did not
/// repair the cache, so it names the target directory to drop rather than
/// rebuilding again.
fn unrecoverable_shared_dylib_error(
    stale: &[StaleSharedDylib],
    target_dir: &Path,
) -> RustBuildError {
    let units = stale.iter().fold(String::new(), |mut units, unit| {
        let _ = std::fmt::Write::write_fmt(
            &mut units,
            format_args!(
                "\n  - {} ({}): {}",
                unit.artifact.display(),
                unit.package,
                unit.reason
            ),
        );
        units
    });
    let message = format!(
        "Cargo still reports a shared dylib unit as fresh after its package was cleaned and rebuilt:{units}\nThe shared Cargo target directory {} cannot be repaired by rebuilding; remove it with `water gc build-cache --shared-target` and build again.",
        target_dir.display()
    );
    RustBuildError::FailToBuildRustLibrary(io::Error::other(message))
}

/// Dependency packages whose `fresh` dynamic-library unit reports an artifact
/// this build did not verifiably write, with the reason for each.
///
/// Dep-info is the one record that names the producing sources: the `.d`
/// Cargo writes beside an uplifted dylib lists the writer's inputs, while the
/// unit's own `manifest_path` says which source *this* graph resolved. A
/// dep-info that names no file under the unit's manifest root was produced by
/// a different source's build, and the unhashed artifact it accompanies does
/// not belong to this project. An uplifted dylib with no dep-info at all is
/// a cache state this CLI's own builds leave behind (observed on Windows),
/// and it is flagged the same way so the caller rebuilds the package instead
/// of trusting bytes nothing accounts for.
///
/// `needed` is the selected artifact's recorded shared-library names — the
/// same records [`RustDynamicLibraries::resolve`] stages under — empty when
/// the artifact is not a linked image; see [`dep_info_path`].
async fn stale_shared_dylib_packages(
    stdout: &[u8],
    needed: &[String],
) -> Result<Vec<StaleSharedDylib>, RustBuildError> {
    let mut stale: Vec<StaleSharedDylib> = Vec::new();
    for artifact in compiler_artifacts(stdout)? {
        if !artifact.fresh {
            continue;
        }
        let Some(manifest_dir) = artifact.manifest_path.as_std_path().parent() else {
            continue;
        };
        // Only a `dylib`/`cdylib` unit uplifts to an unhashed, shareable
        // filename. A proc-macro's dylib keeps its metadata hash — the hash
        // covers the package id, so two sources never meet — and cargo's
        // build-dir layout stores it where no dep-info convention below
        // applies.
        if !uplifts_dynamic_library(&artifact.target) {
            continue;
        }
        let manifest_root = dunce::simplified(manifest_dir);
        let package = artifact_package_name(&artifact.package_id);
        let mut package_stale = None;
        for filename in &artifact.filenames {
            let file = filename.as_std_path();
            if !is_dynamic_library(file) {
                continue;
            }
            let Some(dep_info) = dep_info_path(file, needed, &artifact.filenames) else {
                package_stale = Some(StaleSharedDylib {
                    package: package.to_owned(),
                    artifact: file.to_path_buf(),
                    reason: StaleSharedDylibReason::MissingDepInfo {
                        reported_files: artifact
                            .filenames
                            .iter()
                            .map(|reported| reported.as_std_path().to_path_buf())
                            .collect(),
                    },
                });
                break;
            };
            let contents = smol::fs::read_to_string(&dep_info).await.map_err(|error| {
                RustBuildError::FailToBuildRustLibrary(io::Error::other(format!(
                    "Cargo reported {} fresh but its dep-info {} is unreadable: {error}",
                    file.display(),
                    dep_info.display()
                )))
            })?;
            // A dep-info that names no prerequisite under this unit's own
            // manifest root was written by a different source's build; a rare
            // miss costs one package rebuild — never a wrong artifact.
            if !dep_info_prerequisites(&contents).iter().any(|source| {
                let source = if source.is_absolute() {
                    source.clone()
                } else {
                    manifest_dir.join(source)
                };
                dunce::simplified(&source).starts_with(manifest_root)
            }) {
                package_stale = Some(StaleSharedDylib {
                    package: package.to_owned(),
                    artifact: file.to_path_buf(),
                    reason: StaleSharedDylibReason::ForeignDepInfo { dep_info },
                });
                break;
            }
        }
        if let Some(unit) = package_stale
            && !stale.iter().any(|known| known.package == unit.package)
        {
            stale.push(unit);
        }
    }
    stale.sort_unstable_by(|left, right| left.package.cmp(&right.package));
    Ok(stale)
}

/// Whether `file` names a dynamically linked library — the artifact shape a
/// dependency's final target uplifts to one unhashed filename per name.
fn is_dynamic_library(file: &Path) -> bool {
    file.extension()
        .is_some_and(|extension| matches!(extension.to_str(), Some("so" | "dylib" | "dll")))
}

/// Whether the unit's final artifact is a dynamic library cargo uplifts to
/// an unhashed filename: a `dylib` or `cdylib` crate type. Proc-macro
/// crates are dynamic libraries too, but stay hashed and are never shared.
fn uplifts_dynamic_library(target: &cargo_metadata::Target) -> bool {
    target.crate_types.iter().any(|kind| {
        matches!(
            kind,
            cargo_metadata::CrateType::DyLib | cargo_metadata::CrateType::CDyLib
        )
    })
}

/// The dep-info `.d` cargo wrote for the unit that produced `artifact_file`.
///
/// The name a linked consumer records for a needed library already carries
/// the `-C metadata` hash the dep-info is named for: a git- or
/// registry-sourced `dylib` unit compiles as `deps/lib<crate>-<metadata>.so`,
/// the artifact's `DT_NEEDED` (PE import / Mach-O `LC_LOAD_DYLIB`) records
/// that hashed name, and rustc writes the dep-info as
/// `deps/<crate>-<metadata>.d` — the recorded name minus its platform `lib`
/// prefix and shared-library suffix. `needed` is the selected artifact's own
/// record, so `deps/<stem>.d` names one exact file — never a
/// `deps/<crate>-*.d` enumeration, which several coexisting metadata hashes
/// could collide in (water-rs/cli#184).
///
/// When no linked artifact records the unit — the build's selected artifact
/// is a static archive, which carries no dynamic section — the dylib's own
/// `DT_SONAME` / `LC_ID_DYLIB` supplies the same string, since a consumer
/// simply re-records it.
///
/// The candidates after those are Cargo's documented spellings for a unit
/// whose names carry no metadata hash: the uplifted `<profile>/lib<name>.d`,
/// the stable `deps/<name>.d`, the unit directory a sibling output names
/// under the build-dir layout (a `<name>.d` beside the `.rmeta`/`out/` dir),
/// and a bare `<name>.d` beside the artifact.
fn dep_info_path(
    artifact_file: &Path,
    needed: &[String],
    sibling_files: &[cargo_metadata::camino::Utf8PathBuf],
) -> Option<PathBuf> {
    let file_stem = artifact_file.file_stem()?.to_str()?;
    let name = file_stem.strip_prefix("lib").unwrap_or(file_stem);
    let dir = artifact_file.parent()?;
    let deps = dir.join("deps");
    // The recorded name — the consumer's record first, then the library's
    // own when nothing linked it — names `deps/<stem>.d` exactly.
    let recorded = needed
        .iter()
        .find(|needed_name| needed_library_matches(needed_name, name))
        .cloned()
        .or_else(|| recorded_library_name(artifact_file));
    if let Some(dep_info) = recorded
        .and_then(|recorded_name| recorded_dep_info(&deps, &recorded_name))
        .filter(|candidate| candidate.is_file())
    {
        return Some(dep_info);
    }
    // The hashed dep-info cargo writes beside the hashed `deps/` copy —
    // `deps/lib<name>-<meta>.dylib` pairs with `deps/<name>-<meta>.d` — a
    // spelling neither the unhashed uplift's stem nor a retargeted `@rpath`
    // install record derives (water-rs/cli#197). Cargo's report never names
    // the hashed dylib, so read the directory rather than `sibling_files`.
    let base = name.split('-').next().unwrap_or(name);
    if let Ok(entries) = std::fs::read_dir(&deps) {
        let mut hashed = entries
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let file_name = entry.file_name();
                let file_name = file_name.to_str()?;
                let stem = file_name.strip_suffix(".d")?;
                if stem.starts_with(&format!("{base}-")) {
                    Some(entry.path())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        hashed.sort();
        if let Some(dep_info) = hashed.into_iter().find(|candidate| candidate.is_file()) {
            return Some(dep_info);
        }
    }
    let mut candidates = vec![
        dir.join(format!("{file_stem}.d")),
        deps.join(format!("{name}.d")),
    ];
    candidates.extend(
        sibling_files
            .iter()
            .filter_map(|sibling| sibling.as_std_path().parent())
            .filter(|unit_dir| *unit_dir != dir)
            .map(|unit_dir| unit_dir.join(format!("{name}.d"))),
    );
    candidates.push(dir.join(format!("{name}.d")));
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// `deps/<stem>.d` for a recorded library name — `DT_NEEDED`, PE import,
/// `LC_LOAD_DYLIB`, `DT_SONAME` or `LC_ID_DYLIB` — whose `lib` prefix and
/// shared-library suffix the dep-info's stem drops.
fn recorded_dep_info(deps_dir: &Path, recorded_name: &str) -> Option<PathBuf> {
    let stem = Path::new(needed_file_name(recorded_name))
        .file_stem()?
        .to_str()?;
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    Some(deps_dir.join(format!("{stem}.d")))
}

/// The name a dynamic library records for itself — `DT_SONAME` on ELF,
/// `LC_ID_DYLIB` on Mach-O — the same string a consumer's records then
/// carry. A PE image records no self-name, and a file that is not a dynamic
/// library records none either.
fn recorded_library_name(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    match object::FileKind::parse(&*data).ok()? {
        object::FileKind::Elf32 => {
            elf_recorded_name::<object::elf::FileHeader32<object::Endianness>>(&data)
        }
        object::FileKind::Elf64 => {
            elf_recorded_name::<object::elf::FileHeader64<object::Endianness>>(&data)
        }
        object::FileKind::MachO32 => {
            macho_recorded_name::<object::macho::MachHeader32<object::Endianness>>(&data)
        }
        object::FileKind::MachO64 => {
            macho_recorded_name::<object::macho::MachHeader64<object::Endianness>>(&data)
        }
        _ => None,
    }
}

/// The `DT_SONAME` an ELF image's dynamic section records for itself.
fn elf_recorded_name<Elf>(data: &[u8]) -> Option<String>
where
    Elf: object::read::elf::FileHeader<Endian = object::Endianness>,
{
    use object::read::elf::{Dyn as _, ElfFile};

    let file = ElfFile::<Elf>::parse(data).ok()?;
    let endian = file.endian();
    let sections = file.elf_section_table();
    let (dyns, strings_index) = sections.dynamic(endian, data).ok()??;
    let strings = sections.strings(endian, data, strings_index).ok()?;
    dyns.iter()
        .find(|d| d.tag32(endian) == Some(object::elf::DT_SONAME))
        .and_then(|d| d.string(endian, strings).ok())
        .map(|name| String::from_utf8_lossy(name).into_owned())
}

/// The `LC_ID_DYLIB` install name a Mach-O image records for itself.
fn macho_recorded_name<Mach>(data: &[u8]) -> Option<String>
where
    Mach: object::read::macho::MachHeader,
{
    use object::read::macho::LoadCommandVariant;

    let file = object::read::macho::MachOFile::<Mach>::parse(data).ok()?;
    let endian = file.endian();
    let mut commands = file.macho_load_commands().ok()?;
    while let Ok(Some(command)) = commands.next() {
        if let Ok(LoadCommandVariant::IdDylib(dylib)) = command.variant() {
            return command
                .string(endian, dylib.dylib.name)
                .ok()
                .map(|name| String::from_utf8_lossy(name).into_owned());
        }
    }
    None
}

/// The prerequisite paths a dep-info `.d` lists.
///
/// Cargo writes Makefile syntax: one `<target>: <space-separated
/// prerequisites>` rule per emitted artifact, then an empty `<path>:` rule
/// per prerequisite. rustc's `escape_dep_filename`
/// (`compiler/rustc_interface/src/passes.rs`) escapes *only* a literal space
/// as `\ ` — every other byte, a Windows backslash or drive-letter colon
/// included, is verbatim — and Cargo's own `parse_rustc_dep_info`
/// (`src/cargo/core/compiler/fingerprint/dep_info.rs`) reads the same
/// contract: split a rule at its first `": "` — `C:\` is colon-then-
/// backslash and a literal `": "` inside a name arrives escaped `":\ "`, so
/// the separator is unambiguous — then treat a token's trailing `\` as the
/// escaped space joining it to the next token. rustc never emits `$$` or
/// `\\` escapes in prerequisites, so neither is unescaped here: doing so
/// would corrupt the verbatim bytes a Windows path carries. A `\` at the
/// end of a line is make's continuation and joins the next line before
/// tokenizing.
fn dep_info_prerequisites(contents: &str) -> Vec<PathBuf> {
    // Join `\<newline>` continuations into one logical line per rule before
    // anything looks for the `": "` separator.
    let mut joined = String::with_capacity(contents.len());
    for line in contents.lines() {
        if let Some(head) = line.strip_suffix('\\') {
            joined.push_str(head);
            joined.push(' ');
        } else {
            joined.push_str(line);
            joined.push('\n');
        }
    }
    let mut prerequisites = Vec::new();
    for line in joined.lines() {
        let Some((_, rest)) = line.split_once(": ") else {
            continue;
        };
        let mut token = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek() == Some(&' ') => {
                    chars.next();
                    token.push(' ');
                }
                c if c.is_whitespace() => {
                    if !token.is_empty() {
                        prerequisites.push(PathBuf::from(std::mem::take(&mut token)));
                    }
                }
                c => token.push(c),
            }
        }
        if !token.is_empty() {
            prerequisites.push(PathBuf::from(token));
        }
    }
    prerequisites
}

/// The package name a `package_id` specifier carries — `source#name@version`,
/// or the source's final path segment for the older `source#version` form.
fn artifact_package_name(package_id: &cargo_metadata::PackageId) -> &str {
    let repr = package_id.repr.as_str();
    let (source, fragment) = repr.rsplit_once('#').unwrap_or((repr, ""));
    fragment.split_once('@').map_or_else(
        || source.rsplit('/').next().unwrap_or(repr),
        |(name, _)| name,
    )
}

/// `cargo clean -p <package>` in `crate_dir`, confined to `target_dir`: drops
/// the package's units — including the unhashed artifact another source's
/// build left behind — so the next build re-emits this graph's own.
async fn clean_cargo_package(
    crate_dir: &Path,
    package: &str,
    target_dir: &Path,
) -> Result<(), RustBuildError> {
    let mut command = Command::new("cargo");
    command
        .arg("clean")
        .arg("-p")
        .arg(package)
        .arg("--target-dir")
        .arg(target_dir)
        .current_dir(crate_dir);
    configure_generated_crate_compilation(&mut command);
    let output = command
        .output()
        .await
        .map_err(RustBuildError::FailToExecuteCargoBuild)?;
    if !output.status.success() {
        return Err(RustBuildError::FailToBuildRustLibrary(io::Error::other(
            format!(
                "cargo clean -p {package} failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )));
    }
    Ok(())
}

/// `RustBuildError` for the rustflags-resolution path: a malformed Cargo
/// config file, a failed `rustc --print cfg`, or a flag that cannot be
/// encoded.
fn rustflags_resolution_error(error: impl std::fmt::Display) -> RustBuildError {
    RustBuildError::FailToBuildRustLibrary(io::Error::other(error.to_string()))
}

/// The file extension rustc gives an executable for `triple`: `.exe` on
/// Windows, `.wasm` on a bare `wasm32` target, none elsewhere.
fn executable_suffix(triple: &Triple) -> &'static str {
    if triple.operating_system == OperatingSystem::Windows {
        ".exe"
    } else if matches!(triple.architecture, target_lexicon::Architecture::Wasm32)
        && triple.operating_system != OperatingSystem::Emscripten
    {
        ".wasm"
    } else {
        ""
    }
}

fn combined_build_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.is_empty() {
        stdout.to_string()
    } else {
        stderr.to_string()
    }
}

/// Lines a failure report keeps when the terminal already streamed the whole
/// build live — the dump is truncated to this tail.
const FAILURE_TAIL_LINES: usize = 40;

/// The last [`FAILURE_TAIL_LINES`] lines of `text` — what a failure report
/// needs when the terminal already rendered the full stream.
pub(crate) fn output_tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= FAILURE_TAIL_LINES {
        return text.to_owned();
    }
    format!(
        "… {} earlier lines already streamed above …\n{}",
        lines.len() - FAILURE_TAIL_LINES,
        lines[lines.len() - FAILURE_TAIL_LINES..].join("\n")
    )
}

fn should_auto_install_meson(build_output: &str) -> bool {
    let lower = build_output.to_ascii_lowercase();
    lower.contains("meson")
        && (lower.contains("not found")
            || lower.contains("no such file")
            || lower.contains("failed to execute")
            || lower.contains("is required"))
}

fn should_retry_after_cmake_generator_mismatch(build_output: &str) -> bool {
    let lower = build_output.to_ascii_lowercase();
    lower.contains("cmake error") && lower.contains("does not match the generator used previously")
}

fn remove_cmake_build_dirs_in(build_root: &Path) -> std::io::Result<usize> {
    if !build_root.exists() {
        return Ok(0);
    }

    let mut removed = 0usize;
    for entry in std::fs::read_dir(build_root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let cmake_build_dir = path.join("out").join("build");
        if cmake_build_dir.join("CMakeCache.txt").exists() {
            std::fs::remove_dir_all(cmake_build_dir)?;
            removed += 1;
        }
    }

    Ok(removed)
}

#[cfg(target_os = "macos")]
async fn ensure_meson_installed_for_build() -> Result<(), String> {
    use crate::toolchain::meson::Meson;
    use crate::toolchain::{Installation as _, Toolchain as _, ToolchainError};

    let host = crate::toolchain::Host::current();
    match Meson.check(&host).await {
        Ok(()) => Ok(()),
        Err(ToolchainError::Fixable(installation)) => {
            installation.install(&host).await.map_err(|e| e.to_string())
        }
        Err(ToolchainError::Unfixable(e)) => Err(e.to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
fn ensure_meson_installed_for_build() -> impl std::future::Future<Output = Result<(), String>> {
    std::future::ready(Err(
        "automatic meson installation is only supported on macOS".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use smol::process::Command;
    use target_lexicon::Triple;
    use tempfile::tempdir;

    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{
        BuildOptions, BuildProfile, BuiltTarget, CargoTarget, CompileEvent, RustBuild,
        RustDynamicLibraries, RustLinkage, classify_compile_line, combined_build_output,
        dynamic_library_file_name, executable_suffix, lib_extension_for_triple,
        reported_shared_runtime, resolve_dxc_runtime_in, resolve_rust_standard_library_in,
    };

    fn shared_runtime_artifact_json(
        manifest: &std::path::Path,
        file: &std::path::Path,
        package: &str,
    ) -> String {
        serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": format!("path+file:///x#{package}@0.1.0"),
            "manifest_path": manifest,
            "target": {
                "kind": ["dylib"],
                "crate_types": ["dylib"],
                "name": package,
                "src_path": manifest.parent().expect("manifest dir").join("src/lib.rs"),
                "edition": "2021",
                "doc": false,
                "doctest": false,
                "test": false,
            },
            "profile": {
                "opt_level": "0",
                "debuginfo": 0,
                "debug_assertions": true,
                "overflow_checks": true,
                "test": false,
            },
            "features": [],
            "filenames": [file],
            "executable": null,
            "fresh": true,
        })
        .to_string()
    }

    #[test]
    fn reported_shared_runtime_selects_waterui_dylib_dynamic_artifact() {
        let temporary = tempdir().expect("tempdir");
        let manifest = temporary.path().join("waterui-dylib/Cargo.toml");
        let runtime = temporary
            .path()
            .join("target/debug/deps/libwaterui_dylib.so");
        let unrelated_manifest = temporary.path().join("app/Cargo.toml");
        let unrelated = temporary.path().join("target/debug/app");
        let stdout = format!(
            "{}\n{}\n",
            shared_runtime_artifact_json(&unrelated_manifest, &unrelated, "app"),
            shared_runtime_artifact_json(&manifest, &runtime, "waterui-dylib"),
        );

        assert_eq!(
            reported_shared_runtime(stdout.as_bytes()).expect("runtime report"),
            Some(runtime)
        );
    }

    #[test]
    fn missing_shared_runtime_report_is_none_and_accessor_errors() {
        let temporary = tempdir().expect("tempdir");
        let stdout = shared_runtime_artifact_json(
            &temporary.path().join("app/Cargo.toml"),
            &temporary.path().join("target/debug/app"),
            "app",
        );
        assert_eq!(
            reported_shared_runtime(stdout.as_bytes()).expect("runtime report"),
            None
        );

        let profile_dir = temporary.path().join("target/debug");
        let error = BuiltTarget {
            profile_dir: profile_dir.clone(),
            artifact: temporary.path().join("app"),
            executable: None,
            entry_binary: None,
            shared_runtime: None,
            app_library: None,
            cef_helper: None,
        }
        .shared_runtime()
        .expect_err("missing runtime should fail");
        let message = error.to_string();
        assert!(message.contains("waterui-dylib"));
        assert!(message.contains(&profile_dir.display().to_string()));
    }

    #[test]
    fn reported_shared_runtime_rejects_multiple_manifests() {
        let temporary = tempdir().expect("tempdir");
        let first_manifest = temporary.path().join("first/Cargo.toml");
        let second_manifest = temporary.path().join("second/Cargo.toml");
        let stdout = format!(
            "{}\n{}\n",
            shared_runtime_artifact_json(
                &first_manifest,
                &temporary.path().join("target/debug/libfirst.so"),
                "waterui-dylib",
            ),
            shared_runtime_artifact_json(
                &second_manifest,
                &temporary.path().join("target/debug/libsecond.so"),
                "waterui-dylib",
            ),
        );

        let error =
            reported_shared_runtime(stdout.as_bytes()).expect_err("ambiguous runtime report");
        let message = error.to_string();
        assert!(message.contains(&first_manifest.display().to_string()));
        assert!(message.contains(&second_manifest.display().to_string()));
    }

    fn triple(value: &str) -> Triple {
        value.parse().expect("test target triple must parse")
    }

    #[test]
    fn crate_type_override_applies_only_to_library_targets() {
        assert!(CargoTarget::Lib.accepts_crate_type_override());
        assert!(!CargoTarget::Binary("waterui-cef-helper").accepts_crate_type_override());
        assert_eq!(CargoTarget::Lib.cargo_args(None), ["--lib"]);
        assert_eq!(
            CargoTarget::Lib.cargo_args(Some("cdylib")),
            ["--lib", "--crate-type", "cdylib"]
        );
        assert_eq!(
            CargoTarget::Binary("waterui-cef-helper").cargo_args(None),
            ["--bin", "waterui-cef-helper"]
        );
    }

    #[test]
    fn build_std_envs_wire_the_wrapper_and_clear_workspace_wrappers() {
        use std::ffi::OsStr;

        let dir = tempdir().expect("target dir");
        let toolchain = "nightly-2026-09-09-aarch64-apple-darwin";
        let target_dir = dir.path().join("target");
        let build = RustBuild::new(dir.path(), triple("aarch64-linux-android"))
            .with_build_std(toolchain)
            .with_target_dir(target_dir.clone())
            .with_sccache(std::path::PathBuf::from("/fake/sccache"));
        let mut cmd = smol::process::Command::new("cargo");
        smol::block_on(build.with_build_std_envs(&mut cmd, false)).expect("build-std envs apply");

        let env = |key: &str| -> Option<Option<OsString>> {
            cmd.get_envs()
                .find(|(name, _)| *name == OsStr::new(key))
                .map(|(_, value)| value.map(ToOwned::to_owned))
        };
        assert_eq!(
            env("RUSTUP_TOOLCHAIN"),
            Some(Some(OsString::from(toolchain)))
        );
        assert_eq!(
            env("RUSTC_WRAPPER"),
            Some(Some(
                crate::toolchain::Host::current_exe()
                    .expect("the test binary path")
                    .into_os_string()
            )),
            "the wrapper must name this binary"
        );
        assert_eq!(
            env(crate::workflows::rustc_wrapper::WRAPPER_MODE_ENV),
            Some(Some(OsString::from("1")))
        );
        assert_eq!(
            env(crate::workflows::rustc_wrapper::BUILD_STD_TARGET_ENV),
            Some(Some(OsString::from("aarch64-linux-android")))
        );
        let expected_dylib_dir = target_dir
            .join("aarch64-linux-android")
            .join("debug")
            .join("deps");
        assert_eq!(
            env(crate::workflows::rustc_wrapper::BUILD_STD_DYLIB_DIR_ENV),
            Some(Some(expected_dylib_dir.into_os_string()))
        );
        assert_eq!(
            env(crate::workflows::rustc_wrapper::WRAPPER_CHAIN_ENV),
            Some(Some(OsString::from("/fake/sccache"))),
            "a configured sccache chains behind the shim"
        );
        // A workspace wrapper would replace RUSTC_WRAPPER on exactly the
        // link-emitting member units, so both spellings must be removed.
        assert_eq!(env("RUSTC_WORKSPACE_WRAPPER"), Some(None));
        assert_eq!(env("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"), Some(None));
    }

    #[test]
    fn apple_platform_dylibs_use_macho_extension() {
        assert_eq!(
            lib_extension_for_triple(&triple("aarch64-apple-darwin")),
            "dylib"
        );
        assert_eq!(
            lib_extension_for_triple(&triple("aarch64-apple-ios-sim")),
            "dylib"
        );
        assert_eq!(
            lib_extension_for_triple(&triple("aarch64-apple-ios")),
            "dylib"
        );
    }

    #[test]
    fn non_apple_platform_dylibs_keep_platform_extensions() {
        assert_eq!(
            lib_extension_for_triple(&triple("aarch64-linux-android")),
            "so"
        );
        assert_eq!(
            lib_extension_for_triple(&triple("x86_64-unknown-linux-gnu")),
            "so"
        );
        assert_eq!(
            lib_extension_for_triple(&triple("x86_64-pc-windows-msvc")),
            "dll"
        );
    }

    #[test]
    fn development_and_packaging_have_distinct_linkage() {
        assert_eq!(
            BuildOptions::development(BuildProfile::Debug).linkage(),
            RustLinkage::SharedRuntime
        );
        assert_eq!(
            BuildOptions::packaging(BuildProfile::Debug).linkage(),
            RustLinkage::Static
        );
        assert!(BuildOptions::development(BuildProfile::Release).is_release());
        assert!(BuildOptions::packaging(BuildProfile::Release).is_release());
    }

    #[test]
    fn build_profile_release_variants_select_the_release_profile() {
        assert!(BuildProfile::Release.is_release());
        assert!(BuildProfile::Profiling.is_release());
        assert!(!BuildProfile::Debug.is_release());
        assert!(!BuildProfile::Optimized.is_release());
    }

    #[test]
    fn development_profile_envs_realize_the_selected_trade_off() {
        let optimized = BuildOptions::development(BuildProfile::Optimized);
        let envs = optimized.cargo_envs();
        assert!(
            envs.contains(&(
                "CARGO_PROFILE_DEV_OPT_LEVEL".to_string(),
                OsString::from("1")
            )),
            "optimized development lifts the dev opt-level: {envs:?}"
        );
        assert!(
            envs.contains(&(
                "CARGO_PROFILE_DEV_DEBUG_ASSERTIONS".to_string(),
                OsString::from("false")
            )),
            "optimized development drops dep debug assertions: {envs:?}"
        );
        assert!(
            envs.contains(&(
                "CARGO_PROFILE_DEV_DEBUG".to_string(),
                OsString::from("true")
            )),
            "optimized development keeps full debug info: {envs:?}"
        );

        let shared_runtime_envs = [
            (
                "CARGO_PROFILE_RELEASE_PANIC".to_string(),
                OsString::from("unwind"),
            ),
            (
                "CARGO_PROFILE_RELEASE_LTO".to_string(),
                OsString::from("off"),
            ),
        ];
        for env in &shared_runtime_envs {
            assert!(
                BuildOptions::development(BuildProfile::Release)
                    .cargo_envs()
                    .contains(env),
                "a release development build links the shared runtime: missing {env:?}"
            );
            assert!(
                !BuildOptions::development(BuildProfile::Release)
                    .with_static_runtime()
                    .cargo_envs()
                    .contains(env),
                "a static runtime keeps the manifest's {env:?}"
            );
        }
        let unwind = &shared_runtime_envs[0];

        let profiling = BuildOptions::development(BuildProfile::Profiling);
        let envs = profiling.cargo_envs();
        assert!(
            envs.contains(unwind),
            "profiling links the shared runtime too"
        );
        for key in [
            "CARGO_PROFILE_RELEASE_OPT_LEVEL",
            "CARGO_PROFILE_RELEASE_DEBUG",
            "CARGO_PROFILE_RELEASE_STRIP",
        ] {
            assert!(
                envs.iter().any(|(env_key, _)| env_key == key),
                "profiling keeps debug info and symbols: missing {key} in {envs:?}"
            );
        }

        assert!(
            BuildOptions::development(BuildProfile::Debug)
                .cargo_envs()
                .is_empty(),
            "plain debug runs the declared dev profile"
        );
    }

    #[test]
    fn packaging_never_overrides_the_declared_profile() {
        for profile in [
            BuildProfile::Debug,
            BuildProfile::Optimized,
            BuildProfile::Release,
            BuildProfile::Profiling,
        ] {
            assert!(
                BuildOptions::packaging(profile).cargo_envs().is_empty(),
                "packaging {profile:?} must ship the declared profile"
            );
        }
    }

    #[test]
    fn resolves_target_standard_library_without_guessing_hash() {
        let directory = tempdir().expect("temporary target libdir");
        let android_triple = triple("aarch64-linux-android");
        let expected = directory.path().join("libstd-1234567890abcdef.so");
        std::fs::write(&expected, []).expect("write test std library");
        std::fs::write(directory.path().join("libcore.rlib"), []).expect("write unrelated library");

        assert_eq!(
            resolve_rust_standard_library_in(directory.path(), &android_triple)
                .expect("resolve dynamic std"),
            expected
        );
        assert_eq!(
            dynamic_library_file_name("waterui_dylib", &android_triple),
            "libwaterui_dylib.so"
        );
        assert_eq!(
            dynamic_library_file_name("waterui_dylib", &triple("x86_64-pc-windows-msvc")),
            "waterui_dylib.dll"
        );
    }

    #[test]
    fn compile_progress_classifies_cargo_unit_lines() {
        assert_eq!(
            classify_compile_line("   Compiling serde v1.0.228"),
            CompileEvent::Unit {
                phase: "Compiling",
                name: "serde".to_string(),
                version: Some("1.0.228".to_string()),
            }
        );
        assert_eq!(
            classify_compile_line("   Compiling waterui-app v0.1.0 (/tmp/app)"),
            CompileEvent::Unit {
                phase: "Compiling",
                name: "waterui-app".to_string(),
                version: Some("0.1.0".to_string()),
            }
        );
        assert_eq!(
            classify_compile_line("    Checking libc v0.2.171"),
            CompileEvent::Unit {
                phase: "Checking",
                name: "libc".to_string(),
                version: Some("0.2.171".to_string()),
            }
        );
    }

    #[test]
    fn compile_progress_keeps_non_unit_lines_verbatim() {
        assert_eq!(
            classify_compile_line("   Compiling 12 crates"),
            CompileEvent::Line("Compiling 12 crates".to_string())
        );
        assert_eq!(
            classify_compile_line("     Downloaded 300 crates (5.2 MB) in 1.23s"),
            CompileEvent::Line("Downloaded 300 crates (5.2 MB) in 1.23s".to_string())
        );
        assert_eq!(
            classify_compile_line(
                "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.23s"
            ),
            CompileEvent::Finished(
                "Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.23s".to_string()
            )
        );
        assert_eq!(
            classify_compile_line("warning: unused import"),
            CompileEvent::Line("warning: unused import".to_string())
        );
    }

    #[test]
    fn compile_progress_classifies_through_ansi_color() {
        // A user-forced `[term] color = "always"` or the CARGO_TERM_COLOR the
        // CLI sets for terminals wraps cargo's status words in escapes.
        let colored = "\u{1b}[0m\u{1b}[1m\u{1b}[32m   Compiling\u{1b}[0m serde v1.0.228";
        assert_eq!(
            classify_compile_line(colored),
            CompileEvent::Unit {
                phase: "Compiling",
                name: "serde".to_string(),
                version: Some("1.0.228".to_string()),
            }
        );
        let colored_finished =
            "\u{1b}[0m\u{1b}[1m\u{1b}[32m    Finished\u{1b}[0m `dev` profile in 1.23s";
        assert_eq!(
            classify_compile_line(colored_finished),
            CompileEvent::Finished(colored_finished.trim().to_string())
        );
    }

    /// Two projects named `demo` in different directories generate crates
    /// whose package names differ by the project-root tag, so one shared
    /// Cargo target gives each its own uplifted artifact — and the build
    /// resolves it from Cargo's `compiler-artifact` report rather than a
    /// bare `<profile>/<name>` guess.
    #[test]
    fn same_named_projects_resolve_their_own_artifacts_in_one_shared_target() {
        use crate::project_model::project_types::{CrateName, generated_crate_name};

        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let shared_target = temporary.path().join("shared-target");
            let demo = CrateName::try_from("demo").expect("crate name");
            let mut artifacts = Vec::new();
            for (directory, marker) in [("first", "first"), ("second", "second")] {
                let project_root = temporary.path().join(directory);
                let crate_dir = project_root.join("hydrolysis");
                std::fs::create_dir_all(crate_dir.join("src")).expect("crate dir");
                let package = generated_crate_name(&demo, "hydrolysis", &project_root);
                std::fs::write(
                    crate_dir.join("Cargo.toml"),
                    format!(
                        "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
                    ),
                )
                .expect("manifest");
                std::fs::write(
                    crate_dir.join("src/main.rs"),
                    format!("fn main() {{ println!(\"{marker}\"); }}\n"),
                )
                .expect("main.rs");

                let artifact = super::RustBuild::new(&crate_dir, Triple::host())
                    .with_target_dir(&shared_target)
                    .build_binary(package.as_str(), false)
                    .await
                    .expect("the generated crate builds")
                    .artifact;
                assert!(artifact.is_file(), "the reported artifact exists");
                artifacts.push(artifact);
            }

            assert_ne!(
                artifacts[0], artifacts[1],
                "each same-named project resolves its own artifact"
            );
            for (artifact, marker) in artifacts.iter().zip(["first", "second"]) {
                let ran = std::process::Command::new(artifact)
                    .output()
                    .expect("the resolved artifact executes");
                assert_eq!(
                    String::from_utf8_lossy(&ran.stdout).trim(),
                    marker,
                    "the artifact is this project's binary, not the sibling's"
                );
            }
        });
    }

    /// Two same-named crates in different directories share one Cargo
    /// target: Cargo 1.99's unit metadata omits the manifest path, so
    /// without the marker the second build's fingerprint would collide
    /// with the first's — reported `fresh`, its `executable` naming the
    /// sibling's bytes. The marker `--cfg` keeps each source its own
    /// unit, and the resolved artifact is the variant-stable
    /// `deps/<name>-<marker>` link of the `executable` this run reported.
    #[test]
    fn binary_artifact_is_the_output_of_the_build_that_just_ran() {
        let temporary = tempdir().expect("tempdir");
        let shared_target = temporary.path().join("shared-target");
        let package = "demo-hydrolysis-deadbeef";
        // Both crates' sources exist before either build runs: each is its
        // own unit under the marker, so `second` compiles against `first`'s
        // unit rather than aliasing it.
        for (directory, marker) in [("first", "first"), ("second", "second")] {
            let crate_dir = temporary.path().join(directory).join("hydrolysis");
            std::fs::create_dir_all(crate_dir.join("src")).expect("crate dir");
            std::fs::write(
                crate_dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
                ),
            )
            .expect("manifest");
            std::fs::write(
                crate_dir.join("src/main.rs"),
                format!("fn main() {{ println!(\"{marker}\"); }}\n"),
            )
            .expect("main.rs");
        }
        smol::block_on(async {
            // The third entry rebuilds `first`: its unit is `fresh` this
            // time, and Cargo re-uplifts its `deps/` output over the
            // sibling's bytes at the shared `<profile>/<name>` — the
            // marked link keeps each variant's artifact stable regardless.
            let mut artifacts = Vec::new();
            for (directory, marker) in
                [("first", "first"), ("second", "second"), ("first", "first")]
            {
                let crate_dir = temporary.path().join(directory).join("hydrolysis");
                let artifact = super::RustBuild::new(&crate_dir, Triple::host())
                    .with_target_dir(&shared_target)
                    .build_binary(package, false)
                    .await
                    .expect("the generated crate builds")
                    .artifact;
                // `deps/<name>-<marker>`: the marked name is unique per
                // variant — running it checks the slot carries this
                // build's output whether the unit compiled or stayed fresh.
                let ran = Command::new(&artifact)
                    .output()
                    .await
                    .expect("the resolved artifact executes");
                assert_eq!(
                    String::from_utf8_lossy(&ran.stdout).trim(),
                    marker,
                    "the launched artifact is the build that just ran, not the sibling's"
                );
                artifacts.push(artifact);
            }
            assert_ne!(
                artifacts[0], artifacts[1],
                "the marker separates the two same-named sources' artifacts"
            );
            assert_eq!(
                artifacts[0], artifacts[2],
                "the same variant relinks to the same marked path"
            );
        });
    }

    /// The resolved binary artifact is the variant-stable
    /// `deps/<name>-<marker>` hard link of the reported `executable`.
    /// The marker covers the build's identity, not its sources, so a
    /// rebuild of the same crate relinks the same marked path to the new
    /// run's bytes — never keeping the earlier build's.
    #[test]
    fn binary_artifact_is_the_variant_stable_marked_link() {
        let temporary = tempdir().expect("tempdir");
        let shared_target = temporary.path().join("shared-target");
        let package = "demo-hydrolysis-deadbeef";
        let crate_dir = temporary.path().join("first").join("hydrolysis");
        std::fs::create_dir_all(crate_dir.join("src")).expect("crate dir");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        )
        .expect("manifest");

        smol::block_on(async {
            let mut artifacts = Vec::new();
            for marker in ["first", "second"] {
                smol::fs::write(
                    crate_dir.join("src/main.rs"),
                    format!("fn main() {{ println!(\"{marker}\"); }}\n"),
                )
                .await
                .expect("main.rs");
                let artifact = super::RustBuild::new(&crate_dir, Triple::host())
                    .with_target_dir(&shared_target)
                    .build_binary(package, false)
                    .await
                    .expect("the generated crate builds")
                    .artifact;
                let file_name = artifact
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("artifact file name");
                assert_eq!(
                    artifact
                        .parent()
                        .and_then(|parent| parent.file_name())
                        .and_then(|name| name.to_str()),
                    Some("deps"),
                    "the resolved artifact lives under deps/: {}",
                    artifact.display()
                );
                assert!(
                    file_name
                        .strip_prefix(package)
                        .and_then(|rest| rest.strip_prefix('-'))
                        .is_some_and(|marked| {
                            !marked
                                .strip_suffix(executable_suffix(&Triple::host()))
                                .unwrap_or(marked)
                                .is_empty()
                        }),
                    "the resolved artifact is `deps/<name>-<marker><suffix>`: {file_name}"
                );

                let ran = Command::new(&artifact)
                    .output()
                    .await
                    .expect("the resolved artifact executes");
                assert_eq!(
                    String::from_utf8_lossy(&ran.stdout).trim(),
                    marker,
                    "the rebuild relinked the marked path instead of leaving stale bytes"
                );
                artifacts.push(artifact);
            }
            assert_eq!(
                artifacts[0], artifacts[1],
                "the same build variant resolves the same marked path"
            );
        });
    }

    /// `marked_binary_artifact` hard-links the reported uplift to
    /// `deps/<name>-<marker>` and replaces it atomically on the next call.
    #[test]
    fn marked_binary_artifact_relinks_the_uplift_to_the_marked_path() {
        let temporary = tempdir().expect("tempdir");
        let profile_dir = temporary.path().join("debug");
        std::fs::create_dir_all(profile_dir.join("deps")).expect("deps dir");
        let uplift = profile_dir.join("probe_bin");
        std::fs::write(&uplift, b"first bytes").expect("uplift");

        smol::block_on(async {
            let marked = super::marked_binary_artifact(
                &uplift,
                &profile_dir,
                "probe_bin",
                "deadbeef",
                &Triple::host(),
            )
            .await
            .expect("the marked link is made");
            let expected = format!("probe_bin-deadbeef{}", executable_suffix(&Triple::host()));
            assert_eq!(
                marked.file_name().and_then(|name| name.to_str()),
                Some(expected.as_str()),
            );
            assert_eq!(
                smol::fs::read(&marked).await.expect("marked artifact"),
                b"first bytes"
            );

            // A second relink — as a same-variant build performs it —
            // replaces the marked path with the new run's bytes atomically
            // through the same-directory temporary and `rename`. The uplift
            // is replaced with a new inode carrying different bytes, as a
            // rebuild leaves it: writing through the shared inode would
            // change the marked link's bytes without exercising the relink.
            let replacement = profile_dir.join("probe_bin.next");
            smol::fs::write(&replacement, b"second bytes")
                .await
                .expect("rewrite uplift");
            smol::fs::rename(&replacement, &uplift)
                .await
                .expect("replace the uplift with a new inode");
            let relinked = super::marked_binary_artifact(
                &uplift,
                &profile_dir,
                "probe_bin",
                "deadbeef",
                &Triple::host(),
            )
            .await
            .expect("the marked link is replaced");
            assert_eq!(marked, relinked);
            assert!(
                super::same_file(&uplift, &relinked)
                    .await
                    .expect("compare the relinked artifact"),
                "the marked path relinks to the new uplift's inode"
            );
            assert_eq!(
                smol::fs::read(&relinked).await.expect("marked artifact"),
                b"second bytes"
            );
            let staging = format!(
                ".probe_bin-deadbeef{}.tmp",
                executable_suffix(&Triple::host())
            );
            assert!(
                !profile_dir.join("deps").join(&staging).exists(),
                "no staging file is left behind"
            );
        });
    }

    /// `reported_artifact` matches on the artifact's manifest path — the
    /// identity Cargo assigns the unit — and returns the file the message
    /// reports even when that path is the hash-suffixed `deps/` copy, so a
    /// sibling package's artifact in the same stream is never picked up.
    #[test]
    fn reported_artifact_selects_the_matching_manifests_file() {
        let temporary = tempdir().expect("tempdir");
        let crate_dir = temporary.path().join("demo-hydrolysis-deadbeef");
        std::fs::create_dir_all(&crate_dir).expect("crate dir");
        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\n").expect("manifest");
        let manifest =
            dunce::canonicalize(crate_dir.join("Cargo.toml")).expect("canonical manifest");
        let reported = crate_dir.join("target/debug/deps/demo_hydrolysis_deadbeef-abc123.rlib");
        std::fs::create_dir_all(reported.parent().expect("deps dir")).expect("deps dir");
        std::fs::write(&reported, []).expect("reported artifact");

        // The messages are serialized, never formatted: a `Path` must land in
        // the JSON as an escaped string, which `display()` cannot do on
        // Windows where paths carry backslashes.
        let artifact_json = |manifest: &std::path::Path, file: &std::path::Path, name: &str| {
            serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": format!("path+file:///x#{name}@0.1.0"),
                "manifest_path": manifest,
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": name,
                    "src_path": manifest.parent().expect("manifest dir").join("src/lib.rs"),
                    "edition": "2021",
                    "doc": true,
                    "doctest": true,
                    "test": true,
                },
                "profile": {
                    "opt_level": "0",
                    "debuginfo": 0,
                    "debug_assertions": true,
                    "overflow_checks": true,
                    "test": false,
                },
                "features": [],
                "filenames": [file],
                "executable": null,
                "fresh": true,
            })
            .to_string()
        };

        let other_manifest = temporary.path().join("other").join("Cargo.toml");
        let other_file = temporary.path().join("other.rlib");
        let stdout = format!(
            "{}\n{}\n",
            artifact_json(&other_manifest, &other_file, "other"),
            artifact_json(&manifest, &reported, "demo_hydrolysis_deadbeef"),
        );
        let resolved = super::reported_artifact(
            stdout.as_bytes(),
            &crate_dir,
            CargoTarget::Lib,
            Some("rlib"),
        )
        .expect("the matching manifest's artifact resolves");
        assert_eq!(resolved, reported);

        let foreign_only = artifact_json(&other_manifest, &other_file, "other");
        assert!(
            super::reported_artifact(
                foreign_only.as_bytes(),
                &crate_dir,
                CargoTarget::Lib,
                Some("rlib"),
            )
            .is_err(),
            "an artifact for another manifest is never selected"
        );
    }

    /// A manifest declaring `staticlib`, `cdylib` and `rlib` reports all
    /// emitted files on one artifact message; the archive selector picks the
    /// `.a` out of the set by extension and never the shared library or rlib.
    #[test]
    fn reported_artifact_selects_the_static_archive_from_declared_types() {
        let temporary = tempdir().expect("tempdir");
        let crate_dir = temporary.path().join("demo-ffi-deadbeef");
        std::fs::create_dir_all(&crate_dir).expect("crate dir");
        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\n").expect("manifest");
        let manifest =
            dunce::canonicalize(crate_dir.join("Cargo.toml")).expect("canonical manifest");
        let deps = crate_dir.join("target/release");
        std::fs::create_dir_all(&deps).expect("profile dir");
        let archive = deps.join("libdemo_ffi.a");
        let dylib = deps.join("libdemo_ffi.dylib");
        let rlib = deps.join("libdemo_ffi.rlib");
        for file in [&archive, &dylib, &rlib] {
            std::fs::write(file, []).expect("reported artifact");
        }

        let stdout = serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": "path+file:///x#demo-ffi@0.1.0",
            "manifest_path": manifest,
            "target": {
                "kind": ["lib"],
                "crate_types": ["staticlib", "cdylib", "rlib"],
                "name": "demo_ffi",
                "src_path": crate_dir.join("src/lib.rs"),
                "edition": "2021",
                "doc": true,
                "doctest": true,
                "test": true,
            },
            "profile": {
                "opt_level": "3",
                "debuginfo": 0,
                "debug_assertions": false,
                "overflow_checks": false,
                "test": false,
            },
            "features": [],
            "filenames": [archive, dylib, rlib],
            "executable": null,
            "fresh": false,
        })
        .to_string();

        let resolved =
            super::reported_artifact(stdout.as_bytes(), &crate_dir, CargoTarget::Lib, Some("a"))
                .expect("the declared static archive resolves");
        assert_eq!(resolved, archive);
    }

    /// A dependency's uplifted dylib is unhashed, so a `fresh` report does not
    /// prove the file is this source's — the dep-info beside it records the
    /// producing sources, and only a dep-info naming this unit's own manifest
    /// root clears it.
    #[test]
    fn stale_shared_dylib_packages_flags_a_foreign_written_artifact() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let deps = temporary.path().join("debug/deps");
            std::fs::create_dir_all(&deps).expect("deps dir");
            let dylib = deps.join("libwaterui_dylib.so");
            std::fs::write(&dylib, []).expect("dylib");

            // The manifest root carries a space so the dep-info fixture
            // exercises the `\ ` escape end to end: the written prerequisite
            // must still resolve to this root.
            let ours = temporary.path().join("our project");
            std::fs::create_dir_all(ours.join("src")).expect("our manifest dir");
            let manifest = ours.join("Cargo.toml");
            std::fs::write(&manifest, "").expect("manifest");
            let own_source = ours.join("src/lib.rs");
            std::fs::write(&own_source, "").expect("own source");

            let artifact = |fresh: bool| {
                serde_json::json!({
                    "reason": "compiler-artifact",
                    "package_id": "path+file:///x#waterui-dylib@0.1.0",
                    "manifest_path": manifest,
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["dylib"],
                        "name": "waterui_dylib",
                        "src_path": own_source,
                        "edition": "2021",
                        "doc": true,
                        "doctest": true,
                        "test": true,
                    },
                    "profile": {
                        "opt_level": "0",
                        "debuginfo": 0,
                        "debug_assertions": true,
                        "overflow_checks": true,
                        "test": false,
                    },
                    "features": [],
                    "filenames": [dylib],
                    "executable": null,
                    "fresh": fresh,
                })
                .to_string()
            };
            let dep_info = deps.join("waterui_dylib.d");

            // Dep-info rides in rustc's Makefile spelling: a literal space in
            // a path is `\ ` and every other byte is verbatim, so the fixture
            // writes real tempdir paths through the same escaping.
            let foreign = temporary.path().join("foreign");
            std::fs::create_dir_all(foreign.join("src")).expect("foreign source dir");
            let foreign_source = foreign.join("src/lib.rs");
            std::fs::write(&foreign_source, "").expect("foreign source");
            let dep_escape =
                |path: &std::path::Path| path.display().to_string().replace(' ', "\\ ");
            let write_dep_info = |source: &std::path::Path| {
                std::fs::write(
                    &dep_info,
                    format!("{}: {}\n", dep_escape(&dylib), dep_escape(source)),
                )
                .expect("dep-info");
            };

            // A `fresh` unit whose dep-info names another source's checkout.
            write_dep_info(&foreign_source);
            let needed = vec!["libwaterui_dylib.so".to_string()];
            let stale = super::stale_shared_dylib_packages(artifact(true).as_bytes(), &needed)
                .await
                .expect("scan");
            assert_eq!(
                stale,
                [super::StaleSharedDylib {
                    package: "waterui-dylib".to_owned(),
                    artifact: dylib.clone(),
                    reason: super::StaleSharedDylibReason::ForeignDepInfo {
                        dep_info: dep_info.clone(),
                    },
                }]
            );

            // The same file written by this unit's own source is trusted.
            write_dep_info(&own_source);
            let stale = super::stale_shared_dylib_packages(artifact(true).as_bytes(), &needed)
                .await
                .expect("scan");
            assert!(stale.is_empty(), "our own artifact is never stale");

            // A unit cargo just emitted needs no dep-info check at all.
            write_dep_info(&foreign_source);
            let stale = super::stale_shared_dylib_packages(artifact(false).as_bytes(), &needed)
                .await
                .expect("scan");
            assert!(stale.is_empty(), "a non-fresh unit wrote the file itself");
        });
    }

    /// A git- or registry-sourced `dylib` dependency hashes its metadata into
    /// every `deps/` name: rustc writes `deps/lib<crate>-<metadata>.so` and
    /// `deps/<crate>-<metadata>.d`, and the report names the unhashed uplift
    /// `<profile>/lib<crate>.so`. The name the consumer's records carry — the
    /// needed name the artifact's dynamic section reports — spells
    /// `deps/<crate>-<metadata>.d` exactly; missing it flags `MissingDepInfo`
    /// on every build, and the clean-and-rebuild remedy loops forever
    /// (water-rs/cli#184).
    #[test]
    fn stale_check_finds_the_hashed_dep_info_the_needed_name_records() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let profile = temporary.path().join("debug");
            let deps = profile.join("deps");
            std::fs::create_dir_all(&deps).expect("deps dir");
            // The layout a git-sourced `dylib`+`rlib` unit leaves (measured,
            // cargo 1.98): the reported files are the unhashed uplift and the
            // hashed rlib; the hashed dylib itself is never reported.
            let dylib = profile.join("libwaterui_dylib.so");
            std::fs::write(&dylib, []).expect("dylib");
            let rlib = deps.join("libwaterui_dylib-0123456789abcdef.rlib");
            std::fs::write(&rlib, []).expect("rlib");

            let ours = temporary.path().join("ours");
            std::fs::create_dir_all(ours.join("src")).expect("our manifest dir");
            let manifest = ours.join("Cargo.toml");
            std::fs::write(&manifest, "").expect("manifest");
            let own_source = ours.join("src/lib.rs");
            std::fs::write(&own_source, "").expect("own source");
            let dep_info = deps.join("waterui_dylib-0123456789abcdef.d");
            std::fs::write(
                &dep_info,
                format!("{}: {}\n", dylib.display(), own_source.display()),
            )
            .expect("dep-info");

            let stdout = serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "registry+https://x#waterui-dylib@0.1.0",
                "manifest_path": manifest,
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["dylib", "rlib"],
                    "name": "waterui_dylib",
                    "src_path": own_source,
                    "edition": "2021",
                    "doc": true,
                    "doctest": true,
                    "test": true,
                },
                "profile": {
                    "opt_level": "0",
                    "debuginfo": 0,
                    "debug_assertions": true,
                    "overflow_checks": true,
                    "test": false,
                },
                "features": [],
                "filenames": [dylib, rlib],
                "executable": null,
                "fresh": true,
            })
            .to_string();

            let needed = vec!["libwaterui_dylib-0123456789abcdef.so".to_string()];
            let stale = super::stale_shared_dylib_packages(stdout.as_bytes(), &needed)
                .await
                .expect("scan");
            assert!(
                stale.is_empty(),
                "a fresh dylib whose hashed dep-info names its own sources is trusted: {stale:?}"
            );
        });
    }

    /// The hashed dep-info a `dylib`-only unit leaves — the report names
    /// only the unhashed uplift, no hashed sibling at all — is found through
    /// the needed name the consumer records, whether Cargo's uplift aliased
    /// the `deps/` output as a hardlink or, on filesystems without links, a
    /// copy.
    #[test]
    fn stale_check_finds_dep_info_when_the_uplift_is_a_hardlink() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let profile = temporary.path().join("debug");
            let deps = profile.join("deps");
            std::fs::create_dir_all(&deps).expect("deps dir");
            // rustc's own output is `deps/lib<crate>-<metadata>.so`; cargo's
            // uplift is the same inode under the unhashed name.
            let deps_dylib = deps.join("libwaterui_dylib-0123456789abcdef.so");
            std::fs::write(&deps_dylib, []).expect("deps dylib");
            let dylib = profile.join("libwaterui_dylib.so");
            std::fs::hard_link(&deps_dylib, &dylib).expect("uplift hardlink");

            let ours = temporary.path().join("ours");
            std::fs::create_dir_all(ours.join("src")).expect("our manifest dir");
            let manifest = ours.join("Cargo.toml");
            std::fs::write(&manifest, "").expect("manifest");
            let own_source = ours.join("src/lib.rs");
            std::fs::write(&own_source, "").expect("own source");
            std::fs::write(
                deps.join("waterui_dylib-0123456789abcdef.d"),
                format!("{}: {}\n", deps_dylib.display(), own_source.display()),
            )
            .expect("dep-info");

            let stdout = serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "registry+https://x#waterui-dylib@0.1.0",
                "manifest_path": manifest,
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["dylib"],
                    "name": "waterui_dylib",
                    "src_path": own_source,
                    "edition": "2021",
                    "doc": true,
                    "doctest": true,
                    "test": true,
                },
                "profile": {
                    "opt_level": "0",
                    "debuginfo": 0,
                    "debug_assertions": true,
                    "overflow_checks": true,
                    "test": false,
                },
                "features": [],
                "filenames": [dylib],
                "executable": null,
                "fresh": true,
            })
            .to_string();

            let needed = vec!["libwaterui_dylib-0123456789abcdef.so".to_string()];
            let stale = super::stale_shared_dylib_packages(stdout.as_bytes(), &needed)
                .await
                .expect("scan");
            assert!(
                stale.is_empty(),
                "the needed name spells the hashed dep-info a hardlinked uplift leaves: {stale:?}"
            );
        });
    }

    /// Cargo's uplift falls back to copying where a hardlink is impossible;
    /// the needed name still spells `deps/<crate>-<metadata>.d` — the lookup
    /// never touched the uplift's link status.
    #[test]
    fn stale_check_finds_dep_info_when_the_uplift_is_a_copy() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let profile = temporary.path().join("debug");
            let deps = profile.join("deps");
            std::fs::create_dir_all(&deps).expect("deps dir");
            // rustc's own output is `deps/lib<crate>-<metadata>.so`; where
            // links are unavailable cargo's uplift copies it under the
            // unhashed name — a different inode sharing only the bytes.
            let deps_dylib = deps.join("libwaterui_dylib-0123456789abcdef.so");
            std::fs::write(&deps_dylib, []).expect("deps dylib");
            let dylib = profile.join("libwaterui_dylib.so");
            std::fs::copy(&deps_dylib, &dylib).expect("uplift copy");

            let ours = temporary.path().join("ours");
            std::fs::create_dir_all(ours.join("src")).expect("our manifest dir");
            let manifest = ours.join("Cargo.toml");
            std::fs::write(&manifest, "").expect("manifest");
            let own_source = ours.join("src/lib.rs");
            std::fs::write(&own_source, "").expect("own source");
            std::fs::write(
                deps.join("waterui_dylib-0123456789abcdef.d"),
                format!("{}: {}\n", deps_dylib.display(), own_source.display()),
            )
            .expect("dep-info");

            let stdout = serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "registry+https://x#waterui-dylib@0.1.0",
                "manifest_path": manifest,
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["dylib"],
                    "name": "waterui_dylib",
                    "src_path": own_source,
                    "edition": "2021",
                    "doc": true,
                    "doctest": true,
                    "test": true,
                },
                "profile": {
                    "opt_level": "0",
                    "debuginfo": 0,
                    "debug_assertions": true,
                    "overflow_checks": true,
                    "test": false,
                },
                "features": [],
                "filenames": [dylib],
                "executable": null,
                "fresh": true,
            })
            .to_string();

            let needed = vec!["libwaterui_dylib-0123456789abcdef.so".to_string()];
            let stale = super::stale_shared_dylib_packages(stdout.as_bytes(), &needed)
                .await
                .expect("scan");
            assert!(
                stale.is_empty(),
                "the needed name spells the hashed dep-info a copied uplift leaves: {stale:?}"
            );
        });
    }

    /// On Apple the artifact report names the unhashed uplift
    /// `libwaterui_dylib.dylib` while cargo writes `deps/waterui_dylib-<meta>.d`
    /// and the consuming image's `LC_LOAD_DYLIB` records the retargeted
    /// `@rpath/libwaterui_dylib.dylib` — an unhashed `deps/waterui_dylib.d`
    /// never exists, so the lookup reads the hashed sibling `deps/` carries
    /// (#197). Missing it flags `MissingDepInfo` on every build and the
    /// clean-and-rebuild remedy loops forever.
    #[test]
    fn stale_check_finds_the_hashed_dep_info_a_retargeted_apple_record_leaves() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let profile = temporary.path().join("debug");
            let deps = profile.join("deps");
            std::fs::create_dir_all(&deps).expect("deps dir");
            let dylib = profile.join("libwaterui_dylib.dylib");
            std::fs::write(&dylib, []).expect("dylib");

            let ours = temporary.path().join("ours");
            std::fs::create_dir_all(ours.join("src")).expect("our manifest dir");
            let manifest = ours.join("Cargo.toml");
            std::fs::write(&manifest, "").expect("manifest");
            let own_source = ours.join("src/lib.rs");
            std::fs::write(&own_source, "").expect("own source");
            std::fs::write(
                deps.join("waterui_dylib-0123456789abcdef.d"),
                format!(
                    "{}: {}\n",
                    deps.join("libwaterui_dylib-0123456789abcdef.dylib")
                        .display(),
                    own_source.display()
                ),
            )
            .expect("dep-info");

            let stdout = serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "registry+https://x#waterui-dylib@0.1.0",
                "manifest_path": manifest,
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["dylib"],
                    "name": "waterui_dylib",
                    "src_path": own_source,
                    "edition": "2021",
                    "doc": true,
                    "doctest": true,
                    "test": true,
                },
                "profile": {
                    "opt_level": "0",
                    "debuginfo": 0,
                    "debug_assertions": true,
                    "overflow_checks": true,
                    "test": false,
                },
                "features": [],
                "filenames": [dylib],
                "executable": null,
                "fresh": true,
            })
            .to_string();

            // The retargeted consumer records the unhashed install name; the
            // hashed dep-info a `deps/` sibling pairs with is the only `.d`.
            let needed = vec!["@rpath/libwaterui_dylib.dylib".to_string()];
            let stale = super::stale_shared_dylib_packages(stdout.as_bytes(), &needed)
                .await
                .expect("scan");
            assert!(
                stale.is_empty(),
                "the hashed `deps/<crate>-<meta>.d` the sibling dylib pairs with resolves: {stale:?}"
            );
        });
    }

    /// `resolve` stages the runtime under the name the artifact's dynamic
    /// section records — the hashed `deps/` name every platform's loader
    /// resolves — and additionally names the canonical
    /// `libwaterui_dylib.dylib` Apple alone needs: the `-lwaterui_dylib` link
    /// flag and `@rpath` install name cargo's hashed `deps/` output can never
    /// carry (water-rs/cli#197, the Apple arm of #184).
    #[test]
    fn shared_runtime_stages_the_recorded_name_and_the_apple_canonical_one() {
        fn libraries(recorded: &str, triple: Triple) -> super::RustDynamicLibraries {
            let waterui =
                super::StagedDynamicLibrary::needed(recorded, PathBuf::from("/deps/libwaterui"));
            let standard_library = super::StagedDynamicLibrary::needed(
                "libstd-0123456789abcdef.so",
                PathBuf::from("/deps/libstd"),
            );
            super::RustDynamicLibraries {
                waterui,
                standard_library,
                triple,
            }
        }

        let destination = Path::new("/dist");
        let apple = libraries(
            "deps/libwaterui_dylib-0123456789abcdef.dylib",
            triple("aarch64-apple-darwin"),
        );
        // The recorded (hashed) name still stages — a non-retargeted Mach-O
        // resolves it — and the canonical name is the `-l`/`@rpath` target.
        assert_eq!(
            Path::new("/dist").join("libwaterui_dylib-0123456789abcdef.dylib"),
            destination.join(&apple.waterui.staged_name)
        );
        assert_eq!(
            Path::new("/dist/libwaterui_dylib.dylib"),
            apple.apple_canonical_waterui(destination)
        );

        let linux = libraries(
            "deps/libwaterui_dylib-0123456789abcdef.so",
            triple("x86_64-unknown-linux-gnu"),
        );
        assert_eq!(
            Path::new("/dist/libwaterui_dylib-0123456789abcdef.so"),
            linux.apple_canonical_waterui(destination)
        );
    }

    /// Staging inputs shared by the canonical-destination cases: `source` is
    /// the artifact Cargo reported for `waterui-dylib`; the standard library
    /// takes no part in canonical naming.
    fn canonical_staging_libraries(source: PathBuf) -> super::RustDynamicLibraries {
        super::RustDynamicLibraries {
            waterui: super::StagedDynamicLibrary::reported(source).expect("reported name"),
            standard_library: super::StagedDynamicLibrary::reported(PathBuf::from(
                "/deps/libstd-0123456789abcdef.dylib",
            ))
            .expect("standard library name"),
            triple: triple("aarch64-apple-darwin"),
        }
    }

    fn dylib_bytes() -> Vec<u8> {
        (0_u32..25_000).flat_map(u32::to_le_bytes).collect()
    }

    /// When the reported runtime already occupies the canonical destination,
    /// staging must leave its bytes alone: `copy_file` deletes its
    /// destination before reflinking, so a self-copy would destroy the
    /// artifact it stages (water-rs/cli#291).
    #[test]
    fn canonical_stage_preserves_an_already_canonical_source() {
        let temporary = tempdir().expect("tempdir");
        let destination = temporary.path().join("deps");
        std::fs::create_dir_all(&destination).expect("deps dir");
        let source = destination.join("libwaterui_dylib.dylib");
        let bytes = dylib_bytes();
        std::fs::write(&source, &bytes).expect("reported artifact");
        let libraries = canonical_staging_libraries(source.clone());

        let staged = smol::block_on(libraries.stage_apple_canonical(&destination))
            .expect("already-canonical staging is a no-op");
        assert_eq!(staged, source);
        smol::block_on(libraries.stage_apple_canonical(&destination))
            .expect("restaging stays a no-op");
        assert_eq!(
            std::fs::read(&source).expect("read artifact"),
            bytes,
            "repeated canonical staging must preserve the artifact"
        );
    }

    /// A reported runtime living anywhere else still reflinks under the
    /// canonical name, source left intact.
    #[test]
    fn canonical_stage_copies_a_distinct_source() {
        let temporary = tempdir().expect("tempdir");
        let source = temporary
            .path()
            .join("deps/libwaterui_dylib-0123456789abcdef.dylib");
        let destination = temporary.path().join("dist");
        std::fs::create_dir_all(source.parent().expect("deps dir")).expect("deps dir");
        std::fs::create_dir_all(&destination).expect("dist dir");
        let bytes = dylib_bytes();
        std::fs::write(&source, &bytes).expect("reported artifact");
        let libraries = canonical_staging_libraries(source.clone());

        let staged = smol::block_on(libraries.stage_apple_canonical(&destination))
            .expect("distinct source stages");
        assert_eq!(staged, destination.join("libwaterui_dylib.dylib"));
        assert_eq!(std::fs::read(&staged).expect("read staged copy"), bytes);
        assert_eq!(
            std::fs::read(&source).expect("read source"),
            bytes,
            "staging leaves the reported source in place"
        );
    }

    /// A reported runtime that does not exist is a build defect, never a
    /// silent no-op — presence is validated at this artifact boundary.
    #[test]
    fn canonical_stage_errors_when_the_reported_source_is_missing() {
        let temporary = tempdir().expect("tempdir");
        let destination = temporary.path().join("deps");
        std::fs::create_dir_all(&destination).expect("deps dir");
        let source = destination.join("libwaterui_dylib.dylib");
        let error =
            smol::block_on(canonical_staging_libraries(source).stage_apple_canonical(&destination))
                .expect_err("a missing reported source must error");
        assert!(
            format!("{error:#}").contains("libwaterui_dylib.dylib"),
            "the error names the missing reported artifact: {error:#}"
        );
    }

    /// Cargo's build-dir layout (nightly 1.100) writes a unit's dep-info in
    /// `build/<package>/<hash>/out/` beside its other outputs instead of
    /// `<profile>/deps/`; the unit's `.rmeta` names that directory. A fresh
    /// proc-macro unit — hashed, never uplifted, and on that layout without
    /// any dep-info the `deps/` convention could find — takes no part.
    #[test]
    fn stale_check_reads_build_dir_dep_info_and_skips_proc_macros() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let profile = temporary.path().join("debug");
            let unit_dir = profile.join("build/waterui-dylib/0123456789abcdef/out");
            std::fs::create_dir_all(&unit_dir).expect("unit dir");
            let dylib = profile.join("libwaterui_dylib.so");
            std::fs::write(&dylib, []).expect("dylib");
            let rmeta = unit_dir.join("libwaterui_dylib.rmeta");
            std::fs::write(&rmeta, []).expect("rmeta");

            let ours = temporary.path().join("ours");
            std::fs::create_dir_all(ours.join("src")).expect("our manifest dir");
            let manifest = ours.join("Cargo.toml");
            std::fs::write(&manifest, "").expect("manifest");
            let foreign = temporary.path().join("foreign/src/lib.rs");
            std::fs::create_dir_all(foreign.parent().expect("parent")).expect("foreign dir");
            std::fs::write(&foreign, []).expect("foreign source");
            std::fs::write(
                unit_dir.join("waterui_dylib.d"),
                format!("{}: {}\n", dylib.display(), foreign.display()),
            )
            .expect("dep-info");

            let unit = |name: &str, crate_type: &str, filenames: Vec<&std::path::Path>| {
                serde_json::json!({
                    "reason": "compiler-artifact",
                    "package_id": format!("path+file:///x#{name}@0.1.0"),
                    "manifest_path": manifest,
                    "target": {
                        "kind": [if crate_type == "proc-macro" { "proc-macro" } else { "lib" }],
                        "crate_types": [crate_type],
                        "name": name.replace('-', "_"),
                        "src_path": ours.join("src/lib.rs"),
                        "edition": "2021",
                        "doc": true,
                        "doctest": true,
                        "test": true,
                    },
                    "profile": {
                        "opt_level": "0",
                        "debuginfo": 0,
                        "debug_assertions": true,
                        "overflow_checks": true,
                        "test": false,
                    },
                    "features": [],
                    "filenames": filenames,
                    "executable": null,
                    "fresh": true,
                })
                .to_string()
            };
            // The proc-macro's dylib exists nowhere on disk and has no
            // dep-info; only the dylib unit is examined, and its dep-info is
            // found through the `.rmeta` sibling's directory.
            let macro_dylib = unit_dir.join("libthiserror_impl-0123456789abcdef.so");
            let stdout = format!(
                "{}\n{}\n",
                unit("thiserror-impl", "proc-macro", vec![&macro_dylib]),
                unit("waterui-dylib", "dylib", vec![&dylib, &rmeta]),
            );
            // No linked artifact records the unit here — a static archive
            // carries no dynamic section — so the unhashed unit-directory
            // spelling is the record found.
            let stale = super::stale_shared_dylib_packages(stdout.as_bytes(), &[])
                .await
                .expect("scan");
            assert_eq!(stale.len(), 1, "{stale:?}");
            assert_eq!(stale[0].package, "waterui-dylib");
            assert!(
                matches!(
                    stale[0].reason,
                    super::StaleSharedDylibReason::ForeignDepInfo { .. }
                ),
                "{:?}",
                stale[0].reason
            );
        });
    }

    /// A fresh uplifted dylib with no dep-info beside it or in its unit
    /// directory is a cache this CLI wrote and can no longer account for. It
    /// is recovered — flagged so the package is cleaned and rebuilt — rather
    /// than reported as an error, and the reason names the missing record.
    #[test]
    fn fresh_uplifted_dylib_without_dep_info_is_recovered_not_reported() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let profile = temporary
                .path()
                .join("target/shared/x86_64-pc-windows-msvc/debug");
            let deps = profile.join("deps");
            std::fs::create_dir_all(&deps).expect("deps dir");
            let dylib = profile.join("waterui_dylib.dll");
            std::fs::write(&dylib, []).expect("dylib");
            let import_lib = profile.join("waterui_dylib.dll.lib");
            std::fs::write(&import_lib, []).expect("import lib");

            let ours = temporary.path().join("ours");
            std::fs::create_dir_all(ours.join("src")).expect("our manifest dir");
            let manifest = ours.join("Cargo.toml");
            std::fs::write(&manifest, "").expect("manifest");

            let stdout = serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "path+file:///x#waterui-dylib@0.1.0",
                "manifest_path": manifest,
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["dylib"],
                    "name": "waterui_dylib",
                    "src_path": ours.join("src/lib.rs"),
                    "edition": "2021",
                    "doc": true,
                    "doctest": true,
                    "test": true,
                },
                "profile": {
                    "opt_level": "0",
                    "debuginfo": 0,
                    "debug_assertions": true,
                    "overflow_checks": true,
                    "test": false,
                },
                "features": [],
                "filenames": [dylib, import_lib],
                "executable": null,
                "fresh": true,
            })
            .to_string();

            let stale = super::stale_shared_dylib_packages(stdout.as_bytes(), &[])
                .await
                .expect("a fresh dylib without dep-info is recovered, not reported");
            assert_eq!(
                stale,
                [super::StaleSharedDylib {
                    package: "waterui-dylib".to_owned(),
                    artifact: dylib.clone(),
                    reason: super::StaleSharedDylibReason::MissingDepInfo {
                        reported_files: vec![dylib.clone(), import_lib.clone()],
                    },
                }]
            );
            let reason = stale[0].reason.to_string();
            assert!(reason.contains("no dep-info was found"), "{reason}");

            // A second pass in the same state after the rebuild is the loud
            // failure, naming the artifact, the package, and the target
            // directory to drop.
            let target_dir = temporary.path().join("target/shared");
            let error = super::unrecoverable_shared_dylib_error(&stale, &target_dir).to_string();
            assert!(
                error.contains("after its package was cleaned and rebuilt"),
                "{error}"
            );
            assert!(error.contains("waterui-dylib"), "{error}");
            assert!(error.contains(&dylib.display().to_string()), "{error}");
            assert!(error.contains(&target_dir.display().to_string()), "{error}");
        });
    }

    /// Dep-info prerequisites arrive in Makefile spelling: `\ ` escapes a
    /// literal space, a `\` at end of line continues the rule, and a Windows
    /// drive-letter colon is data — only the first `": "` separates the
    /// target. rustc escapes nothing else, so `$$` and `\\` stay verbatim.
    #[test]
    fn dep_info_prerequisites_unescape_spaces_and_join_continued_rules() {
        let contents = concat!(
            "C:\\out\\app.dll: C:\\work\\my\\ app\\src\\lib.rs \\\n",
            "    C:\\work\\my\\ app\\build.rs C:\\work\\cost$$.rs\n",
            "\n",
            "C:\\work\\my\\ app\\src\\lib.rs:\n",
        );
        assert_eq!(
            super::dep_info_prerequisites(contents),
            vec![
                PathBuf::from("C:\\work\\my app\\src\\lib.rs"),
                PathBuf::from("C:\\work\\my app\\build.rs"),
                PathBuf::from("C:\\work\\cost$$.rs"),
            ]
        );
    }

    #[test]
    fn static_packaging_removes_only_staged_android_runtime_libraries() {
        smol::block_on(async {
            let directory = tempdir().expect("temporary Android runtime directory");
            let android_triple = triple("aarch64-linux-android");
            for file_name in [
                "libwaterui_dylib.so",
                "libstd-old.so",
                "libwaterui_app.so",
                "libc++_shared.so",
            ] {
                std::fs::write(directory.path().join(file_name), [])
                    .expect("write staged runtime test file");
            }

            RustDynamicLibraries::remove_staged(directory.path(), &android_triple)
                .await
                .expect("remove shared Rust runtime libraries");

            assert!(!directory.path().join("libwaterui_dylib.so").exists());
            assert!(!directory.path().join("libstd-old.so").exists());
            assert!(directory.path().join("libwaterui_app.so").exists());
            assert!(directory.path().join("libc++_shared.so").exists());
        });
    }

    /// The crate proving a rustflags source reaches rustc: its `lib`
    /// compiles only when `--cfg <probe>` arrives, and its build script
    /// echoes the `CARGO_ENCODED_RUSTFLAGS` the build was handed — the union
    /// ordering and content in one marker.
    fn rustflags_probe_crate(crate_dir: &std::path::Path, probe: &str) {
        std::fs::create_dir_all(crate_dir.join("src")).expect("crate dir");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("manifest");
        std::fs::write(
            crate_dir.join("build.rs"),
            "fn main() {\n    println!(\n        \"cargo:warning=RFPROBE={}\",\n        std::env::var(\"CARGO_ENCODED_RUSTFLAGS\").unwrap_or_default()\n    );\n}\n",
        )
        .expect("build script");
        std::fs::write(
            crate_dir.join("src/lib.rs"),
            format!(
                "#[cfg(not({probe}))]\ncompile_error!(\"a rustflags source was dropped by the build\");\n"
            ),
        )
        .expect("lib.rs");
    }

    /// An empty `$CARGO_HOME` keeps a real user configuration out of the
    /// resolution; an ambient `RUSTFLAGS` in the test process's environment
    /// still applies — exactly as it would to a real cargo build.
    fn rustflags_probe_build(
        crate_dir: &std::path::Path,
        cargo_home: &std::path::Path,
    ) -> super::RustBuild {
        super::RustBuild::new(crate_dir, Triple::host())
            .with_preferred_dynamic_linking()
            .with_env("CARGO_HOME", cargo_home)
    }

    /// `cargo_build_output` for a probe build: resolves the user's
    /// rustflags the way `build_inner` does once per build, then runs the
    /// lib build so `RFPROBE` echoes the union rustc received.
    async fn probe_build_output(build: &super::RustBuild) -> std::process::Output {
        let config_files = build
            .project_cargo_config_files()
            .expect("project config files resolve");
        let user_rustflags = build
            .user_rustflags(&config_files)
            .await
            .expect("rustflags resolve");
        build
            .cargo_build_output(false, CargoTarget::Lib, &user_rustflags)
            .await
            .expect("the probe crate builds")
    }

    /// The `RFPROBE=` marker's encoded value from the build output.
    fn probed_rustflags(output: &std::process::Output) -> String {
        let log = combined_build_output(output);
        assert!(output.status.success(), "{log}");
        let marker = log
            .find("RFPROBE=")
            .expect("the build script echo reached the output");
        log[marker + "RFPROBE=".len()..]
            .chars()
            .take_while(|character| !character.is_whitespace())
            .collect()
    }

    /// `[build] rustflags` in a project's `.cargo/config.toml` joins the
    /// CLI's own `-Cprefer-dynamic`/`-Crpath` instead of being shadowed by
    /// them — the probe compiles and both halves of the union reach rustc.
    #[test]
    fn build_rustflags_join_the_cli_flags() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let fixture = temporary.path().join("fixture");
            let crate_dir = fixture.join("crate");
            rustflags_probe_crate(&crate_dir, "water_config_probe");
            std::fs::create_dir_all(fixture.join(".cargo")).expect("config dir");
            std::fs::write(
                fixture.join(".cargo").join("config.toml"),
                "[build]\nrustflags = [\"--cfg=water_config_probe\"]\n",
            )
            .expect("config");

            let output = probe_build_output(&rustflags_probe_build(
                &crate_dir,
                &temporary.path().join("cargo-home"),
            ))
            .await;

            let rustflags = probed_rustflags(&output);
            let config = rustflags.find("--cfg=water_config_probe");
            let prefer_dynamic = rustflags.find("-Cprefer-dynamic");
            assert!(
                config.is_some() && prefer_dynamic.is_some(),
                "the union carries both flag sets: {rustflags}"
            );
            assert!(
                config < prefer_dynamic,
                "the user's resolved flags apply before the CLI's: {rustflags}"
            );
        });
    }

    /// `[target.<host triple>] rustflags` reaches the build the same way —
    /// the target table's flags join the union ahead of the CLI's own.
    #[test]
    fn target_rustflags_join_the_cli_flags() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let fixture = temporary.path().join("fixture");
            let crate_dir = fixture.join("crate");
            rustflags_probe_crate(&crate_dir, "water_target_probe");
            let host_triple = Triple::host().to_string();
            let mut config = toml_edit::DocumentMut::new();
            config["target"][&host_triple]["rustflags"] = toml_edit::Item::Value(
                toml_edit::Array::from_iter(["--cfg=water_target_probe"]).into(),
            );
            std::fs::create_dir_all(fixture.join(".cargo")).expect("config dir");
            std::fs::write(
                fixture.join(".cargo").join("config.toml"),
                config.to_string(),
            )
            .expect("config");

            let output = probe_build_output(&rustflags_probe_build(
                &crate_dir,
                &temporary.path().join("cargo-home"),
            ))
            .await;

            let rustflags = probed_rustflags(&output);
            assert!(
                rustflags.contains("--cfg=water_target_probe")
                    && rustflags.contains("-Cprefer-dynamic"),
                "the union carries both flag sets: {rustflags}"
            );
        });
    }

    /// `RUSTFLAGS` in the build's environment — a `with_env` entry the old
    /// code overwrote when it wrote its own `RUSTFLAGS` — survives alongside
    /// the CLI's flags.
    #[test]
    fn env_rustflags_join_the_cli_flags() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let crate_dir = temporary.path().join("crate");
            rustflags_probe_crate(&crate_dir, "water_env_probe");

            let output = probe_build_output(
                &rustflags_probe_build(&crate_dir, &temporary.path().join("cargo-home"))
                    .with_env("RUSTFLAGS", "--cfg=water_env_probe"),
            )
            .await;

            let rustflags = probed_rustflags(&output);
            assert!(
                rustflags.contains("--cfg=water_env_probe")
                    && rustflags.contains("-Cprefer-dynamic"),
                "the union carries both flag sets: {rustflags}"
            );
        });
    }

    /// The `--config` files a managed build passes — the project config
    /// hierarchy the managed crate sits outside of — feed the resolution the
    /// same way discovered files do.
    #[test]
    fn cli_config_files_feed_the_resolution() {
        smol::block_on(async {
            let temporary = tempdir().expect("tempdir");
            let project = temporary.path().join("project");
            std::fs::create_dir_all(project.join(".cargo")).expect("config dir");
            let config = project.join(".cargo").join("config.toml");
            std::fs::write(
                &config,
                "[build]\nrustflags = [\"--cfg=water_cli_probe\"]\n",
            )
            .expect("config");
            let crate_dir = temporary.path().join("crate");
            std::fs::create_dir_all(&crate_dir).expect("crate dir");

            let flags = super::RustBuild::new(&crate_dir, Triple::host())
                .with_env("CARGO_HOME", temporary.path().join("cargo-home"))
                .user_rustflags(&[config])
                .await
                .expect("the cli config layer resolves");

            assert_eq!(flags, ["--cfg=water_cli_probe"]);
        });
    }

    /// `water build --release` on an app with an `include_bundle!` mount must
    /// stage the mount's files: the mount plan is read from the
    /// `waterui_meta_bundle_*` statics in the release build's own rlib, and
    /// the framework emits them in every profile. Before the metadata was
    /// emitted under `debug_assertions` a release build carried none and the
    /// staged mount came out empty (water-rs/waterui#1272).
    ///
    /// The project resolves the framework from a local checkout — the pinned
    /// revision by default, or `WATERUI_PATH` to exercise a working tree — so
    /// the test builds the same `waterui` commit the crate's git deps pin.
    #[test]
    #[ignore = "builds a release app against the enclosing workspace"]
    fn release_build_stages_include_bundle_mounts() {
        smol::block_on(async {
            let checkout = crate::toolchain::host::Host::current()
                .env("WATERUI_PATH")
                .map_or_else(
                    || crate::pinned_framework::checkout().to_path_buf(),
                    PathBuf::from,
                );
            let temporary = tempdir().expect("tempdir");
            let root = temporary.path().join("release-app");
            let project = crate::project::Project::create(
                &root,
                crate::project::CreateOptions {
                    name: "Release App".to_string(),
                    bundle_identifier:
                        crate::project_model::project_types::BundleIdentifier::try_from(
                            "dev.waterui.releaseapp",
                        )
                        .expect("bundle identifier"),
                    waterui_path: Some(checkout),
                    channel: None,
                    framework_manifest: None,
                    framework: None,
                    framework_lock: None,
                    author: String::new(),
                    web: None,
                },
            )
            .await
            .expect("project creation must succeed");

            std::fs::create_dir_all(root.join("bundle")).expect("bundle dir");
            std::fs::write(root.join("bundle/hello.txt"), "fixture asset\n").expect("asset");
            let lib_rs = root.join("src/lib.rs");
            let mut source = std::fs::read_to_string(&lib_rs).expect("lib.rs");
            source.push_str("\nwaterui::include_bundle!(\"bundle\", as = media);\n");
            std::fs::write(&lib_rs, source).expect("write lib.rs");

            crate::backend::reinit_backend::<crate::hydrolysis::backend::HydrolysisBackend>(
                &project,
            )
            .await
            .expect("hydrolysis backend scaffolds");
            crate::hydrolysis::platform::build_hydrolysis(
                &project,
                crate::platform::TargetPlatform::Linux,
                BuildOptions::development(BuildProfile::Release),
            )
            .await
            .expect("release build must succeed");

            let staged = project
                .backend_path::<crate::hydrolysis::backend::HydrolysisBackend>()
                .join("resources/waterui_assets/media/hello.txt");
            assert!(
                staged.metadata().is_ok_and(|meta| meta.len() > 0),
                "release build stages a non-empty mount at {}",
                staged.display()
            );
        });
    }

    #[test]
    fn dxc_runtime_resolution_collects_the_pair_beside_dxc() {
        let directory = tempdir().expect("temporary dxc directory");
        for name in ["dxcompiler.dll", "dxil.dll"] {
            std::fs::write(directory.path().join(name), []).expect("write runtime stub");
        }

        assert_eq!(
            resolve_dxc_runtime_in(directory.path()).expect("resolve dxc runtime pair"),
            vec![
                directory.path().join("dxcompiler.dll"),
                directory.path().join("dxil.dll"),
            ]
        );
    }

    #[test]
    fn dxc_runtime_resolution_names_the_missing_library() {
        let directory = tempdir().expect("temporary dxc directory");
        std::fs::write(directory.path().join("dxcompiler.dll"), []).expect("write runtime stub");

        let error = resolve_dxc_runtime_in(directory.path())
            .expect_err("a missing dxil.dll must fail resolution");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(error.to_string().contains("dxil.dll"), "{error}");
    }

    #[test]
    fn static_packaging_removes_staged_shader_compiler_libraries() {
        smol::block_on(async {
            let directory = tempdir().expect("temporary Windows runtime directory");
            let windows_triple = triple("x86_64-pc-windows-msvc");
            for file_name in [
                "waterui_dylib.dll",
                "std-1234567890abcdef.dll",
                "dxcompiler.dll",
                "dxil.dll",
                "keep.dll",
            ] {
                std::fs::write(directory.path().join(file_name), [])
                    .expect("write staged runtime test file");
            }

            RustDynamicLibraries::remove_staged(directory.path(), &windows_triple)
                .await
                .expect("remove shared Rust runtime libraries");

            for file_name in [
                "waterui_dylib.dll",
                "std-1234567890abcdef.dll",
                "dxcompiler.dll",
                "dxil.dll",
            ] {
                assert!(!directory.path().join(file_name).exists(), "{file_name}");
            }
            assert!(directory.path().join("keep.dll").exists());
        });
    }
}

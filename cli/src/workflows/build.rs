//! Build system

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{self, Context as _, bail};
use futures::StreamExt as _;
use sha2::Digest as _;
use smol::{process::Command, unblock};
use target_lexicon::{Environment, OperatingSystem, Triple};

use crate::utils::{command, run_command};

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

/// Resolve the Rust standard-library directory for a target triple.
///
/// # Errors
/// Returns an error if rustc cannot resolve an existing target library directory.
pub async fn rust_target_libdir(triple: &Triple) -> eyre::Result<PathBuf> {
    let target = triple.to_string();
    let output = run_command(
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
/// target kind in the type keeps `cargo rustc -- --crate-type` from ever reaching a
/// binary build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CargoTarget<'a> {
    /// The crate's library target.
    Lib,
    /// One named binary target.
    Binary(&'a str),
}

impl<'a> CargoTarget<'a> {
    fn cargo_args(self) -> Vec<&'a str> {
        match self {
            Self::Lib => vec!["--lib"],
            Self::Binary(name) => vec!["--bin", name],
        }
    }

    const fn accepts_crate_type_override(self) -> bool {
        matches!(self, Self::Lib)
    }
}

/// Selects how Rust dependencies are linked into a native application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustLinkage {
    /// Link the `WaterUI` runtime into the application archive.
    Static,
    /// Link the application and loadable modules against one shared `WaterUI` runtime.
    SharedRuntime,
}

#[derive(Debug, Clone)]
struct ResolvedRuntimeNode {
    features: Vec<String>,
    dependencies: Vec<String>,
}

/// Resolve a stable fingerprint for the shared `WaterUI` runtime selected by a Cargo build.
///
/// The fingerprint includes the complete resolved dependency subtree rooted at
/// `waterui-dylib`, its unified feature set, and the build target. Projects with
/// the same runtime graph can therefore share Cargo artifacts without allowing
/// incompatible runtime variants to overwrite each other.
///
/// # Errors
/// Returns an error when Cargo metadata cannot be resolved or does not contain
/// exactly one active `waterui-dylib` package and its complete dependency graph.
pub async fn shared_rust_runtime_fingerprint(
    manifest_path: &Path,
    build_features: &[String],
    triple: &Triple,
) -> eyre::Result<String> {
    let manifest_path = manifest_path.to_path_buf();
    let build_features = build_features.to_vec();
    let target = triple.to_string();
    let metadata_target = target.clone();
    let metadata = unblock(move || {
        let mut command = cargo_metadata::MetadataCommand::new();
        command
            .manifest_path(&manifest_path)
            .features(cargo_metadata::CargoOpt::SomeFeatures(build_features))
            .other_options(vec!["--filter-platform".to_string(), metadata_target]);
        command.exec()
    })
    .await
    .wrap_err("Failed to resolve Cargo metadata for the shared WaterUI runtime")?;

    let resolve = metadata.resolve.as_ref().ok_or_else(|| {
        eyre::eyre!("Cargo metadata omitted the dependency graph for the shared WaterUI runtime")
    })?;
    let active_package_ids = resolve
        .nodes
        .iter()
        .map(|node| node.id.to_string())
        .collect::<BTreeSet<_>>();
    let runtime_ids = metadata
        .packages
        .iter()
        .filter(|package| package.name == "waterui-dylib")
        .map(|package| package.id.to_string())
        .filter(|id| active_package_ids.contains(id))
        .collect::<Vec<_>>();
    let runtime_id = match runtime_ids.as_slice() {
        [runtime_id] => runtime_id,
        [] => {
            bail!("Development feature did not resolve an active `waterui-dylib` package");
        }
        _ => {
            bail!(
                "Development feature resolved multiple `waterui-dylib` packages; the shared runtime must be unique"
            );
        }
    };

    let graph = resolve
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.to_string(),
                ResolvedRuntimeNode {
                    features: node.features.iter().map(ToString::to_string).collect(),
                    dependencies: node.dependencies.iter().map(ToString::to_string).collect(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    fingerprint_resolved_runtime_graph(runtime_id, &target, &graph)
}

fn fingerprint_resolved_runtime_graph(
    runtime_id: &str,
    target: &str,
    graph: &BTreeMap<String, ResolvedRuntimeNode>,
) -> eyre::Result<String> {
    let mut pending = vec![runtime_id.to_string()];
    let mut visited = BTreeSet::new();
    let mut units = Vec::new();

    while let Some(package_id) = pending.pop() {
        if !visited.insert(package_id.clone()) {
            continue;
        }
        let node = graph.get(&package_id).ok_or_else(|| {
            eyre::eyre!("Cargo metadata omitted runtime dependency node `{package_id}`")
        })?;
        let mut features = node.features.clone();
        features.sort_unstable();
        features.dedup();
        units.push(format!("{package_id}|{}", features.join(",")));
        pending.extend(node.dependencies.iter().cloned());
    }

    units.sort_unstable();
    let mut hasher = sha2::Sha256::new();
    hasher.update(target.as_bytes());
    hasher.update(b"\n");
    for unit in units {
        hasher.update(unit.as_bytes());
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
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
/// such as whether a machine has `sccache` installed. Little is given up: these crates
/// build into runtime-fingerprinted target directories that rotate whenever the dependency
/// graph moves, so incremental state rarely survives to be reused — while an `sccache`
/// entry, which requires incremental to be off, does.
pub fn configure_generated_crate_compilation(command: &mut Command) {
    command.env("CARGO_INCREMENTAL", "0");
}

/// Configure a Cargo command for the selected Rust linkage model.
pub fn configure_cargo_linkage(
    command: &mut Command,
    linkage: RustLinkage,
    development_feature: &str,
    loader_search_path: Option<&str>,
) {
    if linkage == RustLinkage::Static {
        return;
    }

    command.args(["--features", development_feature]);
    let mut rustflags = std::env::var_os("RUSTFLAGS").unwrap_or_default();
    if !rustflags.is_empty() {
        rustflags.push(" ");
    }
    rustflags.push("-Cprefer-dynamic -Crpath");
    if let Some(path) = loader_search_path {
        rustflags.push(format!(" -Clink-arg=-Wl,-rpath,{path}"));
    }
    command.env("RUSTFLAGS", rustflags);
}

/// Dynamic Rust libraries required by a shared-runtime development build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustDynamicLibraries {
    waterui: PathBuf,
    standard_library: PathBuf,
    triple: Triple,
}

impl RustDynamicLibraries {
    /// Resolve the shared `WaterUI` runtime and target Rust standard library.
    ///
    /// # Errors
    /// Returns an error when either required dynamic library is absent or ambiguous.
    pub async fn resolve(lib_dir: &Path, triple: &Triple) -> eyre::Result<Self> {
        let waterui = lib_dir
            .join("deps")
            .join(dynamic_library_file_name("waterui_dylib", triple));
        if !waterui.is_file() {
            bail!(
                "Shared WaterUI runtime was not built at {}",
                waterui.display()
            );
        }

        let target_libdir = rust_target_libdir(triple).await?;
        let resolution_triple = triple.clone();
        let standard_library =
            unblock(move || resolve_rust_standard_library_in(&target_libdir, &resolution_triple))
                .await?;

        Ok(Self {
            waterui,
            standard_library,
            triple: triple.clone(),
        })
    }

    /// Shared `WaterUI` runtime path.
    #[must_use]
    pub fn waterui(&self) -> &Path {
        &self.waterui
    }

    /// Target Rust standard-library dynamic library path.
    #[must_use]
    pub fn standard_library(&self) -> &Path {
        &self.standard_library
    }

    /// Iterate over every library that must be staged with the application.
    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        [self.waterui(), self.standard_library()].into_iter()
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
        Self::remove_staged(destination, &self.triple).await?;
        for source in self.iter() {
            let file_name = source.file_name().ok_or_else(|| {
                eyre::eyre!(
                    "Dynamic library path has no file name: {}",
                    source.display()
                )
            })?;
            crate::utils::copy_file(source, destination.join(file_name)).await?;
        }
        Ok(())
    }

    /// Remove shared-runtime libraries left by an earlier development build.
    ///
    /// # Errors
    /// Returns an error when the destination cannot be read or a matching library cannot be removed.
    pub async fn remove_staged(destination: &Path, triple: &Triple) -> eyre::Result<()> {
        if !destination.is_dir() {
            return Ok(());
        }

        let waterui = dynamic_library_file_name("waterui_dylib", triple);
        let (standard_library_prefix, extension) =
            if triple.operating_system == OperatingSystem::Windows {
                ("std-", "dll")
            } else {
                ("libstd-", lib_extension_for_triple(triple))
            };
        let mut entries = smol::fs::read_dir(destination).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name == waterui
                || (file_name.starts_with(standard_library_prefix)
                    && entry.path().extension().and_then(|value| value.to_str()) == Some(extension))
            {
                smol::fs::remove_file(entry.path()).await?;
            }
        }
        Ok(())
    }
}

fn dynamic_library_file_name(crate_name: &str, triple: &Triple) -> String {
    if triple.operating_system == OperatingSystem::Windows {
        format!("{crate_name}.dll")
    } else {
        format!("lib{crate_name}.{}", lib_extension_for_triple(triple))
    }
}

fn resolve_rust_standard_library_in(libdir: &Path, triple: &Triple) -> eyre::Result<PathBuf> {
    let (prefix, extension) = if triple.operating_system == OperatingSystem::Windows {
        ("std-", "dll")
    } else {
        ("libstd-", lib_extension_for_triple(triple))
    };
    let entries = std::fs::read_dir(libdir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut matches = entries
        .into_iter()
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
        [] => {
            bail!(
                "Rust target libdir {} contains no dynamic standard library for {triple}",
                libdir.display()
            );
        }
        _ => {
            bail!(
                "Rust target libdir {} contains multiple dynamic standard libraries for {triple}: {}",
                libdir.display(),
                matches
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}

/// Represents a Rust build for a specific target triple.
#[derive(Debug, Clone)]
pub struct RustBuild {
    path: PathBuf,
    triple: Triple,
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
    /// Extra environment variables to set for the cargo build process.
    envs: Vec<(String, OsString)>,
}

/// Options for building Rust libraries.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    release: bool,
    output_dir: Option<std::path::PathBuf>,
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<std::path::PathBuf>,
    /// Optional target triple override.
    target_triple: Option<Triple>,
    /// Rust runtime linkage used by the final native application.
    linkage: RustLinkage,
}

impl BuildOptions {
    /// Create options for a development build that uses the shared Rust runtime.
    #[must_use]
    pub const fn development(release: bool) -> Self {
        Self {
            release,
            output_dir: None,
            sccache_path: None,
            target_triple: None,
            linkage: RustLinkage::SharedRuntime,
        }
    }

    /// Create options for a self-contained package build.
    #[must_use]
    pub const fn packaging(release: bool) -> Self {
        Self {
            release,
            output_dir: None,
            sccache_path: None,
            target_triple: None,
            linkage: RustLinkage::Static,
        }
    }

    /// Whether to build in release mode
    #[must_use]
    pub const fn is_release(&self) -> bool {
        self.release
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

impl RustBuild {
    /// Create a new rust build for the given path and target triple.
    pub fn new(path: impl AsRef<Path>, triple: Triple) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            triple,
            target_dir: None,
            sccache_path: None,
            features: Vec::new(),
            crate_type_override: None,
            rustc_flags: Vec::new(),
            envs: Vec::new(),
        }
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

    /// Add a rustc flag to the build via `RUSTFLAGS`.
    #[must_use]
    pub fn with_rustc_flag(mut self, flag: impl Into<String>) -> Self {
        self.rustc_flags.push(flag.into());
        self
    }

    /// Prefer dynamic Rust dependencies and emit loader search paths for them.
    #[must_use]
    pub fn with_preferred_dynamic_linking(self) -> Self {
        self.with_rustc_flag("-Cprefer-dynamic")
            .with_rustc_flag("-Crpath")
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

    /// Get the target triple for this build.
    #[must_use]
    pub const fn triple(&self) -> &Triple {
        &self.triple
    }

    /// Build rust library in development mode.
    ///
    /// Will produce debug symbols and less optimizations for faster builds.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn dev_build(&self) -> Result<PathBuf, RustBuildError> {
        self.build_lib(false).await
    }

    /// Build rust library in release mode.
    ///
    /// Return the directory path containing the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn release_build(&self) -> Result<PathBuf, RustBuildError> {
        self.build_lib(true).await
    }

    /// Build a library with the specified crate type.
    ///
    /// Return the directory path containing the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn build_lib(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        self.build_inner(release, CargoTarget::Lib).await
    }

    /// Build a dynamic library (cdylib) and return the full path to the dylib file.
    ///
    /// This is a convenience method that builds the library and computes the full
    /// path to the resulting dylib file based on the crate name and target triple.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If the library was not found after building.
    pub async fn build_dylib(
        &self,
        crate_name: &str,
        release: bool,
    ) -> Result<PathBuf, RustBuildError> {
        let lib_dir = self.build_inner(release, CargoTarget::Lib).await?;

        let lib_name = crate_name.replace('-', "_");
        let ext = lib_extension_for_triple(&self.triple);
        let dylib_path = lib_dir.join(format!("lib{lib_name}.{ext}"));

        if !dylib_path.exists() {
            return Err(RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Dynamic library not found at {} after cargo build",
                    dylib_path.display()
                ),
            )));
        }

        Ok(dylib_path)
    }

    /// Builds one named binary and returns its full output path.
    ///
    /// # Errors
    ///
    /// Returns an error when Cargo fails or the expected binary is missing.
    pub async fn build_binary(
        &self,
        binary_name: &str,
        release: bool,
    ) -> Result<PathBuf, RustBuildError> {
        let output_dir = self
            .build_inner(release, CargoTarget::Binary(binary_name))
            .await?;
        let binary_path = output_dir.join(binary_name);
        if !binary_path.is_file() {
            return Err(RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Binary not found at {} after cargo build",
                    binary_path.display()
                ),
            )));
        }
        Ok(binary_path)
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
    ) -> Result<PathBuf, RustBuildError> {
        let mut output = self.cargo_build_output(release, cargo_target).await?;

        if !output.status.success() {
            let mut combined = combined_build_output(&output);

            // Handle stale CMake generator caches (e.g. Unix Makefiles vs Ninja)
            // by cleaning crate-local CMake build dirs and retrying once.
            if should_retry_after_cmake_generator_mismatch(&combined)
                && self.clean_stale_cmake_build_dirs().await?
            {
                output = self.cargo_build_output(release, cargo_target).await?;
                combined = combined_build_output(&output);
            }

            if !output.status.success() && should_auto_install_meson(&combined) {
                match ensure_meson_installed_for_build().await {
                    Ok(()) => {
                        output = self.cargo_build_output(release, cargo_target).await?;
                    }
                    Err(install_err) => {
                        return Err(RustBuildError::FailToBuildRustLibrary(
                            std::io::Error::other(format!(
                                "Cargo build failed and meson appears missing.\n\
Automatic meson installation failed: {install_err}\n\n{combined}"
                            )),
                        ));
                    }
                }
            }
        }

        if !output.status.success() {
            let combined = combined_build_output(&output);
            return Err(RustBuildError::FailToBuildRustLibrary(
                std::io::Error::other(format!("Cargo build failed:\n{combined}")),
            ));
        }

        self.lib_output_dir(release).await
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

    async fn cargo_build_output(
        &self,
        release: bool,
        cargo_target: CargoTarget<'_>,
    ) -> Result<std::process::Output, RustBuildError> {
        let crate_type_override = if cargo_target.accepts_crate_type_override() {
            self.crate_type_override.as_deref()
        } else {
            None
        };
        let mut cmd = Command::new("cargo");
        let cargo_subcommand = if crate_type_override.is_some() {
            "rustc"
        } else {
            "build"
        };
        let mut cmd = command(&mut cmd)
            .arg(cargo_subcommand)
            .args(cargo_target.cargo_args())
            .args(["--target", self.triple.to_string().as_str()])
            .current_dir(&self.path);

        if let Some(target_dir) = &self.target_dir {
            cmd = cmd.arg("--target-dir").arg(target_dir);
        }

        // Apply extra environment variables (caller-provided values override defaults).
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }

        if !self.rustc_flags.is_empty() {
            let mut rustflags = std::env::var_os("RUSTFLAGS").unwrap_or_default();
            if !rustflags.is_empty() {
                rustflags.push(" ");
            }
            rustflags.push(self.rustc_flags.join(" "));
            cmd = cmd.env("RUSTFLAGS", rustflags);
        }

        configure_generated_crate_compilation(cmd);

        // Use sccache as rustc wrapper if configured
        if let Some(sccache_path) = &self.sccache_path {
            crate::toolchain::sccache::configure_compilation_cache(cmd, sccache_path);
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

        if let Some(crate_type) = crate_type_override {
            cmd = cmd.arg("--").arg("--crate-type").arg(crate_type);
        }

        let output = cmd
            .output()
            .await
            .map_err(RustBuildError::FailToExecuteCargoBuild)?;
        Ok(output)
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

fn combined_build_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.is_empty() {
        stdout.to_string()
    } else {
        stderr.to_string()
    }
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

    match Meson.check().await {
        Ok(()) => Ok(()),
        Err(ToolchainError::Fixable(installation)) => {
            installation.install().await.map_err(|e| e.to_string())
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
    use target_lexicon::Triple;
    use tempfile::tempdir;

    use super::{
        BuildOptions, CargoTarget, ResolvedRuntimeNode, RustDynamicLibraries, RustLinkage,
        dynamic_library_file_name, fingerprint_resolved_runtime_graph, lib_extension_for_triple,
        resolve_rust_standard_library_in,
    };
    use std::collections::BTreeMap;

    fn triple(value: &str) -> Triple {
        value.parse().expect("test target triple must parse")
    }

    #[test]
    fn crate_type_override_applies_only_to_library_targets() {
        assert!(CargoTarget::Lib.accepts_crate_type_override());
        assert!(!CargoTarget::Binary("waterui-cef-helper").accepts_crate_type_override());
        assert_eq!(CargoTarget::Lib.cargo_args(), ["--lib"]);
        assert_eq!(
            CargoTarget::Binary("waterui-cef-helper").cargo_args(),
            ["--bin", "waterui-cef-helper"]
        );
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
            BuildOptions::development(false).linkage(),
            RustLinkage::SharedRuntime
        );
        assert_eq!(
            BuildOptions::packaging(false).linkage(),
            RustLinkage::Static
        );
        assert!(BuildOptions::development(true).is_release());
        assert!(BuildOptions::packaging(true).is_release());
    }

    #[test]
    fn shared_runtime_fingerprint_ignores_unrelated_application_nodes() {
        let runtime = "path+file:///waterui/utils/dylib#waterui-dylib@0.2.1";
        let internal = "path+file:///waterui/src#waterui-internal@0.2.1";
        let app = "path+file:///apps/menu#menu-example@0.1.0";
        let mut graph = BTreeMap::from([
            (
                runtime.to_string(),
                ResolvedRuntimeNode {
                    features: vec!["gpu".to_string(), "assets".to_string()],
                    dependencies: vec![internal.to_string()],
                },
            ),
            (
                internal.to_string(),
                ResolvedRuntimeNode {
                    features: vec!["gpu".to_string()],
                    dependencies: Vec::new(),
                },
            ),
        ]);
        let expected = fingerprint_resolved_runtime_graph(runtime, "aarch64-apple-darwin", &graph)
            .expect("runtime fingerprint");
        graph.insert(
            app.to_string(),
            ResolvedRuntimeNode {
                features: vec!["dev".to_string()],
                dependencies: vec![runtime.to_string()],
            },
        );

        assert_eq!(
            fingerprint_resolved_runtime_graph(runtime, "aarch64-apple-darwin", &graph)
                .expect("runtime fingerprint with unrelated app"),
            expected
        );
    }

    #[test]
    fn shared_runtime_fingerprint_separates_features_dependencies_and_targets() {
        let runtime = "registry+https://github.com/rust-lang/crates.io-index#waterui-dylib@0.2.1";
        let dependency = "registry+https://github.com/rust-lang/crates.io-index#wgpu@29.0.4";
        let graph = BTreeMap::from([
            (
                runtime.to_string(),
                ResolvedRuntimeNode {
                    features: vec!["gpu".to_string()],
                    dependencies: vec![dependency.to_string()],
                },
            ),
            (
                dependency.to_string(),
                ResolvedRuntimeNode {
                    features: Vec::new(),
                    dependencies: Vec::new(),
                },
            ),
        ]);
        let base = fingerprint_resolved_runtime_graph(runtime, "aarch64-apple-darwin", &graph)
            .expect("base runtime fingerprint");
        assert_ne!(
            fingerprint_resolved_runtime_graph(runtime, "x86_64-unknown-linux-gnu", &graph)
                .expect("target runtime fingerprint"),
            base
        );

        let mut changed_features = graph.clone();
        changed_features
            .get_mut(runtime)
            .expect("runtime node")
            .features
            .push("media".to_string());
        assert_ne!(
            fingerprint_resolved_runtime_graph(runtime, "aarch64-apple-darwin", &changed_features)
                .expect("feature runtime fingerprint"),
            base
        );

        let mut changed_dependency = graph;
        changed_dependency
            .get_mut(dependency)
            .expect("dependency node")
            .features
            .push("metal".to_string());
        assert_ne!(
            fingerprint_resolved_runtime_graph(
                runtime,
                "aarch64-apple-darwin",
                &changed_dependency
            )
            .expect("dependency runtime fingerprint"),
            base
        );
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
}

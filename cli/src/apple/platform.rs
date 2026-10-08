//! Apple platform build and package utilities.
//!
//! This module provides utility functions for building and packaging Apple apps.
//! These functions are used by `AppleBackend` to implement the `Backend` trait.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use eyre::{Context, bail};
use smol::fs;
use tracing::info;

#[cfg(target_os = "macos")]
use crate::browser_runtime;
#[cfg(target_os = "macos")]
use crate::macos_bundle::{package_cef_helper_app, remove_cef_helper_apps};
#[cfg(target_os = "macos")]
use crate::utils::run_command_os;
use crate::{
    apple::app_bundle,
    apple::backend::AppleBackend,
    apple::dynamic_runtime,
    assets,
    build::{BuildOptions, BuiltTarget, RustBuild, RustDynamicLibraries, RustLinkage},
    device::Artifact,
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::{BrowserRuntimePlan, Project, ResolvedWebViewBackend},
    utils::copy_file,
};

/// The generated FFI crate's application binary — the `[[bin]]` target
/// `src/bin/waterui-apple-main.rs` declares.
///
/// Entry-owning Apple packaging installs it as the bundle executable; the
/// library target stays for the embedding path.
pub const APPLE_ENTRY_BINARY_NAME: &str = "waterui-apple-main";

/// Validate the architecture supported by every Apple build and package path.
///
/// # Errors
/// Returns a diagnostic for any architecture other than ARM64.
pub fn validate_architecture(architecture: target_lexicon::Architecture) -> eyre::Result<()> {
    if architecture
        != target_lexicon::Architecture::Aarch64(target_lexicon::Aarch64Architecture::Aarch64)
    {
        bail!("Apple targets only support arm64; unsupported architecture {architecture}");
    }
    Ok(())
}

// ============================================================================
// Build Utilities
// ============================================================================

/// The library shape an Apple build hands to Xcode.
///
/// A packaged app links the runtime into itself and needs a self-contained archive. A
/// development build resolves the runtime from `libwaterui_dylib.dylib` at load time, so
/// the archive's contents are redundant there: `ld` satisfies the symbols from the dylib
/// and pulls almost nothing out of the archive, which is why the shipped executable comes
/// out around 19 MB from a 428 MB input. Emitting a `cdylib` instead expresses the same
/// final link without materializing the archive at all — 9.8 MB instead of 428 MB, and
/// proportionally less I/O on machines whose storage is slower than the one this was
/// measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppleHostLibrary {
    /// Self-contained archive linked into a packaged application.
    Archive,
    /// Shared library that resolves the `WaterUI` runtime at load time.
    Dynamic,
}

impl AppleHostLibrary {
    const fn for_linkage(linkage: RustLinkage) -> Self {
        match linkage {
            RustLinkage::Static => Self::Archive,
            RustLinkage::SharedRuntime => Self::Dynamic,
        }
    }

    const fn crate_type(self) -> &'static str {
        match self {
            Self::Archive => "staticlib",
            Self::Dynamic => "cdylib",
        }
    }

    /// Name Xcode links against, via `-lwaterui_app` in `OTHER_LDFLAGS`.
    const fn linked_file_name(self) -> &'static str {
        match self {
            Self::Archive => "libwaterui_app.a",
            Self::Dynamic => "libwaterui_app.dylib",
        }
    }

    /// The shape this build must delete, so `-lwaterui_app` cannot resolve to a stale
    /// artifact left by a build of the other kind.
    const fn superseded(self) -> Self {
        match self {
            Self::Archive => Self::Dynamic,
            Self::Dynamic => Self::Archive,
        }
    }
}

/// Remove the host library shape this build did not produce.
///
/// `-lwaterui_app` resolves against whatever sits in the products directory, and `ld`
/// prefers a `.dylib` over a `.a` when both are present. Leaving the previous build's
/// artifact behind would let a packaging build silently link the development shared
/// library, or leave a stale archive shadowing nothing at all.
async fn remove_superseded_host_library(
    directory: &Path,
    produced: AppleHostLibrary,
) -> eyre::Result<()> {
    let stale = directory.join(produced.superseded().linked_file_name());
    match fs::remove_file(&stale).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).wrap_err_with(|| {
            format!(
                "Failed to remove superseded host library {}",
                stale.display()
            )
        }),
    }
}

/// Stage the packaged app's static host library beside the `.app` `water
/// package` reports.
///
/// A packaged app's consumers — the device-test job links
/// `dirname(<Packaged at>)/libwaterui_app.a` into its test bundle — read the
/// library from the directory the report names, so this runs once the
/// artifact has landed there, not inside [`package_apple`] where the bundle
/// still sits in the build cache. `water package` always links the app
/// statically, so the archive shape is the only one this staging contract
/// covers; a `SharedRuntime` packaging build would need a different
/// destination contract, not this copy.
///
/// # Errors
/// Returns an error if the copy or the superseded-shape removal fails.
pub async fn stage_packaged_host_library(
    built: &BuiltTarget,
    packaged_dir: &Path,
) -> eyre::Result<()> {
    const HOST_LIBRARY: AppleHostLibrary = AppleHostLibrary::Archive;
    copy_file(
        &built.artifact,
        &packaged_dir.join(HOST_LIBRARY.linked_file_name()),
    )
    .await?;
    remove_superseded_host_library(packaged_dir, HOST_LIBRARY).await
}

/// The features an Apple runtime's generated native companion is compiled with.
///
/// Each name is a feature the generated manifest forwards to the native backend
/// or framework, so the resolve stays inside the seeded
/// lockfile. Anything loaded into that runtime has to be compiled with the
/// same set. Cargo
/// unifies features per build and folds the result into the `-C metadata` hash it
/// mangles into every symbol, so a module that enables one feature more or fewer
/// than its host links against a runtime whose symbols no longer match. Both
/// callers derive the set here rather than each listing it, so the two cannot
/// drift apart.
///
/// # Errors
///
/// Returns an error when the project's enabled capabilities cannot be resolved.
pub(crate) async fn apple_dependency_features(
    project: &Project,
    browser_runtime: BrowserRuntimePlan,
) -> eyre::Result<Vec<String>> {
    let build_manifest = project.ffi_crate_path().join("Cargo.toml");
    let mut features = Vec::new();
    features.extend(
        crate::project_model::assets::capability_ffi_features(project, &build_manifest).await?,
    );
    if browser_runtime.chromium {
        features.push("chromium".to_string());
    }
    if matches!(browser_runtime.webview, Some(ResolvedWebViewBackend::Cef)) {
        features.push("webview-cef".to_string());
    }
    Ok(features)
}

pub(crate) async fn apple_build_features(
    project: &Project,
    browser_runtime: BrowserRuntimePlan,
    linkage: RustLinkage,
) -> eyre::Result<Vec<String>> {
    let mut features = apple_dependency_features(project, browser_runtime).await?;
    if linkage == RustLinkage::SharedRuntime {
        features.push("dev".to_string());
        // The inspector is devtooling: development sessions get it through the
        // shared-runtime linkage while a packaged build leaves its server
        // stack out. The generated manifest forwards `inspector` only when
        // the resolved `waterui-ffi` declares it, so the build can only name
        // it when the scaffold declared it.
        if crate::templates::generated_ffi_manifest_declares(
            &project.ffi_crate_path().join("Cargo.toml"),
            "inspector",
        )? {
            features.push("inspector".to_string());
        }
    }
    Ok(features)
}

/// Build Rust library for an Apple platform.
///
/// # Errors
/// Returns an error if the Rust build fails or the expected Apple archive cannot be copied.
pub async fn build_rust_lib(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<BuiltTarget> {
    build_rust_lib_with_links(project, platform, options)
        .await
        .map(|(built, _)| built)
}

/// Compile the library and retain rustc's native dependency contract for embedding.
#[expect(
    clippy::too_many_lines,
    reason = "the build pipeline is one ordered sequence of toolchain steps; splitting it would hide the order"
)]
pub(crate) async fn build_rust_lib_with_links(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<(BuiltTarget, Vec<crate::build::NativeLink>)> {
    let triple = options
        .target_triple()
        .cloned()
        .unwrap_or_else(|| platform.triple());
    validate_architecture(triple.architecture)?;
    // A packaged app stamps the identifier as `CFBundleIdentifier`; reject an
    // Apple-invalid one before the Rust build pays for it. An embedded build
    // produces a library the host app embeds, so its identifier never reaches
    // an Apple manifest here.
    if !project.manifest().package.embedded {
        let _ = project
            .bundle_identifier()
            .apple_bundle_identifier()
            .map_err(|error| eyre::eyre!("{error}"))?;
    }
    let options = if project.manifest().package.embedded {
        options.with_static_runtime()
    } else {
        options
    };
    // Resolve fonts BEFORE cargo build - this ensures icons.json is present
    // for crates like fontawesome7 that need it during build.rs
    let font_declarations =
        crate::assets::scan_fonts(project, &project.ffi_crate_path().join("Cargo.toml")).await?;
    let _resolved_fonts = crate::assets::resolve_fonts(font_declarations).await?;
    let browser_runtime_plan = project
        .browser_runtime_plan(platform, TargetBackend::Apple)
        .await?;

    let target = triple.to_string();
    let target_underscore = target.replace('-', "_");
    let host_library = AppleHostLibrary::for_linkage(options.linkage());
    let mut build = RustBuild::new(project.ffi_crate_path(), triple.clone())
        .with_project(project)
        .with_features(
            apple_build_features(project, browser_runtime_plan, options.linkage()).await?,
        )
        .with_envs(options.cargo_envs().iter().cloned());
    if let Some(sccache_path) = options.sccache_path() {
        build = build.with_sccache(sccache_path.to_path_buf());
    }
    if let Some(progress) = options.progress() {
        build = build.with_progress(progress.clone());
    }
    if project.manifest().package.embedded {
        build = build.with_final_rustc_arg("--print=native-static-libs");
    }
    build = build
        .with_env("PKG_CONFIG_ALLOW_CROSS", "1")
        .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{target_underscore}"), "1")
        .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{target}"), "1");
    if options.linkage() == RustLinkage::SharedRuntime {
        build = build.with_preferred_dynamic_linking();
    }

    let target_dir = project.water_target_dir(options.linkage()).await?;
    let build = build.with_target_dir(target_dir.clone());
    let built_target = match host_library {
        // The shared runtime's lib build emits every crate type the
        // manifest declares — not a `--crate-type` selection, which
        // writes only the chosen artifact — because this build feeds
        // two consumers: the staged cdylib and the unhashed
        // `deps/lib<ffi>.rlib` that `localize_archive_symbols` edits
        // below and the entry binary's `--extern` resolves. Selecting
        // `cdylib` alone leaves the rlib unwritten on a clean target
        // dir; the dylib is picked out of Cargo's report by extension.
        AppleHostLibrary::Dynamic => build.build_dylib(options.is_release()).await?,
        // An embedded project never builds the entry binary, so the archive
        // keeps its single-crate-type build and the `native_static_libraries`
        // call below stays in the same dependency mode — a union emit here
        // would leave the override compiling the whole graph a second time.
        AppleHostLibrary::Archive if project.manifest().package.embedded => {
            build
                .clone()
                .with_crate_type_override(host_library.crate_type())
                .build_lib(options.is_release())
                .await?
        }
        // Everywhere else the entry (and any CEF helper) binary follows in the
        // same target dir. A `--crate-type` selection narrows the lib to one
        // artifact and puts its dependencies in a mode the binary builds
        // cannot reuse — under LTO they get `-Clinker-plugin-lto` while the
        // binaries' graph wants plain objects, so the entire dependency
        // graph compiled a second time. Emitting the manifest's declared
        // types shares one dependency fingerprint set across every build;
        // the `.a` is picked out of Cargo's report by extension.
        AppleHostLibrary::Archive => build.build_staticlib(options.is_release()).await?,
    };

    let staged_dir = options.output_dir().map(PathBuf::from);
    let deps_dir = target_dir
        .join(&target)
        .join(if options.is_release() {
            "release"
        } else {
            "debug"
        })
        .join("deps");

    // The executable's `-l` names the runtime soname this build recorded —
    // `RustDynamicLibraries` reads it from the artifact's own dynamic
    // section; absent a dynamic runtime it stays the canonical name.
    let mut runtime_link_name = "waterui_dylib".to_string();

    // Stage the host library and shared runtime before the executable links,
    // so its dependencies already carry their final install names.
    if let Some(output_dir) = options.output_dir() {
        fs::create_dir_all(output_dir).await?;
        let dest_lib = output_dir.join(host_library.linked_file_name());
        copy_file(&built_target.artifact, &dest_lib).await?;
        remove_superseded_host_library(output_dir, host_library).await?;
        if options.linkage() == RustLinkage::SharedRuntime {
            let libraries = RustDynamicLibraries::resolve(&built_target, &triple, project).await?;
            runtime_link_name = dynamic_runtime::runtime_link_name(
                &libraries.waterui_staged_name().to_string_lossy(),
            );
            libraries.stage(output_dir).await?;
            let staged_runtime = libraries.stage_apple_canonical(output_dir).await?;
            if host_library == AppleHostLibrary::Dynamic {
                // The app library records the runtime's cargo-written install
                // name; retarget while the canonical copy still carries it.
                dynamic_runtime::retarget_module(&dest_lib, &staged_runtime).await?;
                // The executable binds the app dylib by its install name.
                dynamic_runtime::canonicalize_install_name(&dest_lib).await?;
            }
            dynamic_runtime::prepare_host_runtime(&staged_runtime).await?;
        }
    } else if host_library == AppleHostLibrary::Dynamic {
        // Without an output directory the link's runtime search dir is the
        // deps dir below, which Cargo fills only with the hashed
        // `libwaterui_dylib-<metadata>.dylib` the `-l` below resolves.
        // Stage the canonical install-name copy there first, with the same
        // `@rpath` handling the packaged staging path performs (cli#272).
        let libraries = RustDynamicLibraries::resolve(&built_target, &triple, project).await?;
        runtime_link_name =
            dynamic_runtime::runtime_link_name(&libraries.waterui_staged_name().to_string_lossy());
        let staged_runtime = libraries.stage_apple_canonical(&deps_dir).await?;
        dynamic_runtime::prepare_host_runtime(&staged_runtime).await?;
    }

    if project.manifest().package.embedded {
        let links = build
            .clone()
            .with_crate_type_override("staticlib")
            .native_static_libraries(options.is_release())
            .await?;
        return Ok((built_target, links));
    }

    // The Rust entry links the companion rlib, carrying backend native links
    // and app exports into one image.
    #[cfg(target_os = "macos")]
    if host_library == AppleHostLibrary::Dynamic {
        let ffi_rlib = deps_dir.join(format!(
            "lib{}.rlib",
            project.ffi_crate_name().as_str().replace('-', "_")
        ));
        // The rlib's codegen units also export `rust_eh_personality`. The
        // copy lives in the build dir and is only consumed by this link, so
        // localize it in place.
        localize_archive_symbols(&ffi_rlib, &["rust_eh_personality"]).await?;
    }

    let mut executable = build
        .clone()
        .with_final_rustc_arg("-Clink-arg=-Wl,-rpath,@executable_path/../Frameworks")
        .with_final_rustc_arg("-Clink-arg=-Wl,-rpath,@executable_path/Frameworks")
        .with_final_rustc_arg("-Clink-arg=-lc++");

    // VideoToolbox serves the media codec chain (`waterkit-codec`'s hardware
    // decode); an app without the media capability never loads it, so it
    // joins the link line only when the resolved graph says so — the same
    // predicate that selects the `media` FFI surface.
    if crate::project_model::assets::capability_enabled(
        project,
        &project.ffi_crate_path().join("Cargo.toml"),
        "media",
    )
    .await?
    {
        executable = executable
            .with_final_rustc_arg("-Clink-arg=-framework")
            .with_final_rustc_arg("-Clink-arg=VideoToolbox");
    }

    if host_library == AppleHostLibrary::Dynamic {
        let runtime_dir = staged_dir.clone().unwrap_or(deps_dir);
        executable = executable
            .with_final_rustc_arg(link_search_flag(runtime_dir.as_os_str()))
            .with_final_rustc_arg(format!("-Clink-arg=-l{runtime_link_name}"));
    }
    executable
        .build_binary(APPLE_ENTRY_BINARY_NAME, options.is_release())
        .await?;

    // The helper `[[bin]]` exists only when the manifest declared it — the
    // application's linked engine, not chromium alone — so the build gates
    // on the manifest's own predicate or Cargo reports `no bin target`.
    if project.declares_cef_helper().await? {
        build
            .clone()
            .with_final_rustc_arg("-Clink-arg=-Wl,-rpath,@executable_path/../Frameworks")
            .build_binary(
                &crate::project_model::project_types::cef_helper_binary_name(
                    project.ffi_crate_name().as_str(),
                ),
                options.is_release(),
            )
            .await?;
    }

    Ok((built_target, Vec::new()))
}

fn link_search_flag(dir: &OsStr) -> String {
    let mut flag = OsString::from("-Clink-arg=-L");
    flag.push(dir);
    flag.to_string_lossy().into_owned()
}

/// Turns `symbols` (C names without the Mach-O underscore) into local
/// symbols inside every archive member that defines them as global text.
/// The archive keeps every member — only the symbol's visibility changes —
/// so the symbols still resolve for the member's own internal references
/// while stopping `ld`'s duplicate-symbol diagnostics.
#[cfg(target_os = "macos")]
async fn localize_archive_symbols(archive: &Path, symbols: &[&str]) -> eyre::Result<()> {
    let scratch = archive.with_extension("localize-work");
    fs::create_dir_all(&scratch).await?;
    let members = run_command_os(
        "ar",
        ["t".into(), archive.as_os_str().to_owned()].map(OsString::from),
    )
    .await?;
    for member in members
        .lines()
        .map(str::trim)
        .filter(|member| !member.is_empty() && *member != "__.SYMDEF")
    {
        let member_path = scratch.join(member);
        run_command_os(
            "sh",
            [
                OsString::from("-c"),
                OsString::from(format!(
                    "ar p '{}' '{}' > '{}'",
                    archive.display(),
                    member,
                    member_path.display()
                )),
            ],
        )
        .await?;
        let nm = run_command_os("nm", [member_path.as_os_str().to_owned()])
            .await
            .unwrap_or_default();
        let mut args = vec![member_path.as_os_str().to_owned()];
        let mut changed = false;
        for symbol in symbols {
            if nm
                .lines()
                .any(|line| line.contains(&format!(" T _{symbol}")))
            {
                args.push("-unexported_symbol".into());
                args.push(format!("_{symbol}").into());
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        args.push("-o".into());
        args.push(member_path.as_os_str().to_owned());
        run_command_os("ld", std::iter::once(OsString::from("-r")).chain(args)).await?;
        run_command_os(
            "ar",
            [
                "r".into(),
                archive.as_os_str().to_owned(),
                member_path.as_os_str().to_owned(),
            ]
            .map(OsString::from),
        )
        .await?;
    }
    let _ = fs::remove_dir_all(&scratch).await;
    Ok(())
}

/// The deployment targets the Apple backend supports, as `SEMVER` strings.
///
/// These were `*_DEPLOYMENT_TARGET` build settings in the generated Xcode
/// project; entry-owning packaging has no project file, so they are declared
/// here next to the backend that owns them — the same values the framework
/// tree's root `Package.swift` publishes.
const fn apple_deployment_target_for(platform: TargetPlatform) -> Option<&'static str> {
    match platform {
        TargetPlatform::MacOS
        | TargetPlatform::IOS
        | TargetPlatform::IOSSimulator
        | TargetPlatform::MacCatalyst
        | TargetPlatform::TvOS
        | TargetPlatform::TvOSSimulator
        | TargetPlatform::WatchOS
        | TargetPlatform::WatchOSSimulator => Some("26.0"),
        TargetPlatform::VisionOS | TargetPlatform::VisionOSSimulator => Some("2.5"),
        _ => None,
    }
}

/// Resolve the deployment-target environment variable an Apple build must carry.
///
/// # Errors
///
/// Returns an error when the platform has no Apple deployment target.
pub async fn apple_deployment_target(
    _project: &Project,
    platform: TargetPlatform,
) -> eyre::Result<(&'static str, String)> {
    let environment = platform.deployment_target_setting().ok_or_else(|| {
        eyre::eyre!("Platform {platform:?} does not have an Apple deployment target")
    })?;
    let target = apple_deployment_target_for(platform).ok_or_else(|| {
        eyre::eyre!("Platform {platform:?} does not have an Apple deployment target")
    })?;
    Ok((environment, target.to_string()))
}

/// The `*_DEPLOYMENT_TARGET` environment variables a Cargo compilation for
/// `triple` must carry, when `triple` names an Apple platform.
///
/// Every cargo process the CLI starts whose compilation target is Apple gets
/// them — including host builds, where the host triple *is* the Apple target.
/// A simulator triple shares its device variant's setting name and floor, so
/// `Environment::Sim` reaches the same pair as `Environment::Unknown`.
///
/// Mac Catalyst is the one target that carries two variables: the build
/// scripts an `aarch64-apple-ios-macabi` compilation runs key on either side
/// of the bridge — iOS-keyed scripts read `IPHONEOS_DEPLOYMENT_TARGET`,
/// macOS-keyed ones read `MACOSX_DEPLOYMENT_TARGET` — so the macabi triple
/// sets each to its own floor.
pub(crate) fn apple_deployment_target_env(
    triple: &target_lexicon::Triple,
) -> Option<Vec<(&'static str, &'static str)>> {
    use target_lexicon::{Environment, OperatingSystem};
    let platform = match triple.operating_system {
        OperatingSystem::Darwin(_) | OperatingSystem::MacOSX(_) => TargetPlatform::MacOS,
        OperatingSystem::IOS(_) => match triple.environment {
            Environment::Macabi => TargetPlatform::MacCatalyst,
            _ => TargetPlatform::IOS,
        },
        OperatingSystem::TvOS(_) => TargetPlatform::TvOS,
        OperatingSystem::WatchOS(_) => TargetPlatform::WatchOS,
        OperatingSystem::VisionOS(_) | OperatingSystem::XROS(_) => TargetPlatform::VisionOS,
        _ => return None,
    };
    let floor = apple_deployment_target_for(platform)?;
    if platform == TargetPlatform::MacCatalyst {
        return Some(vec![
            (platform.deployment_target_setting()?, floor),
            (
                TargetPlatform::MacOS.deployment_target_setting()?,
                apple_deployment_target_for(TargetPlatform::MacOS)?,
            ),
        ]);
    }
    Some(vec![(platform.deployment_target_setting()?, floor)])
}

// ============================================================================
// Validation
// ============================================================================

/// The checkout the project's `waterui-apple` dependency compiles from —
/// the source directory `cargo metadata` resolved for the ffi crate's
/// dependency, whether it names the canonical `waterui_path/backends/apple`
/// checkout or the framework repository's member at the selected revision.
///
/// # Errors
/// Returns an error when the backend source cannot be located.
pub(crate) async fn apple_backend_source_root(project: &Project) -> eyre::Result<PathBuf> {
    let manifest_path_arg: OsString = project.ffi_crate_path().join("Cargo.toml").into();
    let output = crate::utils::run_command_os(
        "cargo",
        [
            OsString::from("metadata"),
            OsString::from("--format-version"),
            OsString::from("1"),
            OsString::from("--manifest-path"),
            manifest_path_arg,
        ],
    )
    .await
    .wrap_err("cargo metadata failed to resolve the Apple backend source")?;
    let metadata: serde_json::Value =
        serde_json::from_str(&output).wrap_err("cargo metadata returned unparseable JSON")?;
    let manifest_path = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .and_then(|packages| {
            packages.iter().find_map(|package| {
                (package.get("name").and_then(serde_json::Value::as_str) == Some("waterui-apple"))
                    .then(|| {
                    package
                        .get("manifest_path")
                        .and_then(serde_json::Value::as_str)
                })?
            })
        })
        .ok_or_else(|| eyre::eyre!("the ffi crate does not depend on a `waterui-apple` package"))?;
    PathBuf::from(manifest_path)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| eyre::eyre!("`waterui-apple` manifest path has no parent"))
}

// ============================================================================
// Clean
// ============================================================================

/// Clean build artifacts for an Apple platform.
///
/// Entry-owning packaging keeps only the assembled products directory; the
/// Rust target dir is cleaned by the shared target-dir cache logic.
///
/// # Errors
/// Returns an error when the generated build directories cannot be removed.
pub async fn clean_apple(project: &Project) -> eyre::Result<()> {
    if project.apple_backend().is_none() {
        return Ok(()); // Nothing to clean if no backend configured
    }

    let project_path = project.backend_path::<AppleBackend>();
    for directory in [project_path.join("DerivedData"), project_path.join("build")] {
        if directory.exists() {
            fs::remove_dir_all(&directory).await?;
        }
    }

    Ok(())
}

// ============================================================================
// Package
// ============================================================================

/// Package an Apple app in entry-owning mode.
///
/// The `.app` bundle is assembled directly — the ffi crate's
/// `waterui-apple-main` binary as the executable, resources copied in,
/// `actool` compiling the asset catalog, `codesign` signing — with no Xcode
/// project anywhere in the generated tree.
///
/// # Errors
/// Returns an error if the backend is missing, packaging prerequisites are
/// invalid, or bundle assembly/signing fails.
#[expect(
    clippy::too_many_lines,
    reason = "bundle assembly is one ordered sequence of packaging steps; splitting it would hide the order"
)]
pub async fn package_apple(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
    built: &BuiltTarget,
) -> eyre::Result<Artifact> {
    let backend = project
        .apple_backend()
        .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;
    // The identifier lands in `Info.plist` as `CFBundleIdentifier` and drives
    // codesign/provisioning below — reject an Apple-invalid one before any
    // asset staging or SDK work.
    let bundle_id = project
        .bundle_identifier()
        .apple_bundle_identifier()
        .map_err(|error| eyre::eyre!("{error}"))?;
    let browser_runtime_plan = project
        .browser_runtime_plan(platform, TargetBackend::Apple)
        .await?;
    // Crate-declared entitlements and `Info.plist` keys, collected from the
    // graph the companion compiles with — a conflict fails before any
    // bundle work.
    let mut apple_declarations = crate::assets::scan_apple_declarations(
        project,
        &project.ffi_crate_path().join("Cargo.toml"),
        &apple_dependency_features(project, browser_runtime_plan).await?,
    )
    .await?;
    apple_declarations.supply_app_values(&project.manifest().app_values)?;

    let project_path = project.backend_path::<AppleBackend>();

    let configuration = if options.is_debug() {
        "Debug"
    } else {
        "Release"
    };
    let triple = platform.triple();
    let sdk_name = platform
        .sdk_name()
        .ok_or_else(|| eyre::eyre!("Platform {platform:?} is not an Apple platform"))?;
    let (_, deployment_target) = apple_deployment_target(project, platform).await?;

    // Assets are staged into a scratch directory; the bundle copies them out
    // from there (`waterui_assets`, `fonts`) and `actool` compiles the asset
    // catalog (`WaterUIAssets.xcassets`).
    let staging_dir = project_path.join("DerivedData/AssetStaging");
    copy_assets_and_fonts(
        project,
        &staging_dir,
        &built.app_symbols()?,
        options.uses_dev_server(),
    )
    .await?;

    // Xcode used "Debug-iphonesimulator"-style product configuration names;
    // the same layout keeps `simctl`/`devicectl` installs pointed at a stable
    // location.
    let products_config = if sdk_name == "macosx" {
        configuration.to_string()
    } else {
        format!("{configuration}-{sdk_name}")
    };
    let products_dir = project_path
        .join("DerivedData")
        .join("Build/Products")
        .join(&products_config);
    let product_name = crate::apple::backend::apple_product_name(project)?.to_string();
    let app_path = products_dir.join(format!("{product_name}.app"));

    #[cfg(target_os = "macos")]
    if platform == TargetPlatform::MacOS {
        browser_runtime::remove_macos_app(&app_path.join("Contents")).await?;
        remove_cef_helper_apps(&app_path, &product_name).await?;
    }

    let ctx = AppleBackend::template_context(project).await?;
    let layout = app_bundle::AppleAppLayout::for_app(&app_path, sdk_name);

    let executable = built.profile_dir.join(APPLE_ENTRY_BINARY_NAME);
    let mut info_plist = app_bundle::apple_info_plist(
        &ctx,
        project,
        platform,
        &deployment_target,
        &product_name,
        &bundle_id,
    );
    apple_declarations.merge_into_info_plist(&mut info_plist)?;

    app_bundle::assemble_app_bundle(
        &layout,
        &executable,
        &product_name,
        &staging_dir,
        &info_plist,
        sdk_name,
        &deployment_target,
    )
    .await?;

    // The shared-runtime development linkage ships `libwaterui_dylib` and the
    // Rust standard library inside the bundle's Frameworks directory; a
    // statically linked package carries neither.
    let shared_runtime = if options.uses_shared_rust_runtime() {
        let bin_built = BuiltTarget {
            profile_dir: built.profile_dir.clone(),
            artifact: layout.executable_file(&product_name),
            shared_runtime: built.shared_runtime.clone(),
            app_library: None,
        };
        let libraries = RustDynamicLibraries::resolve(&bin_built, &triple, project).await?;
        libraries.stage(&layout.frameworks_dir).await?;
        let staged_runtime = libraries
            .stage_apple_canonical(&layout.frameworks_dir)
            .await?;
        // Redirect the executable's recorded runtime dependency to the
        // canonical `@rpath` name of the staged Rust runtime.
        dynamic_runtime::retarget_module(&layout.executable_file(&product_name), &staged_runtime)
            .await?;
        dynamic_runtime::prepare_host_runtime(&staged_runtime).await?;
        Some(libraries)
    } else {
        RustDynamicLibraries::remove_staged(&layout.frameworks_dir, &triple).await?;
        None
    };
    let _ = shared_runtime;

    #[cfg(target_os = "macos")]
    if platform == TargetPlatform::MacOS && browser_runtime_plan.requires_cef() {
        browser_runtime::stage_macos_app(
            browser_runtime_plan,
            &built.profile_dir,
            &app_path.join("Contents"),
        )
        .await?;
        // Helper bundles wrap the helper `[[bin]]`, which the manifest
        // declares only when the application links the CEF engine crate —
        // chromium alone stages the runtime but builds no helper.
        if project.declares_cef_helper().await? {
            let main_binary = layout.executable_file(&product_name);
            let helper_binary = built.profile_dir.join(
                crate::project_model::project_types::cef_helper_binary_name(
                    project.ffi_crate_name().as_str(),
                ),
            );
            package_cef_helper_app(&app_path, &main_binary, &helper_binary, &bundle_id).await?;
        }
    }

    app_bundle::sign_apple_app(
        &layout,
        platform,
        &options,
        backend,
        project_path.as_path(),
        project,
        &deployment_target,
        &apple_declarations,
    )
    .await?;

    Ok(Artifact::new(project.bundle_identifier(), app_path))
}

// ============================================================================
// Asset and Font Handling
// ============================================================================

/// Copy project assets and dependency fonts to the app resources directory.
/// `symbols` is the app library artifact the target build produced, whose
/// `waterui_meta_bundle_*` statics declare the asset mounts.
async fn copy_assets_and_fonts(
    project: &Project,
    dest_dir: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
) -> eyre::Result<()> {
    // Stage project assets using platform-native conventions.
    let manifest =
        assets::stage_project_assets_for_apple(project, dest_dir, symbols, dev_server).await?;

    // Scan and resolve dependency fonts
    let font_declarations =
        assets::scan_fonts(project, &project.ffi_crate_path().join("Cargo.toml")).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);

    if !resolved_fonts.is_empty() {
        // Copy fonts to app resources; `waterui-apple` registers every font
        // file in the bundle at startup.
        let fonts_dest = dest_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;

        info!("Copied {} fonts to Apple app", resolved_fonts.len());
    }

    Ok(())
}

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the Apple backend.
#[must_use]
pub const fn is_apple_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::MacOS
            | TargetPlatform::IOS
            | TargetPlatform::IOSSimulator
            | TargetPlatform::TvOS
            | TargetPlatform::TvOSSimulator
            | TargetPlatform::WatchOS
            | TargetPlatform::WatchOSSimulator
            | TargetPlatform::VisionOS
            | TargetPlatform::VisionOSSimulator
    )
}

//! Apple platform build and package utilities.
//!
//! This module provides utility functions for building and packaging Apple apps.
//! These functions are used by `AppleBackend` to implement the `Backend` trait.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use askama::Template;
use eyre::{Context, bail};
use smol::fs;
use target_lexicon::Architecture;
use tracing::{debug, info};

#[cfg(target_os = "macos")]
use crate::browser_runtime;
#[cfg(target_os = "macos")]
use crate::macos_bundle::{package_cef_helper_app, remove_cef_helper_apps, sign_macos_app};
use crate::{
    apple::backend::AppleBackend,
    apple::dynamic_runtime,
    assets::{self, ResolvedFont},
    build::{BuildOptions, RustBuild, RustDynamicLibraries, RustLinkage},
    device::Artifact,
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::{BrowserRuntimePlan, Project, ResolvedWebViewBackend},
    templates::FontRegistrationTemplateEntry,
    toolchain::Host,
    utils::{copy_file, run_command_os},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AppleNativeLinkInputs {
    archives: Vec<PathBuf>,
    linker_flags: Vec<String>,
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

    /// Extension Cargo gives the built artifact.
    const fn built_extension(self) -> &'static str {
        match self {
            Self::Archive => "a",
            Self::Dynamic => "dylib",
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

/// Cargo features an Apple FFI build resolves its dependency graph with.
/// The `waterui-ffi` features an Apple runtime is compiled with.
///
/// Anything loaded into that runtime has to be compiled with the same set. Cargo
/// unifies features per build and folds the result into the `-C metadata` hash it
/// mangles into every symbol, so a module that enables one feature more or fewer
/// than its host links against a runtime whose symbols no longer match. Both
/// callers derive the set here rather than each listing it, so the two cannot
/// drift apart.
///
/// # Errors
///
/// Returns an error when the project's enabled capabilities cannot be resolved.
pub(crate) async fn apple_ffi_dependency_features(
    project: &Project,
    browser_runtime: BrowserRuntimePlan,
) -> eyre::Result<Vec<String>> {
    let mut features = vec!["waterui-ffi/c-api".to_string()];
    features.extend(crate::project_model::assets::capability_ffi_features(project).await?);
    if browser_runtime.chromium {
        features.push("waterui-ffi/chromium".to_string());
    }
    if matches!(browser_runtime.webview, Some(ResolvedWebViewBackend::Cef)) {
        features.push("waterui-ffi/webview-cef".to_string());
    }
    Ok(features)
}

async fn apple_ffi_build_features(
    project: &Project,
    browser_runtime: BrowserRuntimePlan,
    linkage: RustLinkage,
) -> eyre::Result<Vec<String>> {
    let mut features = apple_ffi_dependency_features(project, browser_runtime).await?;
    if linkage == RustLinkage::SharedRuntime {
        features.push("dev".to_string());
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
) -> eyre::Result<PathBuf> {
    // Resolve fonts BEFORE cargo build - this ensures icons.json is downloaded
    // for crates like fontawesome7 that need it during build.rs
    let font_declarations = crate::assets::scan_fonts(project).await?;
    let _resolved_fonts = crate::assets::resolve_fonts(font_declarations).await?;
    let browser_runtime_plan = project
        .browser_runtime_plan(platform, TargetBackend::Apple)
        .await?;

    let triple = options
        .target_triple()
        .cloned()
        .unwrap_or_else(|| platform.triple());
    let target = triple.to_string();
    let target_underscore = target.replace('-', "_");
    let host_library = AppleHostLibrary::for_linkage(options.linkage());
    let mut build = RustBuild::new(project.ffi_crate_path(), triple.clone())
        .with_project(project)
        .with_features(
            apple_ffi_build_features(project, browser_runtime_plan, options.linkage()).await?,
        )
        .with_crate_type_override(host_library.crate_type());
    if let Some(sccache_path) = options.sccache_path() {
        build = build.with_sccache(sccache_path.to_path_buf());
    }
    build = build
        .with_env("PKG_CONFIG_ALLOW_CROSS", "1")
        .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{target_underscore}"), "1")
        .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{target}"), "1");
    let (deployment_environment, deployment_target) =
        apple_deployment_target(project, platform).await?;
    build = build.with_env(deployment_environment, deployment_target);
    if options.linkage() == RustLinkage::SharedRuntime {
        build = build.with_preferred_dynamic_linking();
    }
    build = build.with_target_dir(project.water_target_dir(options.linkage()).await?);
    let lib_dir = build.build_lib(options.is_release()).await?;
    if browser_runtime_plan.requires_cef() {
        build
            .clone()
            .with_final_rustc_arg("-Clink-arg=-Wl,-rpath,@executable_path/../Frameworks")
            .build_binary("waterui-cef-helper", options.is_release())
            .await?;
    }

    // If output_dir is specified, copy the library there
    if let Some(output_dir) = options.output_dir() {
        let lib_name = project.ffi_crate_name().replace('-', "_");
        let source_lib = lib_dir.join(format!("lib{lib_name}.{}", host_library.built_extension()));

        if !source_lib.exists() {
            bail!(
                "Built library not found at {} (expected {} for Apple target {})",
                source_lib.display(),
                host_library.crate_type(),
                triple
            );
        }
        fs::create_dir_all(output_dir).await?;
        let dest_lib = output_dir.join(host_library.linked_file_name());
        copy_file(&source_lib, &dest_lib).await?;
        remove_superseded_host_library(output_dir, host_library).await?;
        if options.linkage() == RustLinkage::SharedRuntime {
            let libraries = RustDynamicLibraries::resolve(&lib_dir, &triple).await?;
            dynamic_runtime::prepare_host_runtime(libraries.waterui()).await?;
            libraries.stage(output_dir).await?;
        }
    }

    Ok(lib_dir)
}

/// Resolve the deployment-target environment variable an Apple build must carry.
///
/// # Errors
///
/// Returns an error when the Xcode project does not define exactly one value.
pub(crate) async fn apple_deployment_target(
    project: &Project,
    platform: TargetPlatform,
) -> eyre::Result<(&'static str, String)> {
    let backend = project
        .apple_backend()
        .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;
    let (environment, build_setting) = match platform {
        TargetPlatform::MacOS => ("MACOSX_DEPLOYMENT_TARGET", "MACOSX_DEPLOYMENT_TARGET"),
        TargetPlatform::IOS | TargetPlatform::IOSSimulator => {
            ("IPHONEOS_DEPLOYMENT_TARGET", "IPHONEOS_DEPLOYMENT_TARGET")
        }
        other => {
            bail!("Platform {other:?} does not have an Apple deployment target");
        }
    };
    let project_file = project
        .backend_path::<AppleBackend>()
        .join(format!("{}.xcodeproj", backend.scheme))
        .join("project.pbxproj");
    let contents = fs::read_to_string(&project_file)
        .await
        .wrap_err_with(|| format!("Failed to read {}", project_file.display()))?;
    let target = unique_xcode_build_setting(&contents, build_setting)?;
    Ok((environment, target))
}

fn unique_xcode_build_setting(contents: &str, key: &str) -> eyre::Result<String> {
    let prefix = format!("{key} = ");
    let values = contents
        .lines()
        .filter_map(|line| line.trim().strip_prefix(&prefix))
        .filter_map(|value| value.strip_suffix(';'))
        .map(|value| value.trim_matches('"').to_string())
        .collect::<BTreeSet<_>>();
    match values.len() {
        1 => Ok(values.into_iter().next().expect("one build setting value")),
        0 => {
            bail!("Xcode project does not define {key}");
        }
        _ => {
            bail!(
                "Xcode project defines conflicting {key} values: {}",
                values.into_iter().collect::<Vec<_>>().join(", ")
            );
        }
    }
}

// ============================================================================
// Validation
// ============================================================================

/// The local Apple backend `[backend.apple] backend_path` names is the
/// checkout the generated project references — validate it is a real Swift
/// package. `waterui_path` alone no longer supplies one: the framework
/// checkout carries no `backends/apple` tree since the submodule was dropped.
fn validate_local_apple_backend(project: &Project) -> eyre::Result<()> {
    let Some(backend_path) = project
        .manifest()
        .backends
        .apple()
        .and_then(|backend| backend.backend_path.as_deref())
    else {
        return Ok(());
    };

    let backend_root = {
        let candidate = PathBuf::from(backend_path);
        if candidate.is_absolute() {
            candidate
        } else {
            project.root().join(candidate)
        }
    };

    let package_manifest = backend_root.join("Package.swift");
    if package_manifest.exists() {
        return Ok(());
    }

    bail!(
        "`[backend.apple] backend_path` points at `{}`, which has no `Package.swift` — \
         the Apple backend lives in its own repository now; point it at an \
         `apple-backend` checkout, or remove `backend_path` to consume the pinned \
         release from SwiftPM.",
        backend_root.display()
    );
}

async fn ensure_apple_linker_flags(
    xcodeproj: &Path,
    required_flags: &[String],
) -> eyre::Result<()> {
    let pbxproj_path = xcodeproj.join("project.pbxproj");
    if !pbxproj_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&pbxproj_path)
        .await
        .wrap_err_with(|| format!("Failed to read {}", pbxproj_path.display()))?;
    let (updated, changed) = inject_other_ldflags(&content, required_flags);
    if changed {
        fs::write(&pbxproj_path, updated)
            .await
            .wrap_err_with(|| format!("Failed to write {}", pbxproj_path.display()))?;
        info!(
            "Updated {} with required Apple linker flags",
            pbxproj_path.display()
        );
    }

    Ok(())
}

fn inject_other_ldflags(content: &str, required_flags: &[String]) -> (String, bool) {
    let mut changed = false;
    let mut lines = Vec::new();
    for line in content.lines() {
        if line.contains("OTHER_LDFLAGS = \"")
            && let Some((prefix, rest)) = line.split_once("OTHER_LDFLAGS = \"")
            && let Some((flags, suffix)) = rest.split_once("\";")
        {
            let (mut merged, _) = sanitize_other_ldflags(flags);
            for required in required_flags {
                if !merged.contains(required) {
                    if !merged.is_empty() {
                        merged.push(' ');
                    }
                    merged.push_str(required);
                }
            }
            let line_changed = merged != flags;
            if line_changed {
                changed = true;
            }
            lines.push(format!("{prefix}OTHER_LDFLAGS = \"{merged}\";{suffix}"));
            continue;
        }
        lines.push(line.to_string());
    }

    let mut updated = lines.join("\n");
    if content.ends_with('\n') {
        updated.push('\n');
    }
    (updated, changed)
}

fn sanitize_other_ldflags(flags: &str) -> (String, bool) {
    let normalized = flags
        .split_whitespace()
        .filter(|flag| !matches!(*flag, "-lwaterui_app" | "-lwaterui_dylib"))
        .collect::<Vec<_>>()
        .join(" ");
    let changed = normalized != flags;
    (normalized, changed)
}

async fn collect_apple_native_link_inputs(lib_dir: &Path) -> eyre::Result<AppleNativeLinkInputs> {
    let lib_dir = lib_dir.to_path_buf();
    smol::unblock(move || collect_apple_native_link_inputs_sync(&lib_dir)).await
}

fn collect_apple_native_link_inputs_sync(lib_dir: &Path) -> eyre::Result<AppleNativeLinkInputs> {
    let build_root = lib_dir.join("build");
    if !build_root.exists() {
        return Ok(AppleNativeLinkInputs::default());
    }

    let mut archive_paths = BTreeSet::new();
    let mut linker_flags = Vec::new();

    for entry in std::fs::read_dir(&build_root)? {
        let entry = entry?;
        let crate_build_dir = entry.path();
        if !crate_build_dir.is_dir() {
            continue;
        }

        // Stable names each unit dir `<pkg>-<hash>`; current nightly nests one
        // level deeper under `<pkg>/<hash>` (#901). A top-level dir that is a
        // build-script unit is processed directly, otherwise its hash subdirs
        // are.
        if is_build_script_unit_dir(&crate_build_dir) {
            collect_link_inputs_from_unit_dir(
                &crate_build_dir,
                &mut archive_paths,
                &mut linker_flags,
            )?;
        } else {
            for sub_entry in std::fs::read_dir(&crate_build_dir)?.flatten() {
                let sub_dir = sub_entry.path();
                if sub_dir.is_dir() && is_build_script_unit_dir(&sub_dir) {
                    collect_link_inputs_from_unit_dir(
                        &sub_dir,
                        &mut archive_paths,
                        &mut linker_flags,
                    )?;
                }
            }
        }
    }

    Ok(AppleNativeLinkInputs {
        archives: archive_paths.into_iter().collect(),
        linker_flags,
    })
}

/// A build-script unit dir carries the script's captured stdout — `output` on
/// stable, `run/stdout` on current nightly. Compile units get an `out/` dir
/// for their own artifacts too, so `out/` alone is not proof of a
/// build-script unit under nightly.
fn is_build_script_unit_dir(dir: &Path) -> bool {
    dir.join("output").is_file() || dir.join("run").join("stdout").is_file()
}

fn collect_link_inputs_from_unit_dir(
    unit_dir: &Path,
    archive_paths: &mut BTreeSet<PathBuf>,
    linker_flags: &mut Vec<String>,
) -> eyre::Result<()> {
    // Every crate that ran a build script may have emitted
    // `cargo:rustc-link-*` directives; `-sys` crates like
    // `system-configuration-sys` emit only those, with no archive or Swift
    // bridge artifact to show for it, so the parse cannot be gated on
    // outputs.
    for output_path in [unit_dir.join("output"), unit_dir.join("run").join("stdout")] {
        if output_path.is_file() {
            let output = std::fs::read_to_string(&output_path)?;
            for flag in apple_linker_flags_from_build_output(&output) {
                push_unique_flag(linker_flags, flag);
            }
        }
    }

    // For a build-script unit, `out/` is OUT_DIR (nightly records it in
    // `run/root-output`), so a `lib*.a` inside is an artifact the crate ships
    // for linking, with or without a Swift bridge alongside it.
    let out_dir = unit_dir.join("out");
    if !out_dir.is_dir() {
        return Ok(());
    }

    for out_entry in std::fs::read_dir(&out_dir)? {
        let out_entry = out_entry?;
        let path = out_entry.path();
        if path.extension().is_some_and(|ext| ext == "a")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("lib"))
        {
            archive_paths.insert(path.clone());
            if let Some(flag) = static_archive_link_flag(&path) {
                push_unique_flag(linker_flags, flag);
            }
        }
    }
    Ok(())
}

fn static_archive_link_flag(archive_path: &Path) -> Option<String> {
    let file_name = archive_path.file_name()?.to_str()?;
    let library_name = file_name
        .strip_prefix("lib")?
        .strip_suffix(".a")
        .unwrap_or(file_name);
    Some(format!("-l{library_name}"))
}

fn apple_linker_flags_from_build_output(output: &str) -> Vec<String> {
    let mut flags = Vec::new();
    for line in output.lines() {
        if let Some(framework) = line.strip_prefix("cargo:rustc-link-lib=framework=") {
            push_unique_flag(&mut flags, format!("-framework {framework}"));
        } else if let Some(arg) = line.strip_prefix("cargo:rustc-link-arg=") {
            push_unique_flag(&mut flags, arg.to_string());
        }
    }
    flags
}

fn push_unique_flag(flags: &mut Vec<String>, flag: String) {
    if !flags.iter().any(|existing| existing == &flag) {
        flags.push(flag);
    }
}

// ============================================================================
// Clean
// ============================================================================

/// Clean Xcode build artifacts for an Apple platform.
///
/// # Errors
/// Returns an error if `xcodebuild clean` fails or generated build directories cannot be removed.
pub async fn clean_apple(project: &Project) -> eyre::Result<()> {
    let Some(backend) = project.apple_backend() else {
        return Ok(()); // Nothing to clean if no backend configured
    };

    let project_path = project.backend_path::<AppleBackend>();
    let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

    if !xcodeproj.exists() {
        return Ok(());
    }

    let args: Vec<OsString> = vec![
        "-project".into(),
        xcodeproj.as_os_str().to_owned(),
        "-scheme".into(),
        backend.scheme.as_str().into(),
        "clean".into(),
    ];
    run_command_os("xcodebuild", args).await?;

    let build_dir = project_path.join("build");
    if build_dir.exists() {
        fs::remove_dir_all(&build_dir).await?;
    }

    Ok(())
}

// ============================================================================
// Package
// ============================================================================

/// Package an Apple app using xcodebuild.
///
/// # Errors
/// Returns an error if the backend is missing, packaging prerequisites are invalid, or `xcodebuild` fails.
#[allow(clippy::too_many_lines)]
pub async fn package_apple(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> eyre::Result<Artifact> {
    let backend = project
        .apple_backend()
        .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;
    let browser_runtime_plan = project
        .browser_runtime_plan(platform, TargetBackend::Apple)
        .await?;

    let project_path = project.backend_path::<AppleBackend>();
    let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

    if !xcodeproj.exists() {
        bail!(
            "Xcode project not found at {}. Did you run 'water create'?",
            xcodeproj.display()
        );
    }

    validate_local_apple_backend(project)?;

    // Copy project assets and fonts
    let app_resources_dir = project_path.join(&backend.scheme);
    copy_assets_and_fonts(project, &app_resources_dir, None, options.uses_dev_server()).await?;

    let configuration = if options.is_debug() {
        "Debug"
    } else {
        "Release"
    };

    let derived_data = project_path.join("DerivedData");
    let triple = platform.triple();

    // Copy the built Rust library to where Xcode expects it
    let linkage = if options.uses_shared_rust_runtime() {
        RustLinkage::SharedRuntime
    } else {
        RustLinkage::Static
    };
    let lib_dir = RustBuild::new(project.ffi_crate_path(), triple.clone())
        .with_target_dir(project.water_target_dir(linkage).await?)
        .lib_output_dir(!options.is_debug())
        .await
        .wrap_err("Failed to resolve native FFI crate target directory")?;
    let host_library = AppleHostLibrary::for_linkage(linkage);
    let lib_name = project.ffi_crate_name().replace('-', "_");
    let source_lib = lib_dir.join(format!("lib{lib_name}.{}", host_library.built_extension()));

    // Get SDK name - must be an Apple platform
    let sdk_name = platform
        .sdk_name()
        .ok_or_else(|| eyre::eyre!("Platform {:?} is not an Apple platform", platform))?;

    // Xcode uses "Debug-iphonesimulator" for simulators, "Debug" for macOS
    let products_config = if sdk_name == "macosx" {
        configuration.to_string()
    } else {
        format!("{configuration}-{sdk_name}")
    };
    let products_dir = derived_data.join("Build/Products").join(&products_config);
    fs::create_dir_all(&products_dir).await?;
    // The bundle is named after the project, not after the scheme, so that
    // macOS shows the application's own name rather than the scaffold's.
    let product_name = crate::apple::backend::apple_product_name(project)?;
    let app_path = products_dir.join(format!("{product_name}.app"));

    #[cfg(target_os = "macos")]
    if platform == TargetPlatform::MacOS {
        browser_runtime::remove_macos_app(&app_path.join("Contents")).await?;
        remove_cef_helper_apps(&app_path, product_name).await?;
    }

    let dest_lib = products_dir.join(host_library.linked_file_name());
    copy_file(&source_lib, &dest_lib).await?;
    remove_superseded_host_library(&products_dir, host_library).await?;
    if host_library == AppleHostLibrary::Dynamic {
        dynamic_runtime::set_rpath_install_name(&dest_lib, host_library.linked_file_name()).await?;
    }

    let shared_runtime = if options.uses_shared_rust_runtime() {
        let libraries = RustDynamicLibraries::resolve(&lib_dir, &triple).await?;
        dynamic_runtime::prepare_host_runtime(libraries.waterui()).await?;
        libraries.stage(&products_dir).await?;
        Some(libraries)
    } else {
        RustDynamicLibraries::remove_staged(&products_dir, &triple).await?;
        None
    };

    let native_link_inputs = collect_apple_native_link_inputs(&lib_dir).await?;
    for archive in &native_link_inputs.archives {
        let file_name = archive.file_name().ok_or_else(|| {
            eyre::eyre!(
                "Bridge archive path had no file name: {}",
                archive.display()
            )
        })?;
        copy_file(archive, &products_dir.join(file_name)).await?;
    }

    let mut required_link_flags = vec![
        "-framework VideoToolbox".to_string(),
        // The Xcode project no longer names the Rust library, because its shape depends
        // on the linkage this build selected. `-lwaterui_app` resolves to whichever of
        // `libwaterui_app.a` / `libwaterui_app.dylib` this build left in
        // `BUILT_PRODUCTS_DIR`; the other is removed so the choice is unambiguous.
        "-lwaterui_app".to_string(),
    ];
    if shared_runtime.is_some() {
        required_link_flags.push("-lwaterui_dylib".to_string());
    }
    for flag in native_link_inputs.linker_flags {
        push_unique_flag(&mut required_link_flags, flag);
    }
    ensure_apple_linker_flags(&xcodeproj, &required_link_flags).await?;

    // Build with xcodebuild
    // Determine the Xcode arch name from the platform architecture
    let arch_name = match platform.arch() {
        Architecture::Aarch64(_) => "arm64",
        Architecture::X86_64 => "x86_64",
        other => {
            bail!("Unsupported Apple architecture for xcodebuild ARCHS: {other:?}");
        }
    };
    let archs_arg = format!("ARCHS={arch_name}");

    let mut args = vec![
        OsString::from("-project"),
        xcodeproj.as_os_str().to_owned(),
        OsString::from("-scheme"),
        backend.scheme.as_str().into(),
        OsString::from("-configuration"),
        configuration.into(),
        OsString::from("-sdk"),
        sdk_name.into(),
        OsString::from("-derivedDataPath"),
        derived_data.as_os_str().to_owned(),
        archs_arg.into(),
        OsString::from("ONLY_ACTIVE_ARCH=YES"),
        OsString::from("build"),
    ];

    if platform.is_simulator() || options.is_debug() {
        args.extend([
            OsString::from("CODE_SIGNING_ALLOWED=NO"),
            OsString::from("CODE_SIGNING_REQUIRED=NO"),
            OsString::from("CODE_SIGN_IDENTITY=-"),
        ]);
    }

    // Optional capabilities are compiled out of the backend unless the app
    // enabled the matching FFI feature, which is what exports their symbols.
    let swift_conditions = apple_swift_conditions(project).await?;
    if !swift_conditions.is_empty() {
        args.push(format!("OTHER_SWIFT_FLAGS={}", swift_conditions.join(" ")).into());
    }

    // Tell Xcode's run-script phases not to call `water build` again (the
    // Rust library is already built). The flag is scoped to the xcodebuild
    // child and propagates to its script phases; it must never be set on
    // this process.
    Host::current()
        .with_env("WATERUI_SKIP_RUST_BUILD", "1")
        .run("xcodebuild", args)
        .await?;

    if !app_path.exists() {
        bail!(
            "Built app not found at {}. Check xcodebuild output for errors.",
            app_path.display()
        );
    }

    let frameworks_dir = apple_frameworks_dir(&app_path, sdk_name);
    if let Some(libraries) = shared_runtime {
        fs::create_dir_all(&frameworks_dir).await?;
        libraries.stage(&frameworks_dir).await?;
        // The executable resolves `@rpath/libwaterui_app.dylib` through the bundle's
        // Frameworks directory, the same way it resolves the shared runtime.
        copy_file(
            &dest_lib,
            &frameworks_dir.join(host_library.linked_file_name()),
        )
        .await?;
    } else {
        RustDynamicLibraries::remove_staged(&frameworks_dir, &triple).await?;
    }

    #[cfg(target_os = "macos")]
    if platform == TargetPlatform::MacOS && browser_runtime_plan.requires_cef() {
        browser_runtime::stage_macos_app(
            browser_runtime_plan,
            &lib_dir,
            &app_path.join("Contents"),
        )
        .await?;
        let main_binary = app_path.join("Contents/MacOS").join(product_name);
        let helper_binary = lib_dir.join("waterui-cef-helper");
        package_cef_helper_app(
            &app_path,
            &main_binary,
            &helper_binary,
            project.bundle_identifier(),
        )
        .await?;
        let requires_stable_identity = project.manifest().permissions.iter().any(|(key, entry)| {
            entry.is_enabled() && !key.macos_usage_description_keys().is_empty()
        });
        sign_macos_app(
            &app_path,
            project.bundle_identifier(),
            requires_stable_identity,
        )
        .await?;
    }

    #[cfg(not(target_os = "macos"))]
    let _ = browser_runtime_plan;

    Ok(Artifact::new(project.bundle_identifier(), app_path))
}

fn apple_frameworks_dir(app_path: &Path, sdk_name: &str) -> PathBuf {
    if sdk_name == "macosx" {
        app_path.join("Contents/Frameworks")
    } else {
        app_path.join("Frameworks")
    }
}

// ============================================================================
// Asset and Font Handling
// ============================================================================

/// Copy project assets and dependency fonts to the app resources directory.
async fn copy_assets_and_fonts(
    project: &Project,
    dest_dir: &Path,
    sccache_path: Option<&Path>,
    dev_server: bool,
) -> eyre::Result<()> {
    // Stage project assets using platform-native conventions.
    let manifest =
        assets::stage_project_assets_for_apple(project, dest_dir, sccache_path, dev_server).await?;

    // Scan and resolve dependency fonts
    let font_declarations = assets::scan_fonts(project).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);

    if !resolved_fonts.is_empty() {
        // Copy fonts to app resources
        let fonts_dest = dest_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;

        // Generate WaterUIFonts.swift for font registration
        generate_font_registration_swift(&resolved_fonts, dest_dir).await?;

        info!("Copied {} fonts to Apple app", resolved_fonts.len());
    }

    Ok(())
}

#[derive(Template)]
#[template(
    path = "src/templates/apple/AppName/WaterUIFonts.swift.tpl",
    escape = "none"
)]
struct WaterUiFontsSwiftTemplate<'a> {
    font_entries: &'a [FontRegistrationTemplateEntry],
}

/// Generate WaterUIFonts.swift file for registering custom fonts.
async fn generate_font_registration_swift(
    fonts: &[ResolvedFont],
    dest_dir: &Path,
) -> eyre::Result<()> {
    let font_entries = fonts
        .iter()
        .map(|font| FontRegistrationTemplateEntry {
            family_name: font.name.clone(),
            file_name: font
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect::<Vec<_>>();

    let content = WaterUiFontsSwiftTemplate {
        font_entries: &font_entries,
    }
    .render()
    .map_err(|error| eyre::eyre!("Failed to render WaterUIFonts.swift template: {error}"))?;

    let swift_path = dest_dir.join("WaterUIFonts.swift");
    fs::write(&swift_path, content).await?;

    debug!("Generated {}", swift_path.display());

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

/// Swift compilation conditions matching the optional capabilities this app's
/// graph carries, so the backend compiles exactly the components whose symbols
/// exist.
///
/// The decision must come from [`assets::capability_enabled`] — the same
/// predicate that forwards each capability's feature to the FFI build. The FFI
/// features travel on the build command line, never into the generated
/// manifest, so re-resolving `waterui-ffi`'s features from the manifest graph
/// reads every capability as off and prunes components whose symbols the
/// dylib does export.
///
/// The two lists mirror each capability's default polarity, so a bare
/// `swift build` of the backend package — no conditions at all — still
/// compiles what a default-featured app links. A default-off capability gets a
/// positive condition when carried; a default-on capability gets a negative
/// condition when dropped.
///
/// [`assets::capability_enabled`]: crate::project_model::assets::capability_enabled
async fn apple_swift_conditions(project: &Project) -> eyre::Result<Vec<String>> {
    /// Default-off capabilities, named when the app's graph carries them.
    const OPTIONAL_COMPONENTS: &[(&str, &str)] =
        &[("map", "WATERUI_MAP"), ("webview", "WATERUI_WEBVIEW")];
    /// Default-on capabilities, named when the app's graph drops them.
    const DEFAULT_COMPONENTS: &[(&str, &str)] =
        &[("gpu", "WATERUI_NO_GPU"), ("media", "WATERUI_NO_MEDIA")];

    let mut conditions = Vec::new();
    for (capability, condition) in OPTIONAL_COMPONENTS {
        if crate::project_model::assets::capability_enabled(project, capability).await? {
            conditions.push(format!("-D{condition}"));
        }
    }
    for (capability, condition) in DEFAULT_COMPONENTS {
        if !crate::project_model::assets::capability_enabled(project, capability).await? {
            conditions.push(format!("-D{condition}"));
        }
    }
    Ok(conditions)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        apple_linker_flags_from_build_output, collect_apple_native_link_inputs_sync,
        inject_other_ldflags, unique_xcode_build_setting,
    };

    #[test]
    fn derives_a_unique_xcode_deployment_target() {
        let settings = "MACOSX_DEPLOYMENT_TARGET = 15.0;\nMACOSX_DEPLOYMENT_TARGET = 15.0;\n";
        assert_eq!(
            unique_xcode_build_setting(settings, "MACOSX_DEPLOYMENT_TARGET")
                .expect("unique deployment target"),
            "15.0"
        );
    }

    #[test]
    fn rejects_conflicting_xcode_deployment_targets() {
        let settings = "MACOSX_DEPLOYMENT_TARGET = 14.0;\nMACOSX_DEPLOYMENT_TARGET = 15.0;\n";
        let error = unique_xcode_build_setting(settings, "MACOSX_DEPLOYMENT_TARGET")
            .expect_err("conflicting targets must fail");
        assert!(error.to_string().contains("14.0, 15.0"));
    }

    #[test]
    fn injects_required_apple_frameworks_into_other_ldflags() {
        let input =
            "OTHER_LDFLAGS = \"-lwaterui_app -lc++\";\nOTHER_LDFLAGS = \"-lwaterui_app -lc++\";\n";
        let required_flags = vec!["-framework VideoToolbox".to_string()];
        let (output, changed) = inject_other_ldflags(input, &required_flags);
        assert!(changed);
        assert_eq!(output.matches("-framework VideoToolbox").count(), 2);
        assert!(!output.contains("-lwaterui_app"));
    }

    #[test]
    fn linker_flag_injection_is_idempotent() {
        let input = "OTHER_LDFLAGS = \"-lc++ -framework VideoToolbox\";\n";
        let required_flags = vec!["-framework VideoToolbox".to_string()];
        let (output, changed) = inject_other_ldflags(input, &required_flags);
        assert!(!changed);
        assert_eq!(output, input);
    }

    #[test]
    fn removes_redundant_waterui_app_link_flag() {
        let input = "OTHER_LDFLAGS = \"-lwaterui_app -lc++ -framework VideoToolbox\";\n";
        let required_flags = vec!["-framework VideoToolbox".to_string()];
        let (output, changed) = inject_other_ldflags(input, &required_flags);
        assert!(changed);
        assert_eq!(
            output,
            "OTHER_LDFLAGS = \"-lc++ -framework VideoToolbox\";\n"
        );
    }

    #[test]
    fn switches_between_shared_runtime_and_static_link_flags() {
        let input = "OTHER_LDFLAGS = \"-lc++ -framework VideoToolbox\";\n";
        let dynamic_flags = vec![
            "-framework VideoToolbox".to_string(),
            "-lwaterui_dylib".to_string(),
        ];
        let (dynamic, changed) = inject_other_ldflags(input, &dynamic_flags);
        assert!(changed);
        assert!(dynamic.contains("-lwaterui_dylib"));

        let static_flags = vec!["-framework VideoToolbox".to_string()];
        let (static_linked, changed) = inject_other_ldflags(&dynamic, &static_flags);
        assert!(changed);
        assert_eq!(static_linked, input);
    }

    #[test]
    fn parses_frameworks_and_link_args_from_build_output() {
        let output = "cargo:rustc-link-lib=framework=AppKit\ncargo:rustc-link-arg=-rpath\ncargo:rustc-link-arg=/usr/lib/swift\ncargo:rustc-link-lib=framework=Foundation\n";
        let flags = apple_linker_flags_from_build_output(output);
        assert_eq!(
            flags,
            vec![
                "-framework AppKit".to_string(),
                "-rpath".to_string(),
                "/usr/lib/swift".to_string(),
                "-framework Foundation".to_string()
            ]
        );
    }

    #[test]
    fn collects_swift_bridge_archives_and_flags_from_target_build_dir() {
        let dir = tempdir().expect("tempdir");
        let lib_dir = dir.path().join("aarch64-apple-darwin/debug");
        let build_dir = lib_dir.join("build/waterkit-haptic-1234");
        let out_dir = build_dir.join("out");
        std::fs::create_dir_all(&out_dir).expect("create out dir");
        std::fs::write(out_dir.join("CombinedHelper.swift"), "// bridge").expect("write swift");
        std::fs::write(out_dir.join("libHelper.a"), "").expect("write archive");
        std::fs::write(
            build_dir.join("output"),
            "cargo:rustc-link-lib=framework=AppKit\ncargo:rustc-link-arg=-rpath\ncargo:rustc-link-arg=/usr/lib/swift\n",
        )
        .expect("write build output");

        let link_inputs =
            collect_apple_native_link_inputs_sync(&lib_dir).expect("collect native link inputs");

        assert_eq!(link_inputs.archives, vec![out_dir.join("libHelper.a")]);
        assert_eq!(
            link_inputs.linker_flags,
            vec![
                "-framework AppKit".to_string(),
                "-rpath".to_string(),
                "/usr/lib/swift".to_string(),
                "-lHelper".to_string()
            ]
        );
    }

    #[test]
    fn collects_link_flags_from_crates_without_swift_or_archives() {
        // A `-sys` crate that only emits `cargo:rustc-link-lib` directives has
        // nothing in `out/`; its flags must still reach the linker.
        let dir = tempdir().expect("tempdir");
        let lib_dir = dir.path().join("aarch64-apple-darwin/release");
        let sys_build_dir = lib_dir.join("build/system-configuration-sys-1234");
        std::fs::create_dir_all(sys_build_dir.join("out")).expect("create out dir");
        std::fs::write(
            sys_build_dir.join("output"),
            "cargo:rustc-link-lib=framework=SystemConfiguration\n",
        )
        .expect("write build output");

        // A crate can also ship an archive with no Swift bridge at all; the
        // archive and its `-l` flag must still be collected.
        let plain_build_dir = lib_dir.join("build/some-native-5678");
        let plain_out_dir = plain_build_dir.join("out");
        std::fs::create_dir_all(&plain_out_dir).expect("create out dir");
        std::fs::write(plain_out_dir.join("libwrapper.a"), "").expect("write archive");
        std::fs::write(
            plain_build_dir.join("output"),
            "cargo:rustc-link-lib=static=wrapper\n",
        )
        .expect("write build output");

        let link_inputs =
            collect_apple_native_link_inputs_sync(&lib_dir).expect("collect native link inputs");

        assert_eq!(
            link_inputs.archives,
            vec![plain_out_dir.join("libwrapper.a")]
        );
        let mut flags = link_inputs.linker_flags;
        flags.sort_unstable();
        assert_eq!(
            flags,
            vec![
                "-framework SystemConfiguration".to_string(),
                "-lwrapper".to_string()
            ]
        );
    }

    #[test]
    fn collects_framework_flags_from_crates_without_swift_archives() {
        let dir = tempdir().expect("tempdir");
        let lib_dir = dir.path().join("aarch64-apple-darwin/debug");
        let build_dir = lib_dir.join("build/system-configuration-sys-1234");
        std::fs::create_dir_all(build_dir.join("out")).expect("create out dir");
        std::fs::write(
            build_dir.join("output"),
            "cargo:rustc-link-lib=framework=SystemConfiguration\n",
        )
        .expect("write build output");

        let link_inputs =
            collect_apple_native_link_inputs_sync(&lib_dir).expect("collect native link inputs");

        assert!(link_inputs.archives.is_empty());
        assert_eq!(
            link_inputs.linker_flags,
            vec!["-framework SystemConfiguration".to_string()]
        );
    }
    #[test]
    fn collects_link_inputs_from_nightly_build_layout() {
        // Nightly cargo nests unit dirs as `build/<pkg>/<hash>` and records the
        // script's captured stdout at `run/stdout` instead of `output` (#901).
        let dir = tempdir().expect("tempdir");
        let lib_dir = dir.path().join("aarch64-apple-darwin/debug");
        let sys_unit = lib_dir.join("build/system-configuration-sys/1234abcd");
        std::fs::create_dir_all(sys_unit.join("run")).expect("create run dir");
        std::fs::write(
            sys_unit.join("run/stdout"),
            "cargo:rustc-link-lib=framework=SystemConfiguration\n",
        )
        .expect("write build stdout");

        // A compile unit's `out/` holds its own artifacts, not OUT_DIR — it
        // must not be mined for archives.
        let compile_unit = lib_dir.join("build/plain-crate/5678efgh");
        std::fs::create_dir_all(compile_unit.join("out")).expect("create out dir");
        std::fs::write(compile_unit.join("out/libplain_crate.a"), "").expect("write archive");

        let link_inputs =
            collect_apple_native_link_inputs_sync(&lib_dir).expect("collect native link inputs");

        assert!(link_inputs.archives.is_empty());
        assert_eq!(
            link_inputs.linker_flags,
            vec!["-framework SystemConfiguration".to_string()]
        );
    }
}

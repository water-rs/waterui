//! Apple platform build and package utilities.
//!
//! This module provides utility functions for building and packaging Apple apps.
//! These functions are used by `AppleBackend` to implement the `Backend` trait.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{env, fmt::Write};

use color_eyre::eyre::{self, Context, bail};
use smol::fs;
use target_lexicon::Architecture;
use tracing::{debug, info};

use crate::{
    apple::backend::AppleBackend,
    assets::{self, ResolvedFont},
    build::{BuildOptions, RustBuild},
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::{copy_file, run_command_os},
};

// ============================================================================
// Build Utilities
// ============================================================================

/// Build Rust library for an Apple platform.
pub async fn build_rust_lib(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<PathBuf> {
    // Resolve fonts BEFORE cargo build - this ensures icons.json is downloaded
    // for crates like fontawesome7 that need it during build.rs
    let font_declarations = crate::assets::scan_fonts(project).await?;
    let _resolved_fonts = crate::assets::resolve_fonts(font_declarations).await?;

    let triple = options
        .target_triple()
        .cloned()
        .unwrap_or_else(|| platform.triple());
    let target = triple.to_string();
    let target_underscore = target.replace('-', "_");
    let mut build = RustBuild::new(project.root(), triple.clone());
    if let Some(sccache_path) = options.sccache_path() {
        build = build.with_sccache(sccache_path.to_path_buf());
    }
    build = build
        .with_env("PKG_CONFIG_ALLOW_CROSS", "1")
        .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{target_underscore}"), "1")
        .with_env(format!("PKG_CONFIG_ALLOW_CROSS_{target}"), "1");
    let lib_dir = build.build_lib(options.is_release()).await?;

    // If output_dir is specified, copy the library there
    if let Some(output_dir) = options.output_dir() {
        let lib_name = project.crate_name().replace('-', "_");
        let source_lib = lib_dir.join(format!("lib{lib_name}.a"));

        if !source_lib.exists() {
            bail!(
                "Built library not found at {} (expected staticlib for Apple target {})",
                source_lib.display(),
                triple
            );
        }
        fs::create_dir_all(output_dir).await?;
        let dest_lib = output_dir.join("libwaterui_app.a");
        copy_file(&source_lib, &dest_lib).await?;
    }

    Ok(lib_dir)
}

// ============================================================================
// Validation
// ============================================================================

async fn validate_local_apple_backend(project: &Project) -> eyre::Result<()> {
    let Some(waterui_path) = project.manifest().waterui_path.as_deref() else {
        return Ok(());
    };

    let waterui_root = {
        let candidate = PathBuf::from(waterui_path);
        if candidate.is_absolute() {
            candidate
        } else {
            project.root().join(candidate)
        }
    };

    let package_manifest = waterui_root.join("backends/apple/Package.swift");
    if package_manifest.exists() {
        return Ok(());
    }

    let gitmodules_path = waterui_root.join(".gitmodules");
    let submodule_hint = if gitmodules_path.exists() {
        fs::read_to_string(&gitmodules_path)
            .await
            .ok()
            .filter(|c| c.contains("backends/apple"))
            .map(|_| {
                format!(
                    "It looks like `backends/apple` is a git submodule; run `git submodule update --init --recursive` in `{}`.",
                    waterui_root.display()
                )
            })
    } else {
        None
    };

    let mut message = format!(
        "Local Apple backend Swift package manifest not found at `{}`.\n\
         This is typically caused by an incomplete local WaterUI checkout (e.g. missing submodules) or an incorrect `waterui_path` in `Water.toml`.\n",
        package_manifest.display()
    );

    if let Some(hint) = submodule_hint {
        writeln!(&mut message, "{hint}\n").unwrap();
    } else {
        writeln!(
            &mut message,
            "If you're using a local WaterUI checkout, ensure `backends/apple/` exists and contains `Package.swift`."
        ).unwrap();
    }

    bail!("{message}");
}

async fn ensure_apple_framework_linking(xcodeproj: &Path) -> eyre::Result<()> {
    const REQUIRED_FLAGS: [&str; 1] = ["-framework VideoToolbox"];
    let pbxproj_path = xcodeproj.join("project.pbxproj");
    if !pbxproj_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&pbxproj_path)
        .await
        .wrap_err_with(|| format!("Failed to read {}", pbxproj_path.display()))?;
    let (updated, changed) = inject_other_ldflags(&content, &REQUIRED_FLAGS);
    if changed {
        fs::write(&pbxproj_path, updated)
            .await
            .wrap_err_with(|| format!("Failed to write {}", pbxproj_path.display()))?;
        info!(
            "Updated {} to link required Apple media frameworks",
            pbxproj_path.display()
        );
    }

    Ok(())
}

fn inject_other_ldflags(content: &str, required_flags: &[&str]) -> (String, bool) {
    let mut changed = false;
    let mut lines = Vec::new();
    for line in content.lines() {
        if line.contains("OTHER_LDFLAGS = \"")
            && line.contains("-lwaterui_app")
            && let Some((prefix, rest)) = line.split_once("OTHER_LDFLAGS = \"")
            && let Some((flags, suffix)) = rest.split_once("\";")
        {
            let mut merged = flags.to_string();
            let mut line_changed = false;
            for required in required_flags {
                if !flags.contains(required) {
                    if !merged.is_empty() {
                        merged.push(' ');
                    }
                    merged.push_str(required);
                    line_changed = true;
                }
            }
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

// ============================================================================
// Clean
// ============================================================================

/// Clean Xcode build artifacts for an Apple platform.
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
pub async fn package_apple(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> eyre::Result<Artifact> {
    let backend = project
        .apple_backend()
        .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;

    let project_path = project.backend_path::<AppleBackend>();
    let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

    if !xcodeproj.exists() {
        bail!(
            "Xcode project not found at {}. Did you run 'water create'?",
            xcodeproj.display()
        );
    }

    ensure_apple_framework_linking(&xcodeproj).await?;
    validate_local_apple_backend(project).await?;

    // Copy project assets and fonts
    let app_resources_dir = project_path.join(&backend.scheme);
    copy_assets_and_fonts(project, &app_resources_dir).await?;

    // Tell Xcode not to call `water build` again (we already built)
    // SAFETY: CLI runs on main thread before spawning build processes
    unsafe {
        env::set_var("WATERUI_SKIP_RUST_BUILD", "1");
    }

    let configuration = if options.is_debug() {
        "Debug"
    } else {
        "Release"
    };

    let derived_data = project_path.join("DerivedData");
    let target_dir = project.target_dir();
    let triple = platform.triple();

    // Copy the built Rust library to where Xcode expects it
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let lib_dir = target_dir.join(triple.to_string()).join(profile);
    let lib_name = project.crate_name().replace('-', "_");
    let source_lib = lib_dir.join(format!("lib{lib_name}.a"));

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
    let dest_lib = products_dir.join("libwaterui_app.a");
    copy_file(&source_lib, &dest_lib).await?;

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
        OsString::from("ONLY_ACTIVE_ARCHITECTURE=YES"),
        OsString::from("build"),
    ];

    if platform.is_simulator() || options.is_debug() {
        args.extend([
            OsString::from("CODE_SIGNING_ALLOWED=NO"),
            OsString::from("CODE_SIGNING_REQUIRED=NO"),
            OsString::from("CODE_SIGN_IDENTITY=-"),
        ]);
    }

    run_command_os("xcodebuild", args).await?;

    // Reset the environment variable
    unsafe {
        env::set_var("WATERUI_SKIP_RUST_BUILD", "0");
    }

    let app_path = products_dir.join(format!("{}.app", backend.scheme));

    if !app_path.exists() {
        bail!(
            "Built app not found at {}. Check xcodebuild output for errors.",
            app_path.display()
        );
    }

    Ok(Artifact::new(project.bundle_identifier(), app_path))
}

// ============================================================================
// Asset and Font Handling
// ============================================================================

/// Copy project assets and dependency fonts to the app resources directory.
async fn copy_assets_and_fonts(project: &Project, dest_dir: &Path) -> eyre::Result<()> {
    // Stage project assets using platform-native conventions.
    assets::stage_project_assets_for_apple(project, dest_dir).await?;

    // Scan and resolve dependency fonts
    let font_declarations = assets::scan_fonts(project).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(project)?);

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

/// Template for WaterUIFonts.swift
const WATERUI_FONTS_TEMPLATE: &str =
    include_str!("../templates/apple/AppName/WaterUIFonts.swift.tpl");

/// Generate WaterUIFonts.swift file for registering custom fonts.
async fn generate_font_registration_swift(
    fonts: &[ResolvedFont],
    dest_dir: &Path,
) -> eyre::Result<()> {
    // Build font entries
    let font_entries: String = fonts
        .iter()
        .map(|font| {
            let file_name = font
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            format!("            (\"{}\", \"{}\"),", font.name, file_name)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Render template
    let content = WATERUI_FONTS_TEMPLATE.replace("__FONT_ENTRIES__", &font_entries);

    let swift_path = dest_dir.join("WaterUIFonts.swift");
    fs::write(&swift_path, content).await?;

    debug!("Generated {}", swift_path.display());

    Ok(())
}

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the Apple backend.
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

#[cfg(test)]
mod tests {
    use super::inject_other_ldflags;

    #[test]
    fn injects_required_apple_frameworks_into_other_ldflags() {
        let input =
            "OTHER_LDFLAGS = \"-lwaterui_app -lc++\";\nOTHER_LDFLAGS = \"-lwaterui_app -lc++\";\n";
        let (output, changed) = inject_other_ldflags(input, &["-framework VideoToolbox"]);
        assert!(changed);
        assert_eq!(output.matches("-framework VideoToolbox").count(), 2);
    }

    #[test]
    fn linker_flag_injection_is_idempotent() {
        let input = "OTHER_LDFLAGS = \"-lwaterui_app -lc++ -framework VideoToolbox\";\n";
        let (output, changed) = inject_other_ldflags(input, &["-framework VideoToolbox"]);
        assert!(!changed);
        assert_eq!(output, input);
    }
}

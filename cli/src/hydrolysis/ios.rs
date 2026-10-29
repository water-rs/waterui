//! Hydrolysis on iOS: the cocoa-ui `UIKit` host packaged as a flat `.app`.
//!
//! The executable is the generated launcher binary — `cocoa_ui` owns the
//! `UIApplication` lifecycle and launches through its `SceneDelegate` — so the
//! bundle differs from the Apple backend's in the scene manifest declaring a
//! single scene and nothing else. The
//! bundle assembly, asset catalog, signing and install paths are the Apple
//! backend's own: this module only supplies the inputs they take.

use eyre::Context as _;

use crate::{
    apple::{
        app_bundle::{self, AppleAppLayout, AppleSdkSpec, PlistApp, SigningFiles},
        backend::apple_product_name,
        platform::{apple_deployment_target, copy_assets_and_fonts},
    },
    assets,
    build::{BuiltTarget, RustDynamicLibraries},
    device::Artifact,
    hydrolysis::{backend::HydrolysisBackend, platform::synchronize_shared_runtime},
    platform::{PackageOptions, TargetPlatform},
    platforming::packaging::dist_dir,
    project::Project,
};

/// The `dist/` directory name and SDK name of an iOS platform.
fn sdk_name(platform: TargetPlatform) -> eyre::Result<&'static str> {
    match platform {
        TargetPlatform::IOS | TargetPlatform::IOSSimulator => platform
            .sdk_name()
            .ok_or_else(|| eyre::eyre!("{platform:?} has no Apple SDK")),
        _ => eyre::bail!("{platform:?} is not an iOS platform"),
    }
}

/// Assemble, sign and place the Hydrolysis `.app` for an iOS device or
/// simulator.
///
/// # Errors
/// Returns an error when the bundle identifier is not a valid
/// `CFBundleIdentifier`, assets cannot be staged, the bundle cannot be
/// assembled, or signing fails.
pub(super) async fn package_hydrolysis_ios(
    project: &Project,
    platform: TargetPlatform,
    options: &PackageOptions,
    built: &BuiltTarget,
) -> eyre::Result<Artifact> {
    let sdk_name = sdk_name(platform)?;
    // The identifier lands in `Info.plist` and drives signing and
    // provisioning — reject an Apple-invalid one before any staging.
    let bundle_id = project
        .bundle_identifier()
        .apple_bundle_identifier()
        .map_err(|error| eyre::eyre!("{error}"))?;
    let triple = platform.triple();
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let build_manifest = backend_path.join("Cargo.toml");

    // Crate-declared entitlements and `Info.plist` keys, collected from the
    // launcher's graph — a conflict fails before any bundle work.
    let mut declarations = assets::scan_apple_declarations(project, &build_manifest, &[]).await?;
    declarations.supply_app_values(&project.manifest().app_values)?;

    let (_, deployment_target) = apple_deployment_target(project, platform).await?;

    let staging_dir = backend_path.join("ios-assets").join(sdk_name);
    copy_assets_and_fonts(
        project,
        platform,
        &staging_dir,
        &build_manifest,
        &built.app_symbols().await?,
        options.uses_dev_server(),
    )
    .await?;

    let product_name = apple_product_name(project)?;
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let app_path =
        dist_dir(&backend_path, sdk_name, Some(profile)).join(format!("{product_name}.app"));
    let layout = AppleAppLayout::for_app(&app_path, sdk_name);

    let mut info_plist = app_bundle::apple_info_plist(
        &PlistApp {
            display_name: &project.manifest().package.name,
            launch: app_bundle::launch_template_entry(project)?,
            accessory: false,
            multiple_scenes: false,
        },
        project,
        platform,
        &deployment_target,
        product_name,
        &bundle_id,
    );
    declarations.merge_into_info_plist(&mut info_plist)?;

    app_bundle::assemble_app_bundle(
        project.host(),
        &layout,
        &built.artifact,
        product_name,
        &staging_dir,
        &info_plist,
        &AppleSdkSpec {
            sdk_name,
            deployment_target: &deployment_target,
        },
    )
    .await?;

    let shared_libraries = if options.uses_shared_rust_runtime(&triple) {
        Some(
            RustDynamicLibraries::resolve(built, &triple, project)
                .await
                .wrap_err("Failed to resolve the shared WaterUI runtime for the iOS bundle")?,
        )
    } else {
        None
    };
    synchronize_shared_runtime(
        project.host(),
        &layout.frameworks_dir,
        shared_libraries.as_ref(),
        &triple,
    )
    .await?;

    app_bundle::sign_apple_app(
        &layout,
        platform,
        options,
        &SigningFiles {
            project_entitlements: None,
            scratch_root: &backend_path,
            name: product_name,
        },
        project,
        &deployment_target,
        &declarations,
    )
    .await?;

    Ok(Artifact::new(project.bundle_identifier(), app_path))
}

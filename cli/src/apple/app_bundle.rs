//! `.app` bundle assembly with no Xcode project.
//!
//! The CLI lays out the bundle itself — executable, resources,
//! `actool`-compiled asset catalog, `Info.plist`, `codesign`.

use std::path::{Path, PathBuf};

use eyre::{Context, bail};
use smol::fs;
#[cfg(target_os = "macos")]
use tracing::info;

use crate::{
    apple::backend::AppleBackend,
    platform::{DeviceSigning, PackageOptions, TargetPlatform},
    project::Project,
    project_model::templates::TemplateContext,
    project_types::AppleBundleIdentifier,
    utils::{copy_file, run_command_os},
};

#[cfg(target_os = "macos")]
use crate::apple::provisioning;

#[cfg(target_os = "macos")]
use crate::toolchain::Host;

/// The on-disk layout of an assembled application bundle.
///
/// macOS uses the `Contents/` layout (`Contents/MacOS`, `Contents/Resources`,
/// `Contents/Frameworks`); every other Apple platform uses the flat layout —
/// executable, resources and `Frameworks/` all at the bundle root.
#[derive(Debug)]
pub struct AppleAppLayout {
    /// The `<name>.app` directory itself.
    pub app_path: PathBuf,
    /// Directory the executable is copied into.
    pub executable_dir: PathBuf,
    /// Directory resources are copied into.
    pub resources_dir: PathBuf,
    /// Directory dynamic libraries are staged into.
    pub frameworks_dir: PathBuf,
    /// `Info.plist` location.
    pub info_plist_path: PathBuf,
}

impl AppleAppLayout {
    /// Compute the layout for `app_path` under the given SDK name.
    #[must_use]
    pub fn for_app(app_path: &Path, sdk_name: &str) -> Self {
        if sdk_name == "macosx" {
            let contents = app_path.join("Contents");
            Self {
                app_path: app_path.to_path_buf(),
                executable_dir: contents.join("MacOS"),
                resources_dir: contents.join("Resources"),
                frameworks_dir: contents.join("Frameworks"),
                info_plist_path: contents.join("Info.plist"),
            }
        } else {
            Self {
                app_path: app_path.to_path_buf(),
                executable_dir: app_path.to_path_buf(),
                resources_dir: app_path.to_path_buf(),
                frameworks_dir: app_path.join("Frameworks"),
                info_plist_path: app_path.join("Info.plist"),
            }
        }
    }

    /// The shipped executable path for the given product name.
    #[must_use]
    pub fn executable_file(&self, product_name: &str) -> PathBuf {
        self.executable_dir.join(product_name)
    }
}

/// The plist entries every Apple platform carries.
fn common_info_plist_entries(
    ctx: &TemplateContext,
    deployment_target: &str,
    product_name: &str,
    bundle_id: &AppleBundleIdentifier,
) -> plist::Dictionary {
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "CFBundleDisplayName".to_string(),
        plist::Value::String(ctx.app_display_name.clone()),
    );
    dict.insert(
        "CFBundleExecutable".to_string(),
        plist::Value::String(product_name.to_string()),
    );
    dict.insert(
        "CFBundleIdentifier".to_string(),
        plist::Value::String(bundle_id.to_string()),
    );
    dict.insert(
        "CFBundleInfoDictionaryVersion".to_string(),
        plist::Value::String("6.0".to_string()),
    );
    dict.insert(
        "CFBundleName".to_string(),
        plist::Value::String(product_name.to_string()),
    );
    dict.insert(
        "CFBundlePackageType".to_string(),
        plist::Value::String("APPL".to_string()),
    );
    dict.insert(
        "CFBundleShortVersionString".to_string(),
        plist::Value::String("1.0".to_string()),
    );
    dict.insert(
        "CFBundleVersion".to_string(),
        plist::Value::String("1".to_string()),
    );
    dict.insert(
        "LSMinimumSystemVersion".to_string(),
        plist::Value::String(deployment_target.to_string()),
    );
    dict
}

/// Build the `Info.plist` dictionary the Xcode build settings used to produce
/// (`GENERATE_INFOPLIST_FILE` plus the `INFOPLIST_KEY_*` entries the generated
/// project set), for one platform.
#[must_use]
pub fn apple_info_plist(
    ctx: &TemplateContext,
    project: &Project,
    platform: TargetPlatform,
    deployment_target: &str,
    product_name: &str,
    bundle_id: &AppleBundleIdentifier,
) -> plist::Dictionary {
    let mut dict = common_info_plist_entries(ctx, deployment_target, product_name, bundle_id);
    dict.insert(
        "CFBundleDevelopmentRegion".to_string(),
        plist::Value::String("en".to_string()),
    );
    if platform == TargetPlatform::MacOS {
        apply_macos_plist_entries(&mut dict, ctx, project);
    } else {
        apply_mobile_plist_entries(&mut dict, ctx, project, platform, deployment_target);
    }
    dict
}

/// The plist entries an enabled manifest permission produces for `platform`.
fn permission_plist_entries(
    project: &Project,
    macos: bool,
) -> impl Iterator<Item = (String, String)> + '_ {
    project
        .manifest()
        .permissions
        .iter()
        .filter(|(_, entry)| entry.is_enabled())
        .flat_map(move |(key, entry)| {
            let description = entry.description().to_string();
            let keys: Vec<String> = if macos {
                key.macos_usage_description_keys()
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            } else {
                key.ios_plist_key()
                    .and_then(|plist_key| plist_key.strip_prefix("INFOPLIST_KEY_"))
                    .map_or_else(Vec::new, |key| vec![key.to_string()])
            };
            keys.into_iter()
                .map(move |plist_key| (plist_key, description.clone()))
        })
}

/// The macOS-only entries: principal class, menu-bar-agent flag, usage
/// descriptions.
fn apply_macos_plist_entries(
    dict: &mut plist::Dictionary,
    ctx: &TemplateContext,
    project: &Project,
) {
    dict.insert(
        "NSPrincipalClass".to_string(),
        plist::Value::String("NSApplication".to_string()),
    );
    dict.insert(
        "LSUIElement".to_string(),
        plist::Value::Boolean(ctx.accessory),
    );
    for (key, description) in permission_plist_entries(project, true) {
        dict.insert(key, plist::Value::String(description));
    }
}

/// The entries every non-macOS Apple bundle declares.
fn apply_mobile_plist_entries(
    dict: &mut plist::Dictionary,
    ctx: &TemplateContext,
    project: &Project,
    platform: TargetPlatform,
    deployment_target: &str,
) {
    let mut insert = |key: &str, value: plist::Value| {
        dict.insert(key.to_string(), value);
    };
    // iOS caps `CADisplayLink` and `CAMetalLayer` presentation at 60 Hz on
    // ProMotion iPhones unless the app declares this key; WaterUI owns frame
    // pacing per view, so generated apps opt out of the cap. iPad ProMotion
    // and the other mobile platforms are not capped this way.
    if matches!(platform, TargetPlatform::IOS | TargetPlatform::IOSSimulator) {
        insert(
            "CADisableMinimumFrameDurationOnPhone",
            plist::Value::Boolean(true),
        );
    }
    // `MinimumOSVersion` is the floor `installd` enforces; it must not exceed
    // the simulator runtime the bundle installs onto.
    insert(
        "MinimumOSVersion",
        plist::Value::String(deployment_target.to_string()),
    );
    insert(
        "UIDeviceFamily",
        plist::Value::Array(vec![
            plist::Value::Integer(1u64.into()),
            plist::Value::Integer(2u64.into()),
            plist::Value::Integer(4u64.into()),
            plist::Value::Integer(7u64.into()),
        ]),
    );
    insert(
        "UIApplicationSupportsIndirectInputEvents",
        plist::Value::Boolean(true),
    );
    insert(
        "UIBackgroundModes",
        plist::Value::Array(vec![plist::Value::String("audio".to_string())]),
    );
    insert(
        "UIStatusBarStyle",
        plist::Value::String("UIStatusBarStyleDefault".to_string()),
    );
    insert(
        "UISupportedInterfaceOrientations",
        plist::Value::Array(vec![
            plist::Value::String("UIInterfaceOrientationPortrait".to_string()),
            plist::Value::String("UIInterfaceOrientationLandscapeLeft".to_string()),
            plist::Value::String("UIInterfaceOrientationLandscapeRight".to_string()),
        ]),
    );
    insert(
        "UISupportedInterfaceOrientations~ipad",
        plist::Value::Array(vec![
            plist::Value::String("UIInterfaceOrientationPortrait".to_string()),
            plist::Value::String("UIInterfaceOrientationPortraitUpsideDown".to_string()),
            plist::Value::String("UIInterfaceOrientationLandscapeLeft".to_string()),
            plist::Value::String("UIInterfaceOrientationLandscapeRight".to_string()),
        ]),
    );

    // UIKit refuses to launch a scene-configured application without the
    // manifest; the scene delegate class lives in the backend itself
    // (`cocoa_ui`'s `SceneDelegate`), exactly as the generated Xcode project
    // declared it.
    let mut scene_configuration = plist::Dictionary::new();
    scene_configuration.insert(
        "UISceneConfigurationName".to_string(),
        plist::Value::String("Default".to_string()),
    );
    scene_configuration.insert(
        "UISceneDelegateClassName".to_string(),
        plist::Value::String("SceneDelegate".to_string()),
    );
    let mut scene_configurations = plist::Dictionary::new();
    scene_configurations.insert(
        "UIWindowSceneSessionRoleApplication".to_string(),
        plist::Value::Array(vec![plist::Value::Dictionary(scene_configuration)]),
    );
    let mut scene_manifest = plist::Dictionary::new();
    scene_manifest.insert(
        "UIApplicationSupportsMultipleScenes".to_string(),
        plist::Value::Boolean(true),
    );
    scene_manifest.insert(
        "UISceneConfigurations".to_string(),
        plist::Value::Dictionary(scene_configurations),
    );
    insert(
        "UIApplicationSceneManifest",
        plist::Value::Dictionary(scene_manifest),
    );

    let mut launch_screen = plist::Dictionary::new();
    if ctx.launch.has_background {
        launch_screen.insert(
            "UIColorName".to_string(),
            plist::Value::String("LaunchBackground".to_string()),
        );
    }
    if ctx.launch.has_image {
        launch_screen.insert(
            "UIImageName".to_string(),
            plist::Value::String("LaunchImage".to_string()),
        );
        launch_screen.insert(
            "UIImageRespectsSafeAreaInsets".to_string(),
            plist::Value::Boolean(true),
        );
    }
    insert("UILaunchScreen", plist::Value::Dictionary(launch_screen));

    for (key, description) in permission_plist_entries(project, false) {
        dict.insert(key, plist::Value::String(description));
    }
}

/// Assemble the `.app` directory: executable, copied resources, the
/// `actool`-compiled asset catalog, `Info.plist`, `PkgInfo`.
///
/// `staging_dir` is what `copy_assets_and_fonts` populated (`waterui_assets/`,
/// `WaterUIAssets.xcassets/`, `fonts/`).
///
/// # Errors
/// Returns an error when the executable is missing, a copy fails, or `actool`
/// fails to compile the asset catalog.
pub async fn assemble_app_bundle(
    layout: &AppleAppLayout,
    executable: &Path,
    product_name: &str,
    staging_dir: &Path,
    info_plist: &plist::Dictionary,
    sdk_name: &str,
    deployment_target: &str,
) -> eyre::Result<()> {
    if !executable.is_file() {
        bail!(
            "Application executable not found at {}. Build must succeed before packaging.",
            executable.display()
        );
    }
    if layout.app_path.exists() {
        fs::remove_dir_all(&layout.app_path).await?;
    }
    fs::create_dir_all(&layout.executable_dir).await?;
    fs::create_dir_all(&layout.resources_dir).await?;

    let executable_dest = layout.executable_file(product_name);
    copy_file(executable, &executable_dest).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&executable_dest).await?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&executable_dest, perms).await?;
    }

    // `waterui_assets` and `fonts` are ordinary bundle resources.
    for name in ["waterui_assets", "fonts"] {
        let source = staging_dir.join(name);
        if source.is_dir() {
            copy_dir_contents(&source, &layout.resources_dir.join(name)).await?;
        }
    }

    let mut plist = info_plist.clone();
    compile_asset_catalog(
        layout,
        &staging_dir.join("WaterUIAssets.xcassets"),
        sdk_name,
        deployment_target,
        &mut plist,
    )
    .await?;

    let mut plist_file = Vec::new();
    plist::Value::Dictionary(plist)
        .to_writer_xml(&mut plist_file)
        .wrap_err("failed to serialize Info.plist")?;
    fs::write(&layout.info_plist_path, plist_file).await?;

    if sdk_name == "macosx" {
        fs::write(
            layout.app_path.join("Contents").join("PkgInfo"),
            b"APPL????",
        )
        .await?;
    }

    Ok(())
}

/// Compile the staged asset catalog into the bundle's resources directory
/// with `actool`, then merge the `--output-partial-info-plist` keys
/// (`CFBundleIconFile`, `CFBundleIconName`, …) into `info_plist`.
#[cfg(target_os = "macos")]
async fn compile_asset_catalog(
    layout: &AppleAppLayout,
    xcassets: &Path,
    sdk_name: &str,
    deployment_target: &str,
    info_plist: &mut plist::Dictionary,
) -> eyre::Result<()> {
    if !xcassets.is_dir() {
        return Ok(());
    }
    let partial_plist = layout.resources_dir.join(".waterui-actool-partial.plist");
    run_command_os(
        "xcrun",
        [
            "actool".into(),
            "--compile".into(),
            layout.resources_dir.as_os_str().to_os_string(),
            "--platform".into(),
            sdk_name.into(),
            "--minimum-deployment-target".into(),
            deployment_target.into(),
            "--app-icon".into(),
            "AppIcon".into(),
            "--accent-color".into(),
            "AccentColor".into(),
            "--output-partial-info-plist".into(),
            partial_plist.as_os_str().to_os_string(),
            xcassets.as_os_str().to_os_string(),
        ],
    )
    .await
    .wrap_err("actool failed to compile the asset catalog")?;

    if partial_plist.is_file() {
        let partial = plist::Value::from_file(&partial_plist)
            .wrap_err("failed to read actool's partial Info.plist")?;
        if let plist::Value::Dictionary(entries) = partial {
            for (key, value) in entries {
                info_plist.insert(key, value);
            }
        }
        fs::remove_file(&partial_plist).await?;
    }
    Ok(())
}

/// Non-macOS hosts cannot run `actool`; Apple packaging only ever ran on
/// macOS (it drove `xcodebuild` before), so the check is a plain error.
#[cfg(not(target_os = "macos"))]
#[expect(
    clippy::unused_async,
    reason = "keeps the signature of the macOS implementation, which awaits"
)]
async fn compile_asset_catalog(
    _layout: &AppleAppLayout,
    _xcassets: &Path,
    _sdk_name: &str,
    _deployment_target: &str,
    _info_plist: &mut plist::Dictionary,
) -> eyre::Result<()> {
    bail!("Apple packaging requires macOS (actool is part of the Xcode toolchain)")
}

pub(crate) async fn copy_dir_contents(from: &Path, to: &Path) -> eyre::Result<()> {
    let source = from.to_path_buf();
    let destination = to.to_path_buf();
    smol::unblock(move || {
        let mut options = fs_extra::dir::CopyOptions::new();
        options.copy_inside = true;
        options.overwrite = true;
        fs_extra::dir::copy(&source, &destination, &options)
            .map(|_| ())
            .map_err(|error| {
                eyre::eyre!(
                    "Failed to copy resources from {} to {}: {error}",
                    source.display(),
                    destination.display()
                )
            })
    })
    .await
}

/// Sign the assembled bundle per platform: ad-hoc for macOS and simulators,
/// the resolved development identity for devices, nothing for an unsigned
/// device package.
///
/// # Errors
/// Returns an error when signing fails, or when a device build's identity or
/// provisioning profile cannot be resolved.
pub async fn sign_apple_app(
    layout: &AppleAppLayout,
    platform: TargetPlatform,
    options: &PackageOptions,
    backend: &AppleBackend,
    backend_root: &Path,
    project: &Project,
    deployment_target: &str,
) -> eyre::Result<()> {
    let bundle_id = project
        .bundle_identifier()
        .apple_bundle_identifier()
        .map_err(|error| eyre::eyre!("{error}"))?;
    if platform == TargetPlatform::MacOS {
        #[cfg(target_os = "macos")]
        {
            use crate::platform::PackageAudience;

            let requires_stable_identity =
                project.manifest().permissions.iter().any(|(key, entry)| {
                    entry.is_enabled() && !key.macos_usage_description_keys().is_empty()
                });
            let signing = match options.audience() {
                PackageAudience::Development => crate::macos_bundle::MacOsSigning::Development {
                    requires_stable_identity,
                },
                PackageAudience::Distribution => {
                    let entitlements = backend_root
                        .join(&backend.scheme)
                        .join(format!("{}.entitlements", backend.scheme));
                    crate::macos_bundle::MacOsSigning::Distribution(
                        crate::macos_bundle::DistributionSigning::from_manifest(
                            project.manifest().signing.macos.as_ref(),
                            entitlements.is_file().then_some(entitlements),
                        )?,
                    )
                }
            };
            crate::macos_bundle::sign_macos_app(&layout.app_path, &bundle_id, &signing).await?;
            return Ok(());
        }
        #[cfg(not(target_os = "macos"))]
        {
            bail!("Apple packaging requires macOS (codesign is part of the Xcode toolchain)");
        }
    }

    if platform.is_simulator() {
        codesign_bundle(&layout.app_path, &layout.frameworks_dir, "-", None, None).await?;
        return Ok(());
    }

    // A physical Apple OS refuses unsigned code; only an explicitly unsigned
    // package leaves the bundle unsigned (it is signed before install).
    if options.device_signing() == DeviceSigning::Unsigned {
        return Ok(());
    }

    let entitlements = backend_root
        .join(&backend.scheme)
        .join(format!("{}.entitlements", backend.scheme));
    sign_device_app(
        layout,
        &bundle_id,
        options,
        platform,
        &entitlements,
        backend_root,
        deployment_target,
    )
    .await
}

/// Run `codesign` over every member of `frameworks_dir`, then the bundle
/// itself, inside-out the way `xcodebuild` signs.
async fn codesign_bundle(
    app_path: &Path,
    frameworks_dir: &Path,
    identity: &str,
    entitlements: Option<&Path>,
    identifier: Option<&str>,
) -> eyre::Result<()> {
    use smol::stream::StreamExt as _;

    if frameworks_dir.is_dir() {
        let mut members = Vec::new();
        let mut entries = fs::read_dir(frameworks_dir).await?;
        while let Some(entry) = entries.next().await {
            let path = entry?.path();
            if path.is_file()
                || matches!(
                    path.extension().and_then(std::ffi::OsStr::to_str),
                    Some("app" | "framework")
                )
            {
                members.push(path);
            }
        }
        members.sort();
        for member in members {
            codesign_path(&member, identity, None, None).await?;
        }
    }
    codesign_path(app_path, identity, entitlements, identifier).await
}

async fn codesign_path(
    path: &Path,
    identity: &str,
    entitlements: Option<&Path>,
    identifier: Option<&str>,
) -> eyre::Result<()> {
    let mut arguments = vec![
        std::ffi::OsString::from("--force"),
        std::ffi::OsString::from("--sign"),
        std::ffi::OsString::from(identity),
        std::ffi::OsString::from("--timestamp=none"),
    ];
    if let Some(entitlements) = entitlements {
        arguments.push(std::ffi::OsString::from("--entitlements"));
        arguments.push(entitlements.as_os_str().to_owned());
    }
    if let Some(identifier) = identifier {
        arguments.push(std::ffi::OsString::from("--identifier"));
        arguments.push(std::ffi::OsString::from(identifier));
    }
    arguments.push(path.as_os_str().to_owned());
    run_command_os("codesign", arguments).await?;
    Ok(())
}

/// Sign a device build with the resolved development identity.
///
/// The provisioning profile supplies the entitlements (application
/// identifier, team identifier, `get-task-allow`) the signature must claim;
/// the generated project's `.entitlements` file is merged over them. When no
/// installed profile validates for the request, `xcodebuild
/// -allowProvisioningUpdates` mints one first; selection is re-run and a
/// second failure reports every candidate's rejection.
///
/// The entitlement dictionary the signature claims is built from the
/// selected profile's grants — concretized per TN2415 so no wildcard value
/// reaches `codesign` — not copied from it verbatim.
#[cfg(target_os = "macos")]
async fn sign_device_app(
    layout: &AppleAppLayout,
    bundle_id: &AppleBundleIdentifier,
    options: &PackageOptions,
    platform: TargetPlatform,
    entitlements_path: &Path,
    backend_root: &Path,
    deployment_target: &str,
) -> eyre::Result<()> {
    let host = Host::current();
    let team = crate::apple::toolchain::development_team_id(&host).await?;

    let project_entitlements = match fs::read(entitlements_path).await {
        Ok(bytes) => {
            match plist::Value::from_reader(std::io::Cursor::new(bytes)).wrap_err_with(|| {
                format!(
                    "Failed to read entitlements {}",
                    entitlements_path.display()
                )
            })? {
                plist::Value::Dictionary(dict) => dict,
                _ => bail!(
                    "entitlements {} is not a plist dictionary",
                    entitlements_path.display()
                ),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => plist::Dictionary::new(),
        Err(error) => {
            return Err(error).wrap_err_with(|| {
                format!(
                    "Failed to read entitlements {}",
                    entitlements_path.display()
                )
            });
        }
    };

    let request = provisioning::SigningRequest {
        team: &team,
        bundle_id: bundle_id.as_str(),
        device_udid: options.device_udid(),
        entitlements: &project_entitlements,
        platform,
    };
    let selection = match provisioning::select_development_profile(&host, &request).await {
        Ok(selection) => selection,
        Err(provisioning::SelectError::NoMatch(first)) => {
            info!("{first}; asking xcodebuild to provision one");
            provisioning::provision_via_xcodebuild(
                &host,
                &request,
                &backend_root.join("DerivedData/Provisioning"),
                deployment_target,
            )
            .await?;
            match provisioning::select_development_profile(&host, &request).await {
                Ok(selection) => selection,
                Err(provisioning::SelectError::NoMatch(still)) => {
                    bail!(
                        "xcodebuild installed a profile but no installed profile qualifies: {still}"
                    )
                }
                Err(provisioning::SelectError::Failed(error)) => return Err(error),
            }
        }
        Err(provisioning::SelectError::Failed(error)) => return Err(error),
    };

    // Selection already paired the profile with a usable keychain identity:
    // `selection.identity` is the SHA-1 `codesign --sign` takes.
    let identity = &selection.identity;
    copy_file(
        &selection.path,
        layout.app_path.join("embedded.mobileprovision"),
    )
    .await?;

    let entitlements =
        provisioning::signing_entitlements(&selection.data, &request, &selection.app_id_prefix)?;
    let app_name = layout
        .app_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let merged = layout.app_path.with_file_name(format!("{app_name}.xcent"));
    let mut serialized = Vec::new();
    plist::Value::Dictionary(entitlements)
        .to_writer_xml(&mut serialized)
        .wrap_err("failed to serialize signing entitlements")?;
    fs::write(&merged, serialized).await?;

    codesign_bundle(
        &layout.app_path,
        &layout.frameworks_dir,
        identity,
        Some(&merged),
        Some(bundle_id.as_str()),
    )
    .await?;
    info!(
        "Signed {} with {team}/{identity}",
        layout.app_path.display()
    );
    Ok(())
}

/// A physical device cannot be provisioned from a non-macOS host.
#[cfg(not(target_os = "macos"))]
#[expect(
    clippy::unused_async,
    reason = "keeps the signature of the macOS implementation, which awaits"
)]
async fn sign_device_app(
    _layout: &AppleAppLayout,
    _bundle_id: &AppleBundleIdentifier,
    _options: &PackageOptions,
    _platform: TargetPlatform,
    _entitlements_path: &Path,
    _backend_root: &Path,
    _deployment_target: &str,
) -> eyre::Result<()> {
    bail!("Apple device signing requires macOS (codesign and the provisioning profiles live there)")
}

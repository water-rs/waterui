//! Helpers for packaging native binaries into macOS `.app` bundles.

use std::path::{Path, PathBuf};

use askama::Template;
use color_eyre::eyre::{self, bail};
use fs_extra::dir::CopyOptions;
use smol::fs;
#[cfg(target_os = "macos")]
use smol::stream::StreamExt as _;

#[cfg(target_os = "macos")]
use crate::utils::run_command_os;

const CEF_HELPER_VARIANTS: [(&str, &str); 5] = [
    ("", ""),
    (" (Alerts)", ".alerts"),
    (" (GPU)", ".gpu"),
    (" (Plugin)", ".plugin"),
    (" (Renderer)", ".renderer"),
];

#[derive(Template)]
#[template(path = "macos/Info.plist.tpl", escape = "none")]
struct InfoPlistTemplate<'a> {
    bundle_identifier: &'a str,
    app_name: &'a str,
    executable_name: &'a str,
    usage_descriptions: &'a [MacOsUsageDescription],
}

#[cfg(target_os = "macos")]
#[derive(Template)]
#[template(path = "macos/CefHelperInfo.plist.tpl", escape = "none")]
struct CefHelperInfoPlistTemplate<'a> {
    bundle_identifier: &'a str,
    helper_name: &'a str,
    product_name: &'a str,
}

/// Apple Info.plist usage-description entry for a macOS app bundle.
#[derive(Debug, Clone)]
pub struct MacOsUsageDescription {
    /// Raw Info.plist key such as `NSCameraUsageDescription`.
    pub plist_key: &'static str,
    /// User-facing reason declared in `Water.toml`.
    pub description: String,
}

/// Package a compiled binary as a macOS `.app` bundle.
///
/// `resources_dir` is optional and copied to `Contents/Resources` when present.
///
/// # Errors
/// Returns an error if the binary is missing, template rendering fails, or bundle files cannot be created.
pub async fn package_binary_as_app(
    binary_path: &Path,
    bundle_id: &str,
    app_name: &str,
    usage_descriptions: &[MacOsUsageDescription],
    resources_dir: Option<&Path>,
    output_root: &Path,
) -> eyre::Result<PathBuf> {
    if !binary_path.exists() {
        bail!(
            "Binary not found at {}. Build must succeed before packaging.",
            binary_path.display()
        );
    }

    let app_dir = output_root.join(format!("{app_name}.app"));
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let bundle_resources_dir = contents_dir.join("Resources");
    if app_dir.exists() {
        fs::remove_dir_all(&app_dir).await?;
    }
    fs::create_dir_all(&macos_dir).await?;
    fs::create_dir_all(&bundle_resources_dir).await?;

    let executable_name = binary_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| eyre::eyre!("Binary path has no valid executable name"))?;
    let executable_dest = macos_dir.join(executable_name);
    fs::copy(binary_path, &executable_dest).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&executable_dest).await?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&executable_dest, perms).await?;
    }

    if let Some(src_resources) = resources_dir
        && src_resources.exists()
    {
        copy_dir(src_resources, &bundle_resources_dir).await?;
    }

    let plist = InfoPlistTemplate {
        bundle_identifier: bundle_id,
        app_name,
        executable_name,
        usage_descriptions,
    }
    .render()
    .map_err(|error| eyre::eyre!("Failed to render Info.plist template: {error}"))?;
    fs::write(contents_dir.join("Info.plist"), plist).await?;

    Ok(app_dir)
}

/// Signs a local macOS app bundle with an installed development identity.
///
/// Apps declaring protected-resource usage descriptions require a stable
/// identity so macOS can persist privacy grants across local rebuilds. Apps
/// without protected resources use ad-hoc signing when no identity is installed.
///
/// # Errors
///
/// Returns an error when a protected-resource app has no development identity,
/// or when `security`/`codesign` cannot inspect or sign the assembled bundle.
#[cfg(target_os = "macos")]
pub async fn sign_macos_app(
    app_path: &Path,
    bundle_id: &str,
    requires_stable_identity: bool,
) -> eyre::Result<()> {
    let identities = run_command_os(
        "security",
        [
            std::ffi::OsStr::new("find-identity"),
            std::ffi::OsStr::new("-v"),
            std::ffi::OsStr::new("-p"),
            std::ffi::OsStr::new("codesigning"),
        ],
    )
    .await?;
    let identity = if requires_stable_identity {
        first_codesigning_identity(&identities).ok_or_else(|| {
            eyre::eyre!(
                "macOS apps using protected resources require an installed code-signing identity"
            )
        })?
    } else {
        "-"
    };

    let frameworks_dir = app_path.join("Contents").join("Frameworks");
    if frameworks_dir.exists() {
        let mut entries = fs::read_dir(&frameworks_dir).await?;
        let mut framework_paths = Vec::new();
        while let Some(entry) = entries.next().await {
            let path = entry?.path();
            if path.is_file()
                || matches!(
                    path.extension().and_then(std::ffi::OsStr::to_str),
                    Some("app" | "framework")
                )
            {
                framework_paths.push(path);
            }
        }
        framework_paths.sort();
        for framework_path in framework_paths {
            codesign_path(&framework_path, identity, None).await?;
        }
    }

    codesign_path(app_path, identity, Some(bundle_id)).await?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn first_codesigning_identity(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        let (_, identity_and_name) = line.split_once(')')?;
        let identity = identity_and_name.split_whitespace().next()?;
        (identity.len() == 40 && identity.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then_some(identity)
    })
}

#[cfg(target_os = "macos")]
async fn codesign_path(path: &Path, identity: &str, bundle_id: Option<&str>) -> eyre::Result<()> {
    let mut arguments = vec![
        std::ffi::OsString::from("--force"),
        std::ffi::OsString::from("--sign"),
        std::ffi::OsString::from(identity),
        std::ffi::OsString::from("--timestamp=none"),
    ];
    if let Some(bundle_id) = bundle_id {
        arguments.push(std::ffi::OsString::from("--identifier"));
        arguments.push(std::ffi::OsString::from(bundle_id));
    }
    arguments.push(path.as_os_str().to_owned());
    run_command_os("codesign", arguments).await?;
    Ok(())
}

/// Packages the current application executable as the invisible helper variants
/// required by CEF on macOS.
///
/// # Errors
/// Returns an error if the main bundle layout is malformed or the helper cannot
/// be copied and described.
#[cfg(target_os = "macos")]
pub async fn package_cef_helper_app(
    app_dir: &Path,
    main_binary_path: &Path,
    helper_binary_path: &Path,
    bundle_identifier: &str,
) -> eyre::Result<Vec<PathBuf>> {
    let executable_name = main_binary_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| eyre::eyre!("main application has no valid executable name"))?;
    if !helper_binary_path.is_file() {
        bail!(
            "CEF helper executable is missing at {}",
            helper_binary_path.display()
        );
    }
    let main_frameworks_dir = app_dir.join("Contents/Frameworks");
    let mut dynamic_libraries = Vec::new();
    let mut frameworks = fs::read_dir(&main_frameworks_dir).await?;
    while let Some(entry) = frameworks.next().await {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("dylib") {
            dynamic_libraries.push(entry.file_name());
        }
    }

    let mut helper_dirs = Vec::with_capacity(CEF_HELPER_VARIANTS.len());
    for (name_suffix, identifier_suffix) in CEF_HELPER_VARIANTS {
        let helper_name = format!("{executable_name} Helper{name_suffix}");
        let helper_dir = main_frameworks_dir.join(format!("{helper_name}.app"));
        if helper_dir.exists() {
            fs::remove_dir_all(&helper_dir).await?;
        }

        let helper_contents_dir = helper_dir.join("Contents");
        let helper_macos_dir = helper_contents_dir.join("MacOS");
        let helper_frameworks_dir = helper_contents_dir.join("Frameworks");
        fs::create_dir_all(&helper_macos_dir).await?;
        fs::create_dir_all(&helper_frameworks_dir).await?;

        let helper_executable = helper_macos_dir.join(&helper_name);
        fs::copy(helper_binary_path, &helper_executable).await?;
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mut permissions = fs::metadata(&helper_executable).await?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&helper_executable, permissions).await?;
        }
        for name in &dynamic_libraries {
            std::os::unix::fs::symlink(
                Path::new("../../..").join(name),
                helper_frameworks_dir.join(name),
            )?;
        }

        let helper_bundle_identifier = format!("{bundle_identifier}.helper{identifier_suffix}");
        let plist = CefHelperInfoPlistTemplate {
            bundle_identifier: &helper_bundle_identifier,
            helper_name: &helper_name,
            product_name: executable_name,
        }
        .render()
        .map_err(|error| eyre::eyre!("Failed to render CEF helper Info.plist: {error}"))?;
        fs::write(helper_contents_dir.join("Info.plist"), plist).await?;
        fs::write(helper_contents_dir.join("PkgInfo"), b"APPL????").await?;
        helper_dirs.push(helper_dir);
    }

    Ok(helper_dirs)
}

/// Removes CEF helper applications added after a previous macOS build.
///
/// # Errors
///
/// Returns an error when an existing helper application cannot be removed.
#[cfg(target_os = "macos")]
pub async fn remove_cef_helper_apps(app_dir: &Path, executable_name: &str) -> eyre::Result<()> {
    let frameworks_dir = app_dir.join("Contents/Frameworks");
    for (name_suffix, _) in CEF_HELPER_VARIANTS {
        let helper_name = format!("{executable_name} Helper{name_suffix}.app");
        let helper_dir = frameworks_dir.join(helper_name);
        if helper_dir.exists() {
            fs::remove_dir_all(helper_dir).await?;
        }
    }
    Ok(())
}

async fn copy_dir(from: &Path, to: &Path) -> eyre::Result<()> {
    let source = from.to_path_buf();
    let destination = to.to_path_buf();
    smol::unblock(move || {
        let mut options = CopyOptions::new();
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::{first_codesigning_identity, package_cef_helper_app, remove_cef_helper_apps};

    #[test]
    fn parses_first_valid_codesigning_identity() {
        let output = "  1) 645DCB18E20044A687FFE48B0E62D31BF9F6A443 \"Apple Development\"\n     1 valid identities found\n";
        assert_eq!(
            first_codesigning_identity(output),
            Some("645DCB18E20044A687FFE48B0E62D31BF9F6A443")
        );
    }

    #[test]
    fn reports_no_codesigning_identity() {
        assert_eq!(
            first_codesigning_identity("     0 valid identities found\n"),
            None
        );
    }

    #[test]
    fn cef_helpers_are_invisible_variant_bundles_with_shared_runtime_links() {
        smol::block_on(async {
            let temporary = tempfile::tempdir().expect("temporary directory must be available");
            let app = temporary.path().join("Browser.app");
            let frameworks = app.join("Contents/Frameworks");
            let binary = temporary.path().join("browser");
            let helper_binary = temporary.path().join("waterui-cef-helper");
            std::fs::create_dir_all(&frameworks).expect("frameworks directory must be created");
            std::fs::write(&binary, b"browser").expect("fake executable must be written");
            std::fs::write(&helper_binary, b"helper")
                .expect("fake helper executable must be written");
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
                .expect("fake executable must be executable");
            std::fs::set_permissions(&helper_binary, std::fs::Permissions::from_mode(0o755))
                .expect("fake helper executable must be executable");
            std::fs::write(frameworks.join("libwaterui.dylib"), b"runtime")
                .expect("fake runtime must be written");

            let helpers =
                package_cef_helper_app(&app, &binary, &helper_binary, "dev.waterui.browser")
                    .await
                    .expect("CEF helper must package");
            assert_eq!(helpers.len(), 5);
            let helper = &helpers[0];
            let helper_name = "browser Helper";
            let plist = std::fs::read_to_string(helper.join("Contents/Info.plist"))
                .expect("helper plist must be readable");
            assert!(plist.contains("<key>LSUIElement</key>"));
            assert!(plist.contains("dev.waterui.browser.helper"));
            assert!(helper.join("Contents/MacOS").join(helper_name).is_file());
            assert_eq!(
                std::fs::read_link(helper.join("Contents/Frameworks/libwaterui.dylib"))
                    .expect("helper runtime must be linked"),
                std::path::PathBuf::from("../../../libwaterui.dylib")
            );
            let renderer = &helpers[4];
            let renderer_plist = std::fs::read_to_string(renderer.join("Contents/Info.plist"))
                .expect("renderer helper plist must be readable");
            assert!(renderer_plist.contains("browser Helper (Renderer)"));
            assert!(renderer_plist.contains("dev.waterui.browser.helper.renderer"));

            remove_cef_helper_apps(&app, "browser")
                .await
                .expect("CEF helpers must be removable before an incremental build");
            for helper in helpers {
                assert!(!helper.exists());
            }
        });
    }
}

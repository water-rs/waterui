//! Helpers for packaging native binaries into macOS `.app` bundles.

use std::path::{Path, PathBuf};

use askama::Template;
use color_eyre::eyre::{self, bail};
use fs_extra::dir::CopyOptions;
use smol::fs;
use smol::stream::StreamExt as _;

use crate::utils::run_command_os;

#[derive(Template)]
#[template(path = "macos/Info.plist.tpl", escape = "none")]
struct InfoPlistTemplate<'a> {
    bundle_identifier: &'a str,
    app_name: &'a str,
    executable_name: &'a str,
    usage_descriptions: &'a [MacOsUsageDescription],
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
    let identity = first_codesigning_identity(&identities);
    if requires_stable_identity && identity.is_none() {
        bail!("macOS apps using protected resources require an installed code-signing identity");
    }
    let identity = identity.unwrap_or("-");

    let frameworks_dir = app_path.join("Contents").join("Frameworks");
    if frameworks_dir.exists() {
        let mut entries = fs::read_dir(&frameworks_dir).await?;
        let mut framework_paths = Vec::new();
        while let Some(entry) = entries.next().await {
            let path = entry?.path();
            if path.is_file() {
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
    use super::first_codesigning_identity;

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
}

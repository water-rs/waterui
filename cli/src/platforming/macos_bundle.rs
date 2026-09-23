//! Helpers for packaging native binaries into macOS `.app` bundles.

use std::path::{Path, PathBuf};

use askama::Template;
use color_eyre::eyre::{self, bail};
use fs_extra::dir::CopyOptions;
use smol::fs;

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

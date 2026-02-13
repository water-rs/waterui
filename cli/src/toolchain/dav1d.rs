//! dav1d toolchain resolution for cross-platform builds.
//!
//! Strategy:
//! 1. Prefer prebuilt binaries for known WaterUI targets.
//! 2. Fall back to source build via `SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=always`.

use std::ffi::OsString;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, Context, OptionExt};
use smol::fs;
use smol::stream::StreamExt;
use tracing::{debug, info, warn};

const PREBUILT_VERSION: &str = "1.5.0";
const DEFAULT_PREBUILT_BASE_URL: &str = "https://assets.waterui.dev/toolchains/dav1d";

/// WaterUI build targets that should prefer prebuilt `dav1d`.
///
/// This list mirrors targets used by current CLI workflows:
/// - Apple: iOS, iOS Simulator, macOS
/// - Android: all supported ABIs
/// - GTK4: Linux / Windows host targets
pub const WATERUI_DAV1D_TARGETS: &[&str] = &[
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-linux-android",
    "armv7-linux-androideabi",
    "i686-linux-android",
    "x86_64-linux-android",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "x86_64-pc-windows-gnu",
];

/// Build environment variables for dav1d resolution.
///
/// Prefers prebuilt binaries for supported targets. If unavailable, falls back
/// to source build using `system-deps` build-internal mode.
pub async fn cargo_env_for_target(target: &str) -> Vec<(String, OsString)> {
    if env_bool("WATERUI_DAV1D_FORCE_BUILD_INTERNAL", false) {
        debug!("dav1d: forced build-internal mode for target {target}");
        return build_internal_env();
    }

    let prebuilt_target_supported = WATERUI_DAV1D_TARGETS.contains(&target);
    if !prebuilt_target_supported {
        debug!("dav1d: target {target} not in prebuilt matrix; using build-internal fallback");
        return build_internal_env();
    }

    match resolve_prebuilt_root(target).await {
        Ok(Some(root)) => {
            info!(
                "dav1d: using prebuilt binaries for target {} from {}",
                target,
                root.display()
            );
            prebuilt_env(&root)
        }
        Ok(None) => {
            warn!(
                "dav1d: prebuilt binary unavailable for target {}; using build-internal fallback",
                target
            );
            build_internal_env()
        }
        Err(error) => {
            warn!(
                "dav1d: failed to provision prebuilt for target {}: {}; falling back to build-internal",
                target, error
            );
            build_internal_env()
        }
    }
}

/// Check local prebuilt cache coverage against WaterUI target matrix.
pub async fn local_prebuilt_coverage() -> eyre::Result<Vec<(&'static str, bool)>> {
    let mut out = Vec::with_capacity(WATERUI_DAV1D_TARGETS.len());
    for &target in WATERUI_DAV1D_TARGETS {
        let exists = if let Some(custom_root) = custom_prebuilt_root() {
            is_valid_prebuilt_root(&custom_root.join(target)).await?
        } else {
            is_valid_prebuilt_root(&cached_prebuilt_root(target)?).await?
        };
        out.push((target, exists));
    }
    Ok(out)
}

async fn resolve_prebuilt_root(target: &str) -> eyre::Result<Option<PathBuf>> {
    if let Some(custom_root) = custom_prebuilt_root() {
        let custom_target_root = custom_root.join(target);
        if is_valid_prebuilt_root(&custom_target_root).await? {
            return Ok(Some(custom_target_root));
        }

        warn!(
            "dav1d: custom prebuilt dir set but target {} is missing or invalid: {}",
            target,
            custom_target_root.display()
        );
    }

    let cache_root = cached_prebuilt_root(target)?;
    if is_valid_prebuilt_root(&cache_root).await? {
        return Ok(Some(cache_root));
    }

    if !env_bool("WATERUI_DAV1D_PREBUILT_DOWNLOAD", true) {
        debug!(
            "dav1d: download disabled via WATERUI_DAV1D_PREBUILT_DOWNLOAD for target {}",
            target
        );
        return Ok(None);
    }

    download_prebuilt_to_cache(target, &cache_root).await?;
    if is_valid_prebuilt_root(&cache_root).await? {
        return Ok(Some(cache_root));
    }

    Ok(None)
}

async fn download_prebuilt_to_cache(target: &str, cache_root: &Path) -> eyre::Result<()> {
    let base_url = std::env::var("WATERUI_DAV1D_PREBUILT_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_PREBUILT_BASE_URL.to_string());
    let url = format!(
        "{}/{}/{}.zip",
        base_url.trim_end_matches('/'),
        PREBUILT_VERSION,
        target
    );

    if !url.starts_with("https://") {
        eyre::bail!(
            "Refusing non-HTTPS dav1d prebuilt URL: {} (override with HTTPS)",
            url
        );
    }

    info!("dav1d: downloading prebuilt archive from {}", url);
    let bytes = download_bytes(&url).await?;

    if cache_root.exists() {
        let _ = fs::remove_dir_all(cache_root).await;
    }
    if let Some(parent) = cache_root.parent() {
        fs::create_dir_all(parent).await?;
    }

    let temp_root = cache_root.with_extension(format!("tmp-{}", std::process::id()));
    if temp_root.exists() {
        let _ = fs::remove_dir_all(&temp_root).await;
    }
    fs::create_dir_all(&temp_root).await?;

    extract_zip_bytes(&bytes, &temp_root).await?;

    fs::write(temp_root.join(".ready"), PREBUILT_VERSION.as_bytes()).await?;

    if let Err(error) = fs::rename(&temp_root, cache_root).await {
        // Handle race where another process finalized the cache first.
        let _ = fs::remove_dir_all(&temp_root).await;
        if !cache_root.exists() {
            return Err(error).wrap_err_with(|| {
                format!(
                    "Failed to finalize dav1d prebuilt cache at {}",
                    cache_root.display()
                )
            });
        }
    }

    Ok(())
}

async fn download_bytes(url: &str) -> eyre::Result<Vec<u8>> {
    use zenwave::{Client, Method, redirect::FollowRedirect};

    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, url)
        .await
        .wrap_err_with(|| format!("Failed to download {url}"))?;

    if !response.status().is_success() {
        eyre::bail!("Download failed: HTTP {} from {}", response.status(), url);
    }

    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .wrap_err_with(|| format!("Failed to read download body from {url}"))?;
    Ok(bytes.to_vec())
}

async fn extract_zip_bytes(bytes: &[u8], destination: &Path) -> eyre::Result<()> {
    let bytes = bytes.to_vec();
    let destination = destination.to_path_buf();

    smol::unblock(move || -> eyre::Result<()> {
        let reader = Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(reader).wrap_err("Invalid zip archive")?;

        for idx in 0..zip.len() {
            let mut file = zip.by_index(idx)?;
            let Some(enclosed) = file.enclosed_name() else {
                continue;
            };

            let out_path = destination.join(enclosed);
            if file.is_dir() {
                std::fs::create_dir_all(&out_path)?;
                continue;
            }

            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&out_path)?;
            std::io::copy(&mut file, &mut out)?;
            out.flush()?;
        }

        Ok(())
    })
    .await
}

async fn is_valid_prebuilt_root(root: &Path) -> eyre::Result<bool> {
    if !root.exists() || !root.is_dir() {
        return Ok(false);
    }

    let lib_dir = root.join("lib");
    if !lib_dir.exists() || !lib_dir.is_dir() {
        return Ok(false);
    }

    let mut entries = fs::read_dir(&lib_dir).await?;
    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if matches!(
            name,
            "libdav1d.a" | "libdav1d.so" | "dav1d.lib" | "libdav1d.lib"
        ) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn build_internal_env() -> Vec<(String, OsString)> {
    vec![
        (
            "SYSTEM_DEPS_DAV1D_BUILD_INTERNAL".to_string(),
            OsString::from("always"),
        ),
        (
            "SYSTEM_DEPS_DAV1D_LINK".to_string(),
            OsString::from("static"),
        ),
    ]
}

fn prebuilt_env(root: &Path) -> Vec<(String, OsString)> {
    let lib_dir = root.join("lib");
    let include_dir = root.join("include");
    let mut envs = vec![
        (
            "SYSTEM_DEPS_DAV1D_NO_PKG_CONFIG".to_string(),
            OsString::from("1"),
        ),
        ("SYSTEM_DEPS_DAV1D_LIB".to_string(), OsString::from("dav1d")),
        (
            "SYSTEM_DEPS_DAV1D_SEARCH_NATIVE".to_string(),
            lib_dir.into_os_string(),
        ),
        (
            "SYSTEM_DEPS_DAV1D_LINK".to_string(),
            OsString::from("static"),
        ),
    ];

    if include_dir.exists() {
        envs.push((
            "SYSTEM_DEPS_DAV1D_INCLUDE".to_string(),
            include_dir.into_os_string(),
        ));
    }

    envs
}

fn custom_prebuilt_root() -> Option<PathBuf> {
    std::env::var_os("WATERUI_DAV1D_PREBUILT_DIR").map(PathBuf::from)
}

fn cached_prebuilt_root(target: &str) -> eyre::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_eyre("Could not determine home directory for dav1d cache")?;
    Ok(home
        .join(".water")
        .join("cache")
        .join("toolchain")
        .join("dav1d")
        .join(PREBUILT_VERSION)
        .join(target))
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::{WATERUI_DAV1D_TARGETS, prebuilt_env};

    #[test]
    fn target_matrix_covers_cli_primary_targets() {
        assert!(WATERUI_DAV1D_TARGETS.contains(&"aarch64-apple-ios"));
        assert!(WATERUI_DAV1D_TARGETS.contains(&"x86_64-linux-android"));
        assert!(WATERUI_DAV1D_TARGETS.contains(&"aarch64-apple-darwin"));
        assert!(WATERUI_DAV1D_TARGETS.contains(&"x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn prebuilt_env_contains_required_system_deps_keys() {
        let root = std::path::Path::new("/tmp/dav1d-test");
        let envs = prebuilt_env(root);
        let keys: std::collections::BTreeSet<_> = envs.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains("SYSTEM_DEPS_DAV1D_NO_PKG_CONFIG"));
        assert!(keys.contains("SYSTEM_DEPS_DAV1D_LIB"));
        assert!(keys.contains("SYSTEM_DEPS_DAV1D_SEARCH_NATIVE"));
        assert!(keys.contains("SYSTEM_DEPS_DAV1D_LINK"));
    }
}

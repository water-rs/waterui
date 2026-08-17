use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use color_eyre::eyre::{Result, bail, eyre};
use tracing::info;

#[allow(clippy::redundant_pub_crate)]
pub(crate) fn support_app_path(name: &str) -> Result<PathBuf> {
    Ok(crate::water_dir::water_home_dir()?.join(name))
}

/// Discards a support application generated against a different `WaterUI`.
///
/// A support application records absolute paths to the checkout it was built
/// from. One directory is shared by every project, so a checkout that has since
/// moved or been deleted leaves behind a project whose manifests point at
/// nothing — and reading its workspace fails long before the scaffolder would
/// have noticed the mismatch and rebuilt it. Throwing it away first is what
/// makes the rebuild happen.
///
/// A support application is two halves: the small project under the Water home,
/// and the generated backend workspace in the build cache, which is where the
/// manifests that pin the checkout actually live. Discarding one without the
/// other leaves exactly the file the next build reads, so both go together —
/// including when only the cached half is left behind.
#[allow(clippy::redundant_pub_crate)]
pub(crate) async fn discard_support_app_for_other_runtime(
    path: &Path,
    waterui_path: Option<&Path>,
) -> Result<()> {
    let manifest = path.join("Water.toml");
    let contents = match smol::fs::read_to_string(&manifest).await {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(eyre!("Failed to read {}: {error}", manifest.display()));
        }
    };

    let recorded = contents.as_ref().and_then(|contents| {
        contents
            .lines()
            .find_map(|line| line.trim().strip_prefix("waterui_path"))
            .and_then(|value| value.split('"').nth(1))
            .map(PathBuf::from)
    });

    let matches = contents.is_some()
        && match (&recorded, waterui_path) {
            (Some(recorded), Some(wanted)) => {
                let recorded = recorded.canonicalize().unwrap_or_else(|_| recorded.clone());
                let wanted = wanted.canonicalize().unwrap_or_else(|_| wanted.to_path_buf());
                recorded == wanted
            }
            (None, None) => true,
            _ => false,
        };
    if matches {
        return Ok(());
    }

    let cache = crate::water_dir::build_cache_container_for(path).await?;
    if !path.exists() && !cache.exists() {
        return Ok(());
    }

    info!(
        path = %path.display(),
        cache = %cache.display(),
        "Support app was generated against a different WaterUI; regenerating"
    );
    if path.exists() {
        smol::fs::remove_dir_all(path)
            .await
            .map_err(|error| eyre!("Failed to remove the stale support app: {error}"))?;
    }
    if cache.exists() {
        smol::fs::remove_dir_all(&cache).await.map_err(|error| {
            eyre!("Failed to remove the stale support app's build cache: {error}")
        })?;
    }
    Ok(())
}

#[allow(clippy::redundant_pub_crate)]
pub(crate) async fn ensure_support_app<F, Fut>(
    path: &Path,
    metadata_file: &str,
    desired_signature: &str,
    app_kind: &str,
    scaffold: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let metadata_path = path.join(metadata_file);
    let cargo_path = path.join("Cargo.toml");

    let mut needs_scaffold = !cargo_path.exists();
    if !needs_scaffold {
        let stored_signature = match smol::fs::read_to_string(&metadata_path).await {
            Ok(signature) => signature,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(eyre!(
                    "Failed to read {app_kind} metadata {}: {error}",
                    metadata_path.display()
                ));
            }
        };
        if stored_signature.trim() != desired_signature {
            needs_scaffold = true;
        }
    }

    if needs_scaffold {
        if path.exists() {
            remove_dir_all_retry(path).await?;
        }
        info!("Scaffolding {app_kind} app at {}", path.display());
        scaffold().await?;
        smol::fs::write(&metadata_path, desired_signature.as_bytes()).await?;
    } else if !metadata_path.exists() {
        smol::fs::write(&metadata_path, desired_signature.as_bytes()).await?;
    }

    Ok(())
}

async fn remove_dir_all_retry(path: &Path) -> Result<()> {
    const ATTEMPTS: usize = 6;

    for attempt in 0..ATTEMPTS {
        match smol::fs::remove_dir_all(path).await {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error)
                if error.kind() == std::io::ErrorKind::DirectoryNotEmpty
                    && attempt + 1 < ATTEMPTS =>
            {
                smol::Timer::after(Duration::from_millis(50 * (attempt as u64 + 1))).await;
            }
            Err(error) => return Err(error.into()),
        }
    }

    bail!("Failed to remove support app directory after retries");
}

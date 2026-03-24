use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use color_eyre::eyre::{Result, bail, eyre};
use tracing::info;

pub(crate) fn support_app_path(name: &str) -> Result<PathBuf> {
    Ok(crate::water_dir::water_home_dir()?.join(name))
}

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

    bail!("Failed to remove support app directory after retries")
}

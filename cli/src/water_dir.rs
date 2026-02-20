//! Management of the `.water` directory for playground projects.
//!
//! The `.water` directory stores auto-generated backends for playground projects.
//! A metadata file tracks the CLI commit hash to detect when backends need regeneration.

use std::path::Path;

use color_eyre::eyre;
use smol::fs;

/// The CLI commit hash embedded at build time.
pub const CLI_COMMIT: &str = env!("WATERUI_CLI_COMMIT");

const METADATA_FILE: &str = ".water/metadata";

/// Ensure the `.water` directory is valid for the current CLI version.
///
/// If the CLI commit hash has changed since the `.water` directory was created,
/// the entire directory is deleted to force regeneration of backends.
///
/// # Errors
///
/// Returns an error if file operations fail.
pub async fn ensure_valid(project_root: &Path) -> eyre::Result<()> {
    let metadata_path = project_root.join(METADATA_FILE);
    let water_dir = project_root.join(".water");

    if water_dir.exists() {
        // Check if metadata exists and matches current CLI commit
        let should_clean = if metadata_path.exists() {
            match fs::read_to_string(&metadata_path).await {
                Ok(stored_commit) => stored_commit.trim() != CLI_COMMIT,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
                Err(err) => {
                    return Err(eyre::eyre!(
                        "failed to read Water metadata at {}: {err}",
                        metadata_path.display()
                    ));
                }
            }
        } else {
            // No metadata file - old .water directory, clean it
            true
        };

        if should_clean {
            tracing::info!("CLI version changed, cleaning .water directory");
            fs::remove_dir_all(&water_dir).await?;
        }
    }

    // Ensure .water directory exists with metadata
    if !water_dir.exists() {
        fs::create_dir_all(&water_dir).await?;
        fs::write(&metadata_path, CLI_COMMIT).await?;
    }

    Ok(())
}

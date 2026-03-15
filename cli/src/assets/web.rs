use std::path::Path;

use color_eyre::eyre;
use smol::fs;

use crate::project::Project;

pub async fn stage_for_web(project: &Project, site_root: &Path) -> eyre::Result<()> {
    let source = project.assets_dir();
    if !source.exists() {
        return Ok(());
    }

    let destination = site_root.join(project.assets_path());
    if destination.exists() {
        fs::remove_dir_all(&destination).await?;
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::create_dir_all(&destination).await?;

    smol::unblock(move || {
        let mut options = fs_extra::dir::CopyOptions::new();
        options.copy_inside = true;
        options.overwrite = true;
        fs_extra::dir::copy(&source, &destination, &options)
            .map(|_| ())
            .map_err(|error| {
                eyre::eyre!(
                    "Failed to copy web assets from {} to {}: {error}",
                    source.display(),
                    destination.display()
                )
            })
    })
    .await
}

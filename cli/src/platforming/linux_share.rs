//! Freedesktop launcher material for Linux distributions.
//!
//! Packaged Linux applications carry a `share/` prefix tree next to the
//! binary — a desktop entry plus hicolor icon-theme icons — so installing
//! the application into a prefix (or a package recipe copying the tree)
//! gives launchers and taskbars the app icon.

use std::path::Path;

use askama::Template;
use color_eyre::eyre::{self};
use smol::fs;

use crate::project::Project;
use crate::project_model::assets;

#[derive(Template)]
#[template(path = "linux/app.desktop.tpl", escape = "none")]
struct DesktopEntryTemplate<'a> {
    app_name: &'a str,
    executable_name: &'a str,
    bundle_identifier: &'a str,
}

/// Writes `share/applications/<bundle-id>.desktop` and the hicolor icon tree
/// under `share_dir`.
///
/// # Errors
///
/// Fails when the desktop entry cannot be rendered or files cannot be
/// written.
pub async fn write_linux_share_material(
    project: &Project,
    app_name: &str,
    executable_name: &str,
    share_dir: &Path,
) -> eyre::Result<()> {
    let applications_dir = share_dir.join("applications");
    fs::create_dir_all(&applications_dir).await?;
    let entry = DesktopEntryTemplate {
        app_name,
        executable_name,
        bundle_identifier: project.bundle_identifier(),
    }
    .render()
    .map_err(|error| eyre::eyre!("Failed to render desktop entry: {error}"))?;
    fs::write(
        applications_dir.join(format!("{}.desktop", project.bundle_identifier())),
        entry,
    )
    .await?;

    assets::stage_hicolor_icons(project, &share_dir.join("icons")).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::project::{CreateOptions, PackageType, Project};
    use crate::project_model::project_types::BundleIdentifier;

    #[test]
    fn share_tree_carries_desktop_entry_and_hicolor_icons() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("temp dir");
            let root = dir.path().join("share-example");
            let project = Project::create(
                &root,
                CreateOptions {
                    name: "Share Example".to_string(),
                    bundle_identifier: BundleIdentifier::try_from("dev.waterui.shareexample")
                        .expect("bundle identifier"),
                    package_type: PackageType::Playground,
                    waterui_path: None,
                    author: "Lexo Liu".to_string(),
                },
            )
            .await
            .expect("project creation must succeed");

            let share = dir.path().join("share");
            super::write_linux_share_material(&project, "Share Example", "share-example", &share)
                .await
                .expect("share material must be written");

            let entry = std::fs::read_to_string(
                share.join("applications/dev.waterui.shareexample.desktop"),
            )
            .expect("desktop entry must exist");
            assert!(entry.contains("Name=Share Example"));
            assert!(entry.contains("Exec=share-example"));
            assert!(entry.contains("Icon=dev.waterui.shareexample"));

            let icon_128 = share.join("icons/hicolor/128x128/apps/dev.waterui.shareexample.png");
            let decoded = image::open(&icon_128).expect("hicolor icon must decode");
            assert_eq!(decoded.width(), 128);
        });
    }
}

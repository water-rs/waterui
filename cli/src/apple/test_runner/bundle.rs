//! Staging the `<binary>.app` the application route installs and launches.

use eyre::{Context as _, bail};
use smol::fs;

use crate::apple::app_bundle::AppleAppLayout;

use super::binary::{HarnessManifest, TestBinary};

/// A staged `<binary>.app` — layout from [`AppleAppLayout::for_app`], flat
/// for iOS and the simulator, `Contents/` for Catalyst.
#[derive(Debug)]
pub struct HarnessBundle {
    layout: AppleAppLayout,
    bundle_id: String,
}

impl HarnessBundle {
    /// Replaces `<binary>.app` with a fresh bundle: `Info.plist` is the
    /// manifest's bytes verbatim and the executable is the binary itself,
    /// named by `CFBundleExecutable`. The path stays the same for a given
    /// binary, so a later invocation finds the copy already there and
    /// `cargo clean` removes it.
    ///
    /// # Errors
    /// Returns an error when the bundle cannot be staged.
    pub async fn stage(
        binary: &TestBinary,
        manifest: &HarnessManifest,
        sdk_name: &str,
    ) -> eyre::Result<Self> {
        let app = binary.path().with_extension("app");
        let layout = AppleAppLayout::for_app(&app, sdk_name);
        if layout.app_path().exists() {
            fs::remove_dir_all(&layout.app_path())
                .await
                .wrap_err_with(|| format!("failed to remove {}", layout.app_path().display()))?;
        }
        fs::create_dir_all(&layout.executable_dir())
            .await
            .wrap_err_with(|| format!("failed to create {}", layout.executable_dir().display()))?;
        fs::write(&layout.info_plist_path(), manifest.bytes())
            .await
            .wrap_err_with(|| format!("failed to write {}", layout.info_plist_path().display()))?;
        let executable = layout.executable_file(manifest.executable());
        crate::utils::copy_file(binary.path(), &executable)
            .await
            .wrap_err_with(|| format!("failed to stage {}", executable.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut permissions = fs::metadata(&executable).await?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&executable, permissions).await?;
        }
        Ok(Self {
            layout,
            bundle_id: manifest.bundle_id().to_string(),
        })
    }

    /// The bundle's on-disk layout.
    #[must_use]
    pub const fn layout(&self) -> &AppleAppLayout {
        &self.layout
    }

    /// The bundle identifier the staged `Info.plist` declares.
    #[must_use]
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// Rewrites `CFBundleIdentifier` in the staged `Info.plist` — the
    /// device destination signs every harness under its caller-supplied
    /// identity, so the manifest's own identifier is replaced there.
    ///
    /// # Errors
    /// Returns an error when the staged manifest cannot be read or
    /// rewritten, or `bundle_id` is malformed.
    pub fn rewrite_identifier(&mut self, bundle_id: &str) -> eyre::Result<()> {
        if bundle_id.is_empty() {
            bail!("the device bundle identifier must not be empty");
        }
        let path = &self.layout.info_plist_path();
        let mut value = plist::Value::from_file(path)
            .wrap_err_with(|| format!("failed to read {}", path.display()))?;
        let plist::Value::Dictionary(dictionary) = &mut value else {
            bail!("{} is not a plist dictionary", path.display());
        };
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            plist::Value::String(bundle_id.to_string()),
        );
        let mut bytes = Vec::new();
        value
            .to_writer_xml(&mut bytes)
            .wrap_err("failed to serialize the rewritten Info.plist")?;
        std::fs::write(path, bytes)
            .wrap_err_with(|| format!("failed to write {}", path.display()))?;
        self.bundle_id = bundle_id.to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::binary::MachOPlatform;
    use super::super::binary::tests::{manifest_plist, stage_binary};
    use super::*;

    #[test]
    fn stage_lays_out_the_flat_bundle_verbatim() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plist = manifest_plist();
        let path = stage_binary(dir.path(), MachOPlatform::IOSSIMULATOR.0, Some(&plist));
        let binary = TestBinary::read(&path).expect("read the fixture");
        let manifest = binary.manifest().expect("marked");
        let mut bundle = smol::block_on(HarnessBundle::stage(&binary, manifest, "iphonesimulator"))
            .expect("stage");

        // Flat iOS layout: the bundle's Info.plist at its root is the
        // manifest's bytes verbatim, and the executable sits beside it
        // under the manifest's own CFBundleExecutable name.
        assert_eq!(bundle.layout().executable_dir(), bundle.layout().app_path());
        assert_eq!(
            std::fs::read(bundle.layout().info_plist_path()).expect("plist"),
            plist
        );
        assert!(
            bundle
                .layout()
                .executable_file(manifest.executable())
                .exists()
        );
        assert_eq!(bundle.bundle_id(), "dev.waterui.tests");
        assert_eq!(
            bundle.layout().app_path(),
            std::fs::canonicalize(&path)
                .expect("canonical fixture path")
                .with_extension("app")
        );

        bundle
            .rewrite_identifier("cool.test.bench")
            .expect("rewrite");
        let rewritten = plist::Value::from_file(bundle.layout().info_plist_path())
            .expect("parse rewritten plist");
        assert_eq!(
            rewritten
                .as_dictionary()
                .and_then(|d| d.get("CFBundleIdentifier"))
                .and_then(plist::Value::as_string),
            Some("cool.test.bench")
        );
    }
}

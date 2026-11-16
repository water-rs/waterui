//! `Hydrolysis` backend support for `WaterUI` CLI.

pub mod android;
pub mod backend;
pub mod platform;
mod web_launch;

/// Development builds enable inspection when the resolved framework exposes it.
fn development_inspector(
    manifest: &std::path::Path,
    release: bool,
) -> std::io::Result<Option<&'static str>> {
    if !release && crate::templates::generated_manifest_declares(manifest, "inspector")? {
        Ok(Some("inspector"))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn inspection_defaults_only_to_development_builds() {
        let dir = tempfile::tempdir().expect("manifest directory");
        let path = dir.path().join("Cargo.toml");
        for declared in [false, true] {
            let mut manifest = cargo_toml::Manifest::<()>::default();
            if declared {
                manifest.features.insert("inspector".to_owned(), Vec::new());
            }
            std::fs::write(
                &path,
                toml::to_string(&manifest).expect("serialize manifest"),
            )
            .expect("write manifest");
            assert_eq!(
                super::development_inspector(&path, false).expect("development features"),
                declared.then_some("inspector")
            );
            assert_eq!(
                super::development_inspector(&path, true).expect("release features"),
                None
            );
        }
    }
}

//! Locating the artifacts Gradle package tasks produce.
//!
//! `assemble<Variant>` writes `output-metadata.json` next to the APKs in
//! `app/build/outputs/apk/<variant>/`, so the file name is read from that
//! document instead of guessed. `bundle<Variant>` writes no such document
//! next to its output — in AGP 8.x `FinalizeBundleTask` registers only the
//! `.aab` itself on `SingleArtifact.BUNDLE` — so the bundle is located by
//! listing `app/build/outputs/bundle/<variant>/` for exactly one `.aab`.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, Context as _, bail};
use serde::Deserialize;
use smol::{fs, stream::StreamExt as _};

/// The kind of packaged artifact a Gradle package task produces. It selects
/// where under `app/build/outputs/` AGP writes and how the file is located.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// `assemble<Variant>` output: `apk/<variant>/` with an
    /// `output-metadata.json` describing each produced file.
    Apk,
    /// `bundle<Variant>` output: `bundle/<variant>/` holding exactly one
    /// `.aab` and no metadata document.
    Bundle,
}

impl OutputKind {
    /// Subdirectory of `app/build/outputs/` where this artifact kind lands.
    const fn subdirectory(self) -> &'static str {
        match self {
            Self::Apk => "apk",
            Self::Bundle => "bundle",
        }
    }
}

/// Serde model of the `output-metadata.json` document in AGP metadata format
/// version 3, as written next to packaged APKs. Only the fields the CLI reads
/// are modelled.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    artifact_type: ArtifactType,
    variant_name: String,
    elements: Vec<Element>,
}

/// The `artifactType` block of [`Metadata`].
#[derive(Debug, Deserialize)]
struct ArtifactType {
    #[serde(rename = "type")]
    kind: String,
}

/// One entry of the `elements` array of [`Metadata`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Element {
    output_file: String,
}

/// Resolve the single packaged artifact produced by a Gradle package task.
///
/// `variant` is the AGP variant directory name (`debug`, `release`, …) under
/// `app/build/outputs/`.
///
/// # Errors
/// For [`OutputKind::Apk`]: the `output-metadata.json` is missing or
/// unparseable, describes a different artifact type or variant, or lists other
/// than exactly one element — several would mean ABI splits, which this
/// pipeline never requests. For [`OutputKind::Bundle`]: the variant directory
/// is unreadable or holds other than exactly one `.aab`. There is no fallback
/// to a guessed file name.
pub async fn packaged_artifact(
    backend_path: &Path,
    output: OutputKind,
    variant: &str,
) -> eyre::Result<PathBuf> {
    let output_dir = backend_path
        .join("app/build/outputs")
        .join(output.subdirectory())
        .join(variant);
    match output {
        OutputKind::Apk => {
            let metadata_path = output_dir.join("output-metadata.json");
            let text = fs::read_to_string(&metadata_path).await.wrap_err_with(|| {
                format!(
                    "Failed to read AGP output metadata {}",
                    metadata_path.display()
                )
            })?;
            let metadata: Metadata = serde_json::from_str(&text).wrap_err_with(|| {
                format!(
                    "Failed to parse AGP output metadata {}",
                    metadata_path.display()
                )
            })?;

            if metadata.artifact_type.kind != "APK" {
                bail!(
                    "Expected artifact type \"APK\" in {} but found \"{}\"",
                    metadata_path.display(),
                    metadata.artifact_type.kind
                );
            }
            if metadata.variant_name != variant {
                bail!(
                    "Expected variant \"{variant}\" in {} but found \"{}\"",
                    metadata_path.display(),
                    metadata.variant_name
                );
            }

            let [element] = metadata.elements.as_slice() else {
                bail!(
                    "Expected exactly one element in {} but found {}; multiple \
                     elements mean ABI splits, which this pipeline never requests",
                    metadata_path.display(),
                    metadata.elements.len()
                );
            };
            Ok(output_dir.join(&element.output_file))
        }
        OutputKind::Bundle => {
            let mut entries = fs::read_dir(&output_dir).await.wrap_err_with(|| {
                format!(
                    "Failed to list AGP bundle output directory {}",
                    output_dir.display()
                )
            })?;
            let mut bundles = Vec::new();
            while let Some(entry) = entries.next().await {
                let path = entry
                    .wrap_err_with(|| {
                        format!("Failed to read an entry in {}", output_dir.display())
                    })?
                    .path();
                if path.extension().is_some_and(|ext| ext == "aab") {
                    bundles.push(path);
                }
            }
            let [bundle] = bundles.as_slice() else {
                bail!(
                    "Expected exactly one .aab in {} but found {}",
                    output_dir.display(),
                    bundles.len()
                );
            };
            Ok(bundle.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use super::{Metadata, OutputKind, packaged_artifact};

    #[test]
    fn parses_release_output_metadata() {
        let metadata: Metadata = serde_json::from_str(include_str!(
            "../../tests/fixtures/agp-output-metadata-release.json"
        ))
        .expect("release output-metadata.json parses");
        assert_eq!(metadata.artifact_type.kind, "APK");
        assert_eq!(metadata.variant_name, "release");
        let [element] = metadata.elements.as_slice() else {
            panic!("expected exactly one element");
        };
        assert_eq!(element.output_file, "app-release-unsigned.apk");
    }

    #[test]
    fn parses_debug_output_metadata() {
        let metadata: Metadata = serde_json::from_str(include_str!(
            "../../tests/fixtures/agp-output-metadata-debug.json"
        ))
        .expect("debug output-metadata.json parses");
        assert_eq!(metadata.artifact_type.kind, "APK");
        assert_eq!(metadata.variant_name, "debug");
        let [element] = metadata.elements.as_slice() else {
            panic!("expected exactly one element");
        };
        assert_eq!(element.output_file, "app-debug.apk");
    }

    #[test]
    fn resolves_apk_path_from_metadata() {
        let dir = tempdir().expect("temp dir");
        let output_dir = dir.path().join("app/build/outputs/apk/release");
        std::fs::create_dir_all(&output_dir).expect("create output dir");
        std::fs::write(
            output_dir.join("output-metadata.json"),
            include_str!("../../tests/fixtures/agp-output-metadata-release.json"),
        )
        .expect("write metadata");

        let path = smol::block_on(packaged_artifact(dir.path(), OutputKind::Apk, "release"))
            .expect("artifact resolves");
        assert_eq!(path, output_dir.join("app-release-unsigned.apk"));
    }

    #[test]
    fn missing_metadata_is_an_error() {
        let dir = tempdir().expect("temp dir");
        let error = smol::block_on(packaged_artifact(dir.path(), OutputKind::Apk, "release"))
            .expect_err("missing metadata must fail");
        let message = format!("{error:?}");
        assert!(message.contains("output-metadata.json"), "{message}");
    }

    #[test]
    fn zero_elements_is_an_error() {
        let dir = tempdir().expect("temp dir");
        write_metadata(
            dir.path(),
            r#"{"version":3,"artifactType":{"type":"APK"},"variantName":"release","elements":[]}"#,
        );
        let error = smol::block_on(packaged_artifact(dir.path(), OutputKind::Apk, "release"))
            .expect_err("zero elements must fail");
        let message = format!("{error:?}");
        assert!(message.contains("exactly one element"), "{message}");
    }

    #[test]
    fn multiple_elements_is_an_error() {
        let dir = tempdir().expect("temp dir");
        write_metadata(
            dir.path(),
            r#"{"version":3,"artifactType":{"type":"APK"},"variantName":"release","elements":[{"outputFile":"a.apk"},{"outputFile":"b.apk"}]}"#,
        );
        let error = smol::block_on(packaged_artifact(dir.path(), OutputKind::Apk, "release"))
            .expect_err("multiple elements must fail");
        let message = format!("{error:?}");
        assert!(message.contains("exactly one element"), "{message}");
    }

    #[test]
    fn resolves_single_bundle_in_directory() {
        let dir = tempdir().expect("temp dir");
        let output_dir = dir.path().join("app/build/outputs/bundle/release");
        std::fs::create_dir_all(&output_dir).expect("create output dir");
        std::fs::write(output_dir.join("app-release.aab"), b"").expect("write aab");

        let path = smol::block_on(packaged_artifact(dir.path(), OutputKind::Bundle, "release"))
            .expect("artifact resolves");
        assert_eq!(path, output_dir.join("app-release.aab"));
    }

    #[test]
    fn bundle_directory_without_aab_is_an_error() {
        let dir = tempdir().expect("temp dir");
        let output_dir = dir.path().join("app/build/outputs/bundle/release");
        std::fs::create_dir_all(&output_dir).expect("create output dir");

        let error = smol::block_on(packaged_artifact(dir.path(), OutputKind::Bundle, "release"))
            .expect_err("empty bundle directory must fail");
        let message = format!("{error:?}");
        assert!(message.contains("exactly one .aab"), "{message}");
    }

    #[test]
    fn bundle_directory_with_two_aabs_is_an_error() {
        let dir = tempdir().expect("temp dir");
        let output_dir = dir.path().join("app/build/outputs/bundle/release");
        std::fs::create_dir_all(&output_dir).expect("create output dir");
        std::fs::write(output_dir.join("app-release.aab"), b"").expect("write aab");
        std::fs::write(output_dir.join("other.aab"), b"").expect("write aab");

        let error = smol::block_on(packaged_artifact(dir.path(), OutputKind::Bundle, "release"))
            .expect_err("two bundles must fail");
        let message = format!("{error:?}");
        assert!(message.contains("exactly one .aab"), "{message}");
    }

    fn write_metadata(backend_path: &Path, text: &str) {
        let output_dir = backend_path.join("app/build/outputs/apk/release");
        std::fs::create_dir_all(&output_dir).expect("create output dir");
        std::fs::write(output_dir.join("output-metadata.json"), text).expect("write metadata");
    }
}

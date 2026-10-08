//! Embedded-mode Android builds (water-rs/cli#223).
//!
//! On a `[package] embedded = true` project, `water build --platform android`
//! produces the artifact a host application consumes: a `com.android.library`
//! AAR carrying the crate's cdylib for every built ABI, the staged
//! `waterui_assets`, and the runtime's Kotlin glue the host mounts a WaterUI
//! root through (`WaterUiEmbedding` + `WaterUiRootView`). The generated Gradle library project
//! under the managed backend directory assembles the AAR — copied to
//! `target/package/<crate>-<version>-release.aar` inside the project — and
//! publishes it to the local Maven repository as
//! `<bundle_identifier>:<crate_name>:<crate_version>`, so the host's ordinary
//! Gradle build picks up every `water build` rerun without the CLI ever
//! editing the host project.

use std::path::{Path, PathBuf};

use eyre::{Result, bail};
use smol::fs;
use tracing::info;

use crate::{
    android::{
        backend::AndroidBackend,
        platform::{
            AndroidAbi, AndroidPlatform, android_ffi_dependency_features, run_gradle_tasks,
        },
    },
    assets,
    build::{BuildOptions, BuiltTarget},
    project::Project,
};

/// The Gradle library module inside the generated backend project whose AAR
/// the host consumes.
const EMBEDDED_MODULE: &str = "waterui";

/// Every ABI the embedded AAR carries when `--arch` does not narrow the set.
pub const ALL_ABIS: [AndroidAbi; 4] = [
    AndroidAbi::Arm64V8a,
    AndroidAbi::X86_64,
    AndroidAbi::ArmeabiV7a,
    AndroidAbi::X86,
];

/// The artifact `water build` produces for an embedded project, handed back
/// to the terminal for its summary.
#[derive(Debug)]
pub struct EmbeddedArtifact {
    /// `target/package/<crate>-<version>-release.aar` inside the project.
    pub aar_path: PathBuf,
    /// The Maven coordinate the host declares as a dependency:
    /// `group:artifact:version`.
    pub coordinate: String,
}

/// Build the embedded AAR.
///
/// Compiles the crate's cdylib for `abis` into the library module's
/// `jniLibs`, stages assets and fonts into it, then has the generated
/// Gradle project assemble the AAR and publish it to `mavenLocal`.
///
/// # Errors
/// Fails when any ABI's Rust build, the asset staging, or the Gradle
/// assemble/publish step fails, or when the generated backend project has
/// not been scaffolded.
pub async fn build_aar(
    project: &Project,
    options: &BuildOptions,
    abis: &[AndroidAbi],
) -> Result<EmbeddedArtifact> {
    let backend_path = project.backend_path::<AndroidBackend>();
    // The identifier is the Maven `group` the AAR publishes under — a Java
    // package name — so it must satisfy the Android grammar before the build
    // and publish steps run.
    let package_name = project
        .bundle_identifier()
        .android_package_name()
        .map_err(|error| eyre::eyre!("{error}"))?;
    let module_dir = backend_path.join(EMBEDDED_MODULE);
    if !module_dir.join("build.gradle.kts").is_file() {
        bail!(
            "embedded backend project missing at {}; run `water build --platform android` to scaffold it",
            module_dir.display()
        );
    }
    let jni_libs = module_dir.join("src/main/jniLibs");
    // Drop libraries from earlier builds so an ABI no longer built does not
    // linger in the AAR.
    if jni_libs.exists() {
        fs::remove_dir_all(&jni_libs).await?;
    }

    let mut built: Option<BuiltTarget> = None;
    for abi in abis {
        let abi_options = options.clone().with_output_dir(jni_libs.join(abi.as_str()));
        built = Some(
            AndroidPlatform::new(*abi)
                .build(project, abi_options)
                .await?,
        );
    }
    let Some(built) = built else {
        bail!("no Android ABIs selected for the embedded build");
    };

    // Assets and fonts ship inside the AAR exactly as they ship inside an APK.
    stage_embedded_assets(project, &module_dir, &built.app_symbols()?).await?;

    // Kotlin helpers and Maven dependencies likewise belong on the classpath
    // the host app resolves classes from; `api` exports them through the
    // published POM so a consumer's build sees them too. Manifest components
    // go into the library manifest, which the host's manifest merger folds
    // into its own, `${applicationId}` resolving to the host's. The scan
    // mirrors the Rust build's feature selection so helpers behind optional
    // features are not missed.
    crate::assets::stage_android_declarations(
        project,
        &project.ffi_crate_path().join("Cargo.toml"),
        &module_dir,
        crate::assets::AndroidDependencyScope::Api,
        &android_ffi_dependency_features(project).await?,
    )
    .await?;

    run_gradle_tasks(
        project.host(),
        &backend_path,
        &[
            ":waterui:assembleRelease",
            ":waterui:publishReleasePublicationToMavenLocal",
        ],
        &[],
    )
    .await?;

    let version = read_crate_version(project.root()).await?;
    let aar_path = copy_aar_to_package(project, &module_dir, &version).await?;
    let coordinate = format!("{package_name}:{}:{version}", project.crate_name());

    info!(aar = %aar_path.display(), coordinate, "embedded Android artifact built");
    Ok(EmbeddedArtifact {
        aar_path,
        coordinate,
    })
}

/// Stage `waterui_assets` and resolved fonts into the library module — the
/// same content `copy_assets_and_fonts` writes into the app module, minus the
/// app-level `res` files a library does not own.
async fn stage_embedded_assets(
    project: &Project,
    module_dir: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
) -> Result<()> {
    let manifest =
        assets::stage_project_assets_for_android_library(project, module_dir, symbols, false)
            .await?;
    let assets_dir = module_dir.join("src/main/assets");

    let font_declarations =
        assets::scan_fonts(project, &project.ffi_crate_path().join("Cargo.toml")).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);

    if !resolved_fonts.is_empty() {
        let fonts_dest = assets_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
        assets::write_font_manifest(&resolved_fonts, &fonts_dest, None).await?;
        info!("Copied {} fonts to embedded module", resolved_fonts.len());
    }

    Ok(())
}

/// Copy the assembled `waterui-release.aar` into the project's
/// `target/package/` directory named for its Maven coordinate.
async fn copy_aar_to_package(
    project: &Project,
    module_dir: &Path,
    version: &str,
) -> Result<PathBuf> {
    let source = module_dir.join("build/outputs/aar/waterui-release.aar");
    if !source.is_file() {
        bail!(
            "Gradle assembled no AAR at {}; check the embedded build output above",
            source.display()
        );
    }
    let package_dir = project.root().join("target").join("package");
    fs::create_dir_all(&package_dir).await?;
    let dest = package_dir.join(format!("{}-{version}-release.aar", project.crate_name()));
    fs::copy(&source, &dest).await?;
    Ok(dest)
}

/// The `[package] version` of the project crate — the Maven coordinate
/// version the embedded AAR publishes under.
///
/// # Errors
/// Fails when `Cargo.toml` cannot be read or resolves to no `[package]`
/// table — workspace-inherited `version.workspace` resolves through the
/// workspace root.
pub async fn read_crate_version(project_root: &Path) -> Result<String> {
    let cargo_toml = project_root.join("Cargo.toml");
    let manifest_path = cargo_toml.clone();
    let manifest = smol::unblock(move || cargo_toml::Manifest::from_path(&manifest_path))
        .await
        .map_err(|error| {
            eyre::eyre!(
                "cannot read {} for the embedded artifact version: {error}",
                cargo_toml.display()
            )
        })?;
    manifest
        .package
        .as_ref()
        .map(|package| package.version().to_string())
        .ok_or_else(|| {
            eyre::eyre!(
                "embedded projects must declare a [package] version in {}",
                cargo_toml.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_crate_version_reads_literal_version() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            fs::write(
                dir.path().join("Cargo.toml"),
                "[package]\nname = \"demo\"\nversion = \"1.2.3\"\nedition = \"2021\"\n",
            )
            .await
            .expect("write");
            assert_eq!(
                read_crate_version(dir.path()).await.expect("version"),
                "1.2.3"
            );
        });
    }

    #[test]
    fn read_crate_version_resolves_workspace_inheritance() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            fs::write(
                dir.path().join("Cargo.toml"),
                "[workspace]\nmembers = [\"crate\"]\n[workspace.package]\nversion = \"4.5.6\"\n",
            )
            .await
            .expect("workspace");
            let member = dir.path().join("crate");
            fs::create_dir(&member).await.expect("member dir");
            fs::write(
                member.join("Cargo.toml"),
                "[package]\nname = \"demo\"\nversion.workspace = true\nedition = \"2021\"\n",
            )
            .await
            .expect("member");
            assert_eq!(read_crate_version(&member).await.expect("version"), "4.5.6");
        });
    }
}

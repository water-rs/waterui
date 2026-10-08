//! Embedded-mode Android builds (water-rs/waterui#2218).
//!
//! On a `[package] embedded = true` project, `water build --platform android
//! --backend hydrolysis` produces the artifact a host application consumes: a
//! `com.android.library` AAR carrying the Hydrolysis launcher cdylib for every
//! built ABI, the staged `waterui_assets`, and the generated `WaterUi` entry
//! point the host mounts a WaterUI root through. The generated Gradle library
//! project under the managed backend directory assembles the AAR — copied to
//! `target/package/<crate>-<version>-release.aar` inside the project — and
//! publishes it to the local Maven repository as
//! `<bundle_identifier>:<crate_name>:<crate_version>`; the Hydrolysis `host`
//! and painter modules the AAR's POM declares publish to the same repository
//! under `dev.waterui.hydrolysis`, so the host's ordinary Gradle build picks
//! up every `water build` rerun without the CLI ever editing the host
//! project.

use std::path::{Path, PathBuf};

use eyre::{Result, bail};
use smol::fs;
use tracing::info;

use crate::{
    android::{
        backend::manifest_permissions,
        platform::{AndroidAbi, android_ffi_dependency_features, run_gradle_tasks},
    },
    assets::{self, AndroidDependencyScope},
    build::{BuildOptions, BuiltTarget},
    hydrolysis::backend::HydrolysisBackend,
    project::Project,
    templates::{self, HydrolysisAndroidEmbeddedTemplateEntry},
    toolchain::Host,
};

use super::{HydrolysisAndroidPainter, require_painter_module, template_entry};

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
    /// The `dev.waterui.hydrolysis` coordinates this build published the
    /// `host` and painter modules under — the versions the AAR's POM names.
    pub host_coordinates: Vec<String>,
}

/// Build the embedded AAR.
///
/// Compiles the crate's Hydrolysis cdylib for `abis` into the library
/// module's `jniLibs`, stages assets into it, publishes the host checkout's
/// modules under this build's [`host_version`], then has the generated
/// Gradle project assemble the AAR and publish it to `mavenLocal`.
///
/// # Errors
/// Fails when the package name is not a valid Java package, the pinned host
/// checkout cannot be resolved, any ABI's Rust build, the asset staging, or
/// the Gradle assemble/publish step fails, or no ABI is selected.
pub async fn build_aar(
    project: &Project,
    host: &Host,
    painter: HydrolysisAndroidPainter,
    options: &BuildOptions,
    abis: &[AndroidAbi],
) -> Result<EmbeddedArtifact> {
    // The identifier is the Maven `group` the AAR publishes under — a Java
    // package name — so it must satisfy the Android grammar before the build
    // and publish steps run.
    let package_name = project
        .bundle_identifier()
        .android_package_name()
        .map_err(|error| eyre::eyre!("{error}"))?;
    let host_project_dir = require_painter_module(host, project, painter).await?;
    project.scaffold_ffi_companion(false).await?;

    let backend_path = project.backend_path::<HydrolysisBackend>();
    let dir = backend_path.join("android-embedded");
    let version = host_version(&host_project_dir, painter)?;
    let crate_version = read_crate_version(project.root()).await?;
    let ctx = embedded_template_context(
        project,
        painter,
        &host_project_dir,
        &dir,
        &version,
        &crate_version,
    )
    .await?;
    templates::hydrolysis_android_embedded::scaffold(&dir, &ctx).await?;

    // Drop libraries from earlier builds so an ABI no longer built does not
    // linger in the AAR.
    let jni_libs = dir.join("waterui/src/main/jniLibs");
    if jni_libs.exists() {
        fs::remove_dir_all(&jni_libs).await?;
    }

    let mut built: Option<BuiltTarget> = None;
    for abi in abis {
        let abi_options = options.clone().with_output_dir(jni_libs.join(abi.as_str()));
        built = Some(super::build(project, host, *abi, abi_options).await?);
    }
    let Some(built) = built else {
        bail!("no Android ABIs selected for the embedded build");
    };

    // `waterui_assets` ships inside the AAR exactly as it ships inside an
    // APK — `HydrolysisEnvironment.prepare` syncs the same tree at runtime.
    // What the app scaffold stages beyond it (`res`, theme, icons) is the
    // host application's own, never the library's.
    let (manifest, staged) = assets::stage_project_assets_for_android_library(
        project,
        &dir.join("waterui"),
        &built.app_symbols()?,
        false,
    )
    .await?;

    // The library ships the same runtime fonts the app path stages — a
    // dependency's declared font resolves and lands under the module's
    // assets with the table manifest the runtime reads at bootstrap.
    let font_declarations =
        assets::scan_fonts(project, &project.ffi_crate_path().join("Cargo.toml")).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);
    if !resolved_fonts.is_empty() {
        let fonts_dest = staged.root.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
        assets::write_font_manifest(&resolved_fonts, &fonts_dest, None).await?;
    }

    // Kotlin helpers and Maven dependencies likewise belong on the classpath
    // the host app resolves classes from; `api` exports them through the
    // published POM so a consumer's build sees them too. Manifest components
    // go into the library manifest, which the host's manifest merger folds
    // into its own, `${applicationId}` resolving to the host's. The scan
    // mirrors the Rust build's feature selection so helpers behind optional
    // features are not missed.
    assets::stage_android_declarations(
        project,
        &project.ffi_crate_path().join("Cargo.toml"),
        &dir.join("waterui"),
        AndroidDependencyScope::Api,
        &android_ffi_dependency_features(project).await?,
    )
    .await?;

    let painter_publish = format!(
        ":hydrolysis-host:{}:publishReleasePublicationToMavenLocal",
        painter.host_module()
    );
    run_gradle_tasks(
        &dir,
        &[
            ":hydrolysis-host:host:publishReleasePublicationToMavenLocal",
            &painter_publish,
            ":waterui:assembleRelease",
            ":waterui:publishReleasePublicationToMavenLocal",
        ],
        &[("WATERUI_HYDROLYSIS_HOST_VERSION", version.clone())],
    )
    .await?;

    let module_dir = dir.join("waterui");
    let aar_path = copy_aar_to_package(project, &module_dir, &crate_version).await?;
    let coordinate = format!("{package_name}:{}:{crate_version}", project.crate_name());
    let host_coordinates = vec![
        format!("dev.waterui.hydrolysis:host:{version}"),
        format!("{}:{version}", painter.gradle_dependency()),
    ];

    info!(aar = %aar_path.display(), coordinate, "embedded Android artifact built");
    Ok(EmbeddedArtifact {
        aar_path,
        coordinate,
        host_coordinates,
    })
}

/// The template context the generated `android-embedded/` Gradle project
/// renders with: the shared launcher context plus the crate-version and
/// permission entries and the embedded entry carrying the published host
/// version.
async fn embedded_template_context(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    host_project_dir: &Path,
    dir: &Path,
    version: &str,
    crate_version: &str,
) -> Result<crate::templates::TemplateContext> {
    Ok(HydrolysisBackend::template_context(project, &project.resolved_framework().await?)
        .await?
        .with_crate_version(crate_version)
        .with_android_permissions(manifest_permissions(project.manifest()))
        .with_hydrolysis_android_embedded(HydrolysisAndroidEmbeddedTemplateEntry {
            app: template_entry(project, painter, host_project_dir, dir).await?,
            host_version: version.to_owned(),
        }))
}

/// Every file the `android-embedded/` scaffold would write, as
/// android-embedded-dir-relative path and content — the regeneration check's
/// comparison source.
///
/// # Errors
///
/// Returns an error when the template context or rendering fails.
pub async fn rendered_embedded_outputs(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    host_project_dir: &Path,
    version: &str,
    crate_version: &str,
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let dir = project
        .backend_path::<HydrolysisBackend>()
        .join("android-embedded");
    let ctx = embedded_template_context(
        project,
        painter,
        host_project_dir,
        &dir,
        version,
        crate_version,
    )
    .await?;
    templates::hydrolysis_android_embedded::rendered_outputs(&ctx)
        .map_err(|error| eyre::eyre!("{error}"))
}

/// The version this build publishes the Hydrolysis `host` and painter modules
/// under: `0.0.0-<first 16 hex of the sources' sha256>`, so a host that
/// changed never collides with an earlier publish of the same coordinate.
///
/// The hash covers `<host>/settings.gradle.kts` and every file under the
/// `host` and painter modules, sorted by relative path and skipping Gradle
/// `build/` and `.gradle/` outputs.
pub(crate) fn host_version(
    host_project_dir: &Path,
    painter: HydrolysisAndroidPainter,
) -> Result<String> {
    use sha2::{Digest, Sha256};

    let mut files = vec![host_project_dir.join("settings.gradle.kts")];
    collect_host_files(&host_project_dir.join("host"), &mut files)?;
    collect_host_files(&host_project_dir.join(painter.host_module()), &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for file in &files {
        let relative = file
            .strip_prefix(host_project_dir)
            .expect("hashed files live under the host project dir");
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0u8]);
        hasher.update(
            std::fs::read(file).map_err(|error| {
                eyre::eyre!("cannot hash host file {}: {error}", file.display())
            })?,
        );
        hasher.update([0u8]);
    }
    let hex = hex::encode(hasher.finalize());
    Ok(format!("0.0.0-{}", &hex[..16]))
}

/// Appends every regular file under `dir` to `out`, skipping Gradle `build/`
/// and `.gradle/` directories.
fn collect_host_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .map_err(|error| eyre::eyre!("cannot list {}: {error}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name == "build" || name == ".gradle" {
                continue;
            }
            collect_host_files(&path, out)?;
        } else {
            out.push(path);
        }
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

    /// A minimal pinned-host checkout: `settings.gradle.kts` plus `host/` and
    /// `gpu/` module sources, optionally with Gradle `build/` output a hash
    /// must ignore.
    fn stage_fake_host(dir: &Path) -> PathBuf {
        let host = dir.join("hydrolysis-android");
        let write = |path: PathBuf, content: &str| {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdirs");
            std::fs::write(path, content).expect("write");
        };
        write(
            host.join("settings.gradle.kts"),
            "include(\":host\", \":gpu\")",
        );
        write(
            host.join("host/build.gradle.kts"),
            "plugins { id(\"com.android.library\") }",
        );
        write(
            host.join("host/src/main/AndroidManifest.xml"),
            "<manifest />",
        );
        write(
            host.join("gpu/build.gradle.kts"),
            "plugins { id(\"com.android.library\") }",
        );
        host
    }

    #[test]
    fn host_version_is_stable_and_ignores_build_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let host = stage_fake_host(dir.path());
        // A `build/` output directory inside `host/` must not move the hash.
        std::fs::create_dir_all(host.join("host/build/intermediates")).expect("build dir");
        std::fs::write(host.join("host/build/intermediates/x"), "stale").expect("build file");
        std::fs::create_dir_all(host.join(".gradle")).expect("gradle dir");
        std::fs::write(host.join(".gradle/state"), "stale").expect("gradle file");

        let first = host_version(&host, HydrolysisAndroidPainter::Gpu).expect("hash");
        let second = host_version(&host, HydrolysisAndroidPainter::Gpu).expect("hash");
        assert_eq!(first, second);
        assert!(first.starts_with("0.0.0-"), "{first}");
        assert_eq!(first.len(), "0.0.0-".len() + 16, "{first}");
    }

    #[test]
    fn host_version_changes_with_host_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        let host = stage_fake_host(dir.path());
        let before = host_version(&host, HydrolysisAndroidPainter::Gpu).expect("hash");
        std::fs::write(host.join("host/src/main/new.kt"), "// changed").expect("edit");
        let after = host_version(&host, HydrolysisAndroidPainter::Gpu).expect("hash");
        assert_ne!(before, after);
    }

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

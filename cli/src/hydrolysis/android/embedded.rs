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
pub use crate::android::platform::ALL_ABIS;

/// The routing rule for embedded Android builds: Hydrolysis only.
///
/// The Kotlin Android backend has no `com.android.library` output, so
/// selecting it is a configuration error, raised here and in
/// `AndroidBackend::init` before either path scaffolds a project.
pub const EMBEDDED_REQUIRES_HYDROLYSIS: &str =
    "embedded Android libraries are built by Hydrolysis: pass `--backend hydrolysis`";

/// Enforce [`EMBEDDED_REQUIRES_HYDROLYSIS`] for the caller's resolved
/// backend choice.
///
/// # Errors
/// Returns the routing error when `backend_is_hydrolysis` is false.
pub fn require_hydrolysis_backend(backend_is_hydrolysis: bool) -> Result<()> {
    if backend_is_hydrolysis {
        Ok(())
    } else {
        bail!("{EMBEDDED_REQUIRES_HYDROLYSIS}")
    }
}

/// The host checkout's Gradle modules an embedded build substitutes,
/// fingerprints (see [`host_version`]) and publishes — one table so a new
/// host module (e.g. the system-WebView module) joins with a single entry
/// here.
pub(crate) fn publish_modules(painter: HydrolysisAndroidPainter) -> Vec<&'static str> {
    vec!["host", painter.host_module()]
}

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
    /// host modules under — the versions the AAR's POM names.
    pub host_coordinates: Vec<String>,
    /// The validated Android package name (`group` of `coordinate`); the
    /// generated `WaterUi` entry point lives in its `.waterui` subpackage.
    pub android_package_name: String,
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

    let modules = publish_modules(painter);
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let dir = backend_path.join("android-embedded");
    let version = host_version(&host_project_dir, &modules).await?;
    let crate_version = read_crate_version(project.root()).await?;
    let ctx = embedded_template_context(
        project,
        painter,
        &host_project_dir,
        &dir,
        &version,
        &crate_version,
        &modules,
    )
    .await?;
    templates::hydrolysis_android_embedded::scaffold(&dir, &ctx, host).await?;

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

    let mut tasks: Vec<String> = modules
        .iter()
        .map(|module| format!(":hydrolysis-host:{module}:publishReleasePublicationToMavenLocal"))
        .collect();
    tasks.extend([
        ":waterui:assembleRelease".to_owned(),
        ":waterui:publishReleasePublicationToMavenLocal".to_owned(),
    ]);
    run_gradle_tasks(
        &dir,
        &tasks.iter().map(String::as_str).collect::<Vec<_>>(),
        &[("WATERUI_HYDROLYSIS_HOST_VERSION", version.clone())],
        host,
    )
    .await?;

    let module_dir = dir.join("waterui");
    let aar_path = copy_aar_to_package(project, &module_dir, &crate_version).await?;
    let coordinate = format!("{package_name}:{}:{crate_version}", project.crate_name());
    let host_coordinates = modules
        .iter()
        .map(|module| format!("dev.waterui.hydrolysis:{module}:{version}"))
        .collect();

    info!(aar = %aar_path.display(), coordinate, "embedded Android artifact built");
    Ok(EmbeddedArtifact {
        aar_path,
        coordinate,
        host_coordinates,
        android_package_name: package_name.to_string(),
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
    modules: &[&str],
) -> Result<crate::templates::TemplateContext> {
    Ok(
        HydrolysisBackend::template_context(project, &project.resolved_framework().await?)
            .await?
            .with_crate_version(crate_version)
            .with_android_permissions(manifest_permissions(project.manifest()))
            .with_hydrolysis_android_embedded(HydrolysisAndroidEmbeddedTemplateEntry {
                app: template_entry(project, painter, host_project_dir, dir).await?,
                host_version: version.to_owned(),
                host_modules: modules.iter().map(|module| (*module).to_owned()).collect(),
            }),
    )
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
        &publish_modules(painter),
    )
    .await?;
    templates::hydrolysis_android_embedded::rendered_outputs(&ctx)
        .map_err(|error| eyre::eyre!("{error}"))
}

/// The version this build publishes the Hydrolysis host modules under:
/// `0.0.0-<first 16 hex of the sources' sha256>`, so a host that changed
/// never collides with an earlier publish of the same coordinate.
///
/// The hash covers the checkout's `settings.gradle.kts`, `gradle.properties`,
/// the Gradle wrapper files, and every file under `modules`, sorted by
/// relative path and skipping Gradle `build/` and `.gradle/` outputs.
async fn host_version(host_project_dir: &Path, modules: &[&str]) -> Result<String> {
    use sha2::{Digest, Sha256};

    let mut files = vec![host_project_dir.join("settings.gradle.kts")];
    for top_level in ["gradle.properties", "gradle"] {
        let path = host_project_dir.join(top_level);
        if fs::metadata(&path).await.is_ok_and(|meta| meta.is_file()) {
            files.push(path);
        } else if fs::metadata(&path).await.is_ok_and(|meta| meta.is_dir()) {
            // The Gradle wrapper (`gradle/wrapper/`) fingerprints too.
            collect_host_files(&path, &mut files).await?;
        }
    }
    for module in modules {
        collect_host_files(&host_project_dir.join(module), &mut files).await?;
    }
    files.sort();

    let mut hasher = Sha256::new();
    for file in &files {
        let relative = file
            .strip_prefix(host_project_dir)
            .expect("hashed files live under the host project dir");
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0u8]);
        hasher.update(
            fs::read(file).await.map_err(|error| {
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
fn collect_host_files<'a>(
    dir: &'a Path,
    out: &'a mut Vec<PathBuf>,
) -> futures_util::future::BoxFuture<'a, Result<()>> {
    use futures_util::StreamExt as _;

    Box::pin(async move {
        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|error| eyre::eyre!("cannot list {}: {error}", dir.display()))?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type().await?.is_dir() {
                let name = entry.file_name();
                if name == "build" || name == ".gradle" {
                    continue;
                }
                collect_host_files(&path, &mut *out).await?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    })
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

    /// A minimal pinned-host checkout: `settings.gradle.kts`,
    /// `gradle.properties`, the Gradle wrapper, plus `host/` and `gpu/`
    /// module sources, optionally with Gradle `build/` output a hash must
    /// ignore.
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
        write(host.join("gradle.properties"), "org.gradle.jvmargs=-Xmx4g");
        write(
            host.join("gradle/wrapper/gradle-wrapper.properties"),
            "distributionUrl=https\\://services.gradle.org/distributions/gradle-9.3.0-bin.zip",
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

    /// The modules [`publish_modules`] reports for the GPU painter.
    const GPU_MODULES: [&str; 2] = ["host", "gpu"];

    #[test]
    fn host_version_is_stable_and_ignores_build_output() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let host = stage_fake_host(dir.path());
            // A `build/` output directory inside `host/` must not move the
            // hash.
            std::fs::create_dir_all(host.join("host/build/intermediates")).expect("build dir");
            std::fs::write(host.join("host/build/intermediates/x"), "stale").expect("build file");
            std::fs::create_dir_all(host.join(".gradle")).expect("gradle dir");
            std::fs::write(host.join(".gradle/state"), "stale").expect("gradle file");

            let first = host_version(&host, &GPU_MODULES).await.expect("hash");
            let second = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_eq!(first, second);
            assert!(first.starts_with("0.0.0-"), "{first}");
            assert_eq!(first.len(), "0.0.0-".len() + 16, "{first}");
        });
    }

    #[test]
    fn host_version_changes_with_host_sources() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let host = stage_fake_host(dir.path());
            let before = host_version(&host, &GPU_MODULES).await.expect("hash");
            std::fs::write(host.join("host/src/main/new.kt"), "// changed").expect("edit");
            let after = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(before, after);
        });
    }

    #[test]
    fn host_version_covers_gradle_properties_and_wrapper() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let host = stage_fake_host(dir.path());
            let before = host_version(&host, &GPU_MODULES).await.expect("hash");
            std::fs::write(host.join("gradle.properties"), "org.gradle.jvmargs=-Xmx8g")
                .expect("edit");
            let after_properties = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(before, after_properties);

            std::fs::write(
                host.join("gradle/wrapper/gradle-wrapper.properties"),
                "distributionUrl=wrapper-changed",
            )
            .expect("edit");
            let after_wrapper = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(after_properties, after_wrapper);
        });
    }

    #[test]
    fn embedded_routing_accepts_only_hydrolysis() {
        require_hydrolysis_backend(true).expect("hydrolysis routes");
        let error = require_hydrolysis_backend(false).expect_err("kotlin backend rejects");
        assert_eq!(error.to_string(), EMBEDDED_REQUIRES_HYDROLYSIS);
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

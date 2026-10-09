//! Embedded-mode Android builds.
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
use futures_util::{StreamExt as _, TryStreamExt as _};
use serde::{Deserialize, Serialize};
use smol::fs;
use tracing::info;

use crate::{
    android::{
        KotlinToolchain,
        backend::manifest_permissions,
        platform::{
            AndroidAbi, android_ffi_dependency_features, audit_android_permissions,
            run_gradle_tasks,
        },
    },
    assets::{self, AndroidDependencyScope},
    build::BuildOptions,
    hydrolysis::backend::HydrolysisBackend,
    project::Project,
    templates::{self, HydrolysisAndroidEmbeddedTemplateEntry},
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

/// The Maven group every Hydrolysis host module publishes under.
const HOST_GROUP: &str = "dev.waterui.hydrolysis";

/// The environment variable the host modules read their published version
/// from; the host's publishing convention refuses to publish without it.
const HOST_VERSION_ENV: &str = "WATERUI_HYDROLYSIS_HOST_VERSION";

/// The name the generated `settings.gradle.kts` gives the included host
/// checkout build, which addresses its tasks as `:<name>:<module>:<task>`.
const HOST_BUILD_NAME: &str = "hydrolysis-host";

/// The host checkout's Gradle version catalog, relative to its root: the one
/// declaration of the Android toolchain the host modules build with.
const HOST_CATALOG: &str = "gradle/libs.versions.toml";

/// How many host source files [`host_version`] reads at once.
const HOST_HASH_READ_CONCURRENCY: usize = 16;

/// The host checkout's Gradle modules an embedded build substitutes,
/// fingerprints (see [`host_version`]) and publishes — one table so a new
/// host module joins with a single entry here.
pub(crate) fn publish_modules(painter: HydrolysisAndroidPainter) -> Vec<&'static str> {
    vec!["host", painter.host_module()]
}

/// The embedded library's Gradle project, rendered and ready to assemble.
#[derive(Debug, Clone, Serialize)]
pub struct RenderedLibrary {
    /// The generated Gradle root the tasks run in.
    pub dir: PathBuf,
    /// The pinned host checkout's Gradle project the library includes.
    pub host_project_dir: PathBuf,
    /// The `dev.waterui.hydrolysis` version the host modules publish under
    /// for this build; [`Self::gradle_envs`] hands it to Gradle.
    pub host_version: String,
    /// The `dev.waterui.hydrolysis` coordinates the library's POM declares
    /// as `api` dependencies.
    pub host_coordinates: Vec<String>,
    /// The library's Maven group: the bundle identifier as a validated
    /// Android package name. The generated `WaterUi` entry point lives in
    /// its `.waterui` subpackage.
    pub group: String,
    /// The library's Maven artifact id: the crate name.
    pub artifact: String,
    /// The library's Maven version: the crate's `[package] version`.
    pub version: String,
    /// The library's `minSdk`, which a consuming app's `minSdk` must reach.
    pub min_sdk: u32,
    /// The library's `compileSdk` — the host modules' — which a consuming
    /// app's `compileSdk` must reach; the AAR declares it as its
    /// `minCompileSdk`.
    pub compile_sdk: u32,
    /// The Gradle tasks that publish the host modules, then assemble and
    /// publish the library to `mavenLocal`, in order.
    pub gradle_tasks: Vec<String>,
}

impl RenderedLibrary {
    /// The library's Maven coordinate: `group:artifact:version`.
    #[must_use]
    pub fn coordinate(&self) -> String {
        format!("{}:{}:{}", self.group, self.artifact, self.version)
    }

    /// The environment the [`gradle_tasks`](Self::gradle_tasks) run with.
    #[must_use]
    pub fn gradle_envs(&self) -> [(&'static str, String); 1] {
        [(HOST_VERSION_ENV, self.host_version.clone())]
    }
}

/// Render the embedded library's Gradle project into `dir`.
///
/// Resolves the pinned host checkout and its `painter` module, fingerprints
/// the host sources into the version its modules publish under, and writes
/// the composite: the `:waterui` library module, the generated `WaterUi`
/// entry point, and the included host build substituting the coordinates
/// the module declares. Nothing is compiled; [`build_aar`] stages the
/// native libraries and assets into the result before assembling it.
///
/// # Errors
/// Fails when the bundle identifier is not a Java package name, the host
/// checkout or its painter module cannot be resolved, the crate version
/// cannot be read from Cargo metadata, or the scaffold cannot be written.
pub async fn render_library(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    dir: &Path,
) -> Result<RenderedLibrary> {
    // The identifier is the Maven `group` the AAR publishes under — a Java
    // package name — so it must satisfy the Android grammar before any file
    // lands.
    let package_name = project
        .bundle_identifier()
        .android_package_name()
        .map_err(|error| eyre::eyre!("{error}"))?;
    let host_project_dir = require_painter_module(project, painter).await?;
    let modules = publish_modules(painter);
    let (host_version, crate_version, compile_sdk) = futures_util::try_join!(
        host_version(&host_project_dir, &modules),
        project.crate_version(),
        host_compile_sdk(&host_project_dir),
    )?;
    let ctx = embedded_template_context(
        project,
        painter,
        &host_project_dir,
        dir,
        EmbeddedHost {
            version: &host_version,
            modules: &modules,
            compile_sdk,
        },
        &crate_version,
    )
    .await?;
    templates::hydrolysis_android_embedded::scaffold(project.host(), dir, &ctx).await?;

    let mut gradle_tasks: Vec<String> = modules
        .iter()
        .map(|module| format!(":{HOST_BUILD_NAME}:{module}:publishReleasePublicationToMavenLocal"))
        .collect();
    gradle_tasks.extend([
        ":waterui:assembleRelease".to_owned(),
        ":waterui:publishReleasePublicationToMavenLocal".to_owned(),
    ]);
    Ok(RenderedLibrary {
        dir: dir.to_path_buf(),
        host_coordinates: modules
            .iter()
            .map(|module| format!("{HOST_GROUP}:{module}:{host_version}"))
            .collect(),
        group: package_name.to_string(),
        artifact: project.crate_name().to_string(),
        version: crate_version,
        min_sdk: ctx.hydrolysis_android_embedded().app.min_api_level,
        compile_sdk,
        host_project_dir,
        host_version,
        gradle_tasks,
    })
}

/// The artifact `water build` produces for an embedded project, handed back
/// to the terminal for its summary.
#[derive(Debug)]
pub struct EmbeddedArtifact {
    /// `target/package/<crate>-<version>-release.aar` inside the project.
    pub aar_path: PathBuf,
    /// The rendered library the AAR was assembled from: its coordinate,
    /// the host coordinates its POM names, and its consumer constraints.
    pub library: RenderedLibrary,
}

/// Build the embedded AAR.
///
/// Runs the ABI-independent preparation once — the FFI companion render, the
/// permission audit, the launcher's font metadata and the library scaffold —
/// then compiles the crate's Hydrolysis cdylib for each of `abis` into the
/// library module's `jniLibs`, stages assets into it, and has the generated
/// Gradle project publish the host modules under this build's host version,
/// assemble the AAR and publish it to `mavenLocal`.
///
/// # Errors
/// Fails when no ABI is selected, the library cannot be rendered, any ABI's
/// Rust build, the asset staging, or the Gradle assemble/publish step fails.
///
/// `kotlin` is the toolchain the caller's toolchain check resolved — every
/// ABI's build reuses it rather than probing `kotlinc` again.
pub async fn build_aar(
    project: &Project,
    painter: HydrolysisAndroidPainter,
    options: &BuildOptions,
    abis: &[AndroidAbi],
    kotlin: &KotlinToolchain,
) -> Result<EmbeddedArtifact> {
    let Some((last_abi, earlier_abis)) = abis.split_last() else {
        bail!("no Android ABIs selected for the embedded build");
    };
    let dir = project
        .backend_path::<HydrolysisBackend>()
        .join("android-embedded");
    let ((), (), library) = futures_util::try_join!(
        async {
            // The companion is the graph the classpath staging and the
            // feature answers below read, so it renders before its audit.
            project.scaffold_ffi_companion().await?;
            audit_android_permissions(project).await
        },
        super::prepare_for_build(project),
        render_library(project, painter, &dir),
    )?;

    // Drop libraries from earlier builds so an ABI no longer built does not
    // linger in the AAR.
    let module_dir = dir.join("waterui");
    let jni_libs = module_dir.join("src/main/jniLibs");
    super::remove_dir_if_present(&jni_libs).await?;
    let build_abi = |abi: AndroidAbi| {
        let abi_options = options.clone().with_output_dir(jni_libs.join(abi.as_str()));
        super::build_prepared(project, abi, abi_options, &[], kotlin)
    };
    for abi in earlier_abis {
        build_abi(*abi).await?;
    }
    // Asset staging reads the app symbols of one built library; every ABI
    // compiles the same crate, so the last one serves.
    let built = build_abi(*last_abi).await?.built;

    // `waterui_assets` ships inside the AAR exactly as it ships inside an
    // APK — `HydrolysisEnvironment.prepare` syncs the same tree at runtime.
    // What the app scaffold stages beyond it (`res`, theme, icons) is the
    // host application's own, never the library's.
    let (manifest, staged) = assets::stage_project_assets_for_android_library(
        project,
        &module_dir,
        &built.app_symbols()?,
        false,
    )
    .await?;

    // The library ships the same runtime fonts the app path stages — a
    // dependency's declared font resolves and lands under the module's
    // assets with the table manifest the runtime reads at bootstrap.
    let ffi_manifest = project.ffi_crate_path().join("Cargo.toml");
    let font_declarations = assets::scan_fonts(project, &ffi_manifest).await?;
    let mut resolved_fonts = assets::resolve_fonts(project.host(), font_declarations).await?;
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
    // into its own, `${applicationId}` resolving to the host's. The feature
    // set is the one every built ABI's own graph answers, so helpers behind
    // optional features are not missed.
    let triples: Vec<_> = abis.iter().map(|abi| abi.triple()).collect();
    assets::stage_android_declarations(
        project,
        &ffi_manifest,
        &module_dir,
        AndroidDependencyScope::Api,
        &android_ffi_dependency_features(project, &triples).await?,
    )
    .await?;

    {
        // The library `includeBuild`s the host checkout, whose `build/` and
        // `.gradle/` every other build of that checkout writes too.
        let _host_build =
            crate::water_dir::android_host_build_lock(project.host(), &library.host_project_dir)
                .await?;
        run_gradle_tasks(
            project.host(),
            &dir,
            &library
                .gradle_tasks
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &library.gradle_envs(),
        )
        .await?;
    }

    let aar_path = copy_aar_to_package(project, &module_dir, &library).await?;
    info!(aar = %aar_path.display(), coordinate = library.coordinate(), "embedded Android artifact built");
    Ok(EmbeddedArtifact { aar_path, library })
}

/// What the embedded library takes from the host checkout it builds over.
#[derive(Debug, Clone, Copy)]
struct EmbeddedHost<'a> {
    /// The version the host modules publish under.
    version: &'a str,
    /// The host modules the library substitutes and publishes.
    modules: &'a [&'a str],
    /// The host modules' `compileSdk`.
    compile_sdk: u32,
}

/// The part of the host's version catalog the CLI reads.
#[derive(Deserialize)]
struct HostCatalog {
    versions: HostCatalogVersions,
}

#[derive(Deserialize)]
struct HostCatalogVersions {
    /// The `compileSdk` every host module builds against.
    #[serde(rename = "android-compile-sdk")]
    compile_sdk: String,
}

/// The `compileSdk` the host checkout's modules build against, as its
/// version catalog ([`HOST_CATALOG`]) declares it.
async fn host_compile_sdk(host_project_dir: &Path) -> Result<u32> {
    let path = host_project_dir.join(HOST_CATALOG);
    let catalog = fs::read_to_string(&path)
        .await
        .map_err(|error| eyre::eyre!("cannot read {}: {error}", path.display()))?;
    let catalog: HostCatalog = toml::from_str(&catalog).map_err(|error| {
        eyre::eyre!(
            "{} must declare versions.android-compile-sdk: {error}",
            path.display()
        )
    })?;
    let value = catalog.versions.compile_sdk;
    value.parse().map_err(|error| {
        eyre::eyre!(
            "{} declares android-compile-sdk = \"{value}\", not an API level: {error}",
            path.display()
        )
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
    host: EmbeddedHost<'_>,
    crate_version: &str,
) -> Result<crate::templates::TemplateContext> {
    Ok(
        HydrolysisBackend::template_context(project, project.resolved_framework().await?)
            .await?
            .with_crate_version(crate_version)
            .with_android_permissions(manifest_permissions(project.manifest()))
            .with_hydrolysis_android_embedded(HydrolysisAndroidEmbeddedTemplateEntry {
                app: template_entry(project, painter, host_project_dir, dir).await?,
                host_version: host.version.to_owned(),
                host_modules: host
                    .modules
                    .iter()
                    .map(|module| (*module).to_owned())
                    .collect(),
                compile_sdk: host.compile_sdk,
            }),
    )
}

/// The host `compileSdk` [`rendered_embedded_outputs`] renders with.
#[cfg(test)]
pub(super) const TEST_COMPILE_SDK: u32 = 36;

/// Every file the `android-embedded/` scaffold would write, as
/// android-embedded-dir-relative path and content, for scaffold tests.
///
/// # Errors
///
/// Returns an error when the template context or rendering fails.
#[cfg(test)]
pub(super) async fn rendered_embedded_outputs(
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
        EmbeddedHost {
            version,
            modules: &publish_modules(painter),
            compile_sdk: TEST_COMPILE_SDK,
        },
        crate_version,
    )
    .await?;
    templates::hydrolysis_android_embedded::rendered_outputs(&ctx)
        .map_err(|error| eyre::eyre!("{error}"))
}

/// The version this build publishes the Hydrolysis host modules under:
/// `0.0.0-<first 16 hex of the sources' sha256>`, so a host that changed
/// never collides with an earlier publish of the same coordinate.
///
/// The hash covers exactly what Gradle reads to build the modules: the
/// checkout's root `settings.gradle.kts`, `build.gradle.kts`,
/// `gradle.properties` and `gradle/` directory, and per module its
/// `build.gradle.kts`, its `*.pro` rule files and its `src/` tree — sorted
/// by relative path, so editor or OS droppings beside them never move it.
async fn host_version(host_project_dir: &Path, modules: &[&str]) -> Result<String> {
    use sha2::{Digest, Sha256};

    let mut files = Vec::new();
    for root_file in [
        "settings.gradle.kts",
        "build.gradle.kts",
        "gradle.properties",
    ] {
        push_if_file(host_project_dir.join(root_file), &mut files).await?;
    }
    collect_tree(&host_project_dir.join("gradle"), &mut files).await?;
    for module in modules {
        let module_dir = host_project_dir.join(module);
        let mut entries = fs::read_dir(&module_dir)
            .await
            .map_err(|error| eyre::eyre!("cannot list {}: {error}", module_dir.display()))?;
        while let Some(entry) = entries.try_next().await? {
            let path = entry.path();
            let is_gradle_input = path
                .file_name()
                .is_some_and(|name| name == "build.gradle.kts")
                || path.extension().is_some_and(|extension| extension == "pro");
            if is_gradle_input && entry.file_type().await?.is_file() {
                files.push(path);
            }
        }
        collect_tree(&module_dir.join("src"), &mut files).await?;
    }
    files.sort();

    // Reads overlap, bounded; the hash consumes them in path order.
    let hasher = futures_util::stream::iter(&files)
        .map(|file| async move {
            fs::read(file)
                .await
                .map(|content| (file, content))
                .map_err(|error| eyre::eyre!("cannot hash host file {}: {error}", file.display()))
        })
        .buffered(HOST_HASH_READ_CONCURRENCY)
        .try_fold(Sha256::new(), |mut hasher, (file, content)| async move {
            let relative = file
                .strip_prefix(host_project_dir)
                .expect("hashed files live under the host project dir");
            hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
            hasher.update([0u8]);
            hasher.update(content);
            hasher.update([0u8]);
            Ok(hasher)
        })
        .await?;
    let hex = hex::encode(hasher.finalize());
    Ok(format!("0.0.0-{}", &hex[..16]))
}

/// Appends `path` to `out` when it names a regular file.
async fn push_if_file(path: PathBuf, out: &mut Vec<PathBuf>) -> Result<()> {
    match fs::metadata(&path).await {
        Ok(meta) if meta.is_file() => out.push(path),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => bail!("cannot stat {}: {error}", path.display()),
    }
    Ok(())
}

/// Appends every regular file under `dir` to `out`; a missing `dir`
/// contributes nothing.
fn collect_tree<'a>(
    dir: &'a Path,
    out: &'a mut Vec<PathBuf>,
) -> futures_util::future::BoxFuture<'a, Result<()>> {
    Box::pin(async move {
        let mut entries = match fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => bail!("cannot list {}: {error}", dir.display()),
        };
        while let Some(entry) = entries.try_next().await? {
            let path = entry.path();
            if entry.file_type().await?.is_dir() {
                collect_tree(&path, &mut *out).await?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    })
}

/// Copy the assembled `waterui-release.aar` into the project's
/// `target/package/` directory, named `<artifact>-<version>-release.aar`
/// after the library's coordinate.
async fn copy_aar_to_package(
    project: &Project,
    module_dir: &Path,
    library: &RenderedLibrary,
) -> Result<PathBuf> {
    let source = module_dir.join("build/outputs/aar/waterui-release.aar");
    let package_dir = project.root().join("target").join("package");
    fs::create_dir_all(&package_dir).await?;
    let dest = package_dir.join(format!(
        "{}-{}-release.aar",
        library.artifact, library.version
    ));
    fs::copy(&source, &dest).await.map_err(|error| {
        eyre::eyre!(
            "cannot copy the assembled AAR {} to {}: {error}",
            source.display(),
            dest.display()
        )
    })?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal pinned-host checkout: `settings.gradle.kts`,
    /// `gradle.properties`, the version catalog, the Gradle wrapper, plus
    /// `host/` and `gpu/` module sources.
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
            host.join("gradle.properties"),
            "org.gradle.jvmargs=-Xmx4g\n",
        );
        write(
            host.join(HOST_CATALOG),
            "[versions]\nandroid-gradle-plugin = \"9.3.0\"\nandroid-compile-sdk = \"36\"\n",
        );
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
    fn host_version_is_stable_and_ignores_what_gradle_does_not_read() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let host = stage_fake_host(dir.path());
            let first = host_version(&host, &GPU_MODULES).await.expect("hash");

            // Build output, IDE and OS droppings, and machine-local
            // properties beside the inputs must not move the hash.
            for (path, content) in [
                ("host/build/intermediates/x", "stale"),
                (".gradle/state", "stale"),
                ("host/.cxx/cmake", "stale"),
                (".DS_Store", "finder"),
                ("host/.DS_Store", "finder"),
                ("host/host.iml", "<module />"),
                ("host/local.properties", "sdk.dir=/somewhere"),
            ] {
                let path = host.join(path);
                std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdirs");
                std::fs::write(path, content).expect("write");
            }

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
            let after_source = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(before, after_source);
            std::fs::write(host.join("gpu/consumer-rules.pro"), "-keep class x").expect("edit");
            let after_rules = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(after_source, after_rules);
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

            std::fs::write(host.join("build.gradle.kts"), "plugins { base }").expect("edit");
            let after_root_script = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(after_wrapper, after_root_script);
            std::fs::write(host.join("build.gradle.kts"), "plugins { java }").expect("edit");
            let after_root_edit = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(after_root_script, after_root_edit);
        });
    }

    #[test]
    fn host_compile_sdk_reads_the_host_catalog() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let host = stage_fake_host(dir.path());
            assert_eq!(host_compile_sdk(&host).await.expect("compileSdk"), 36);

            let before = host_version(&host, &GPU_MODULES).await.expect("hash");
            std::fs::write(
                host.join(HOST_CATALOG),
                "[versions]\nandroid-gradle-plugin = \"9.3.0\"\n",
            )
            .expect("edit");
            let after = host_version(&host, &GPU_MODULES).await.expect("hash");
            assert_ne!(before, after, "the catalog is a host input");
            let error = host_compile_sdk(&host)
                .await
                .expect_err("a catalog without the compileSdk is an error");
            assert!(error.to_string().contains("android-compile-sdk"), "{error}");
        });
    }
}

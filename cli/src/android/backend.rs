use std::path::{Path, PathBuf};

use crate::{
    android::platform::{AndroidAbi, AndroidPlatform, clean_android, is_android_platform},
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::Project,
    templates::{self, TemplateContext},
};

/// The generated Android backend in a `WaterUI` project.
///
/// Runtime state only — nothing is persisted in `Water.toml`. The project
/// path locates the Gradle project the CLI generates in the managed build
/// cache; the Kotlin runtime is consumed at the revision
/// `android-backend-revision` pins, never from a local checkout.
#[derive(Debug, Clone)]
pub struct AndroidBackend {
    project_path: PathBuf,
}

impl AndroidBackend {
    /// Create a new Android backend with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            project_path: default_android_project_path(),
        }
    }

    /// Get the path to the Android project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }

    /// Get the path to the Gradle wrapper script within the Android project.
    #[must_use]
    pub fn gradlew_path(&self) -> PathBuf {
        let base = &self.project_path;
        if cfg!(windows) {
            base.join("gradlew.bat")
        } else {
            base.join("gradlew")
        }
    }
}

impl Default for AndroidBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for AndroidBackend {
    const DEFAULT_PATH: &'static str = "android";

    // Preserve Gradle build caches during re-scaffolding: `app` is the
    // entry-owning module, `waterui` the embedded-mode library module.
    const CACHE_PATHS: &'static [&'static str] = &[".gradle", "build", "app", "waterui"];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let manifest = project.manifest();

        // The identifier is rendered into the scaffold below as the app's
        // Java package name (`applicationId`, `namespace`, Maven `group`) —
        // check it against the Android grammar before any file lands, rather
        // than letting Gradle reject it mid-build.
        let _ = project
            .bundle_identifier()
            .android_package_name()
            .map_err(|error| crate::backend::FailToInitBackend::Config(eyre::eyre!("{error}")))?;

        // Derive app name from the display name (remove spaces for filesystem)
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();

        // Android is where a missing declaration actually breaks things, so
        // surface anything a dependency needs that the app has not enabled.
        // The audit resolves the FFI companion's graph — the crate the Android
        // build compiles. `Project::open` re-renders the companion for this
        // invocation's selection before any backend runs; only a companion
        // carried over from a prior open is audited here — the fresh render's
        // graph is resolved by the build that follows anyway.
        let ffi_manifest = project.ffi_crate_path().join("Cargo.toml");
        if project.ffi_companion_preexisting && ffi_manifest.exists() {
            crate::assets::seed_managed_crate_lock(project, &ffi_manifest)
                .await
                .map_err(crate::backend::FailToInitBackend::Config)?;
            let required = crate::assets::scan_required_permissions(&ffi_manifest)
                .await
                .map_err(crate::backend::FailToInitBackend::Config)?;
            crate::assets::warn_missing_permissions(project, &required, |key| {
                key.android_permission_name().is_some()
            });
        }

        // Extract enabled permissions from the manifest
        let android_permissions = manifest_permissions(manifest);

        let ctx = TemplateContext::for_project_manifest(
            manifest,
            project.crate_name().clone(),
            app_name,
            &project
                .resolved_framework()
                .await
                .map_err(crate::backend::FailToInitBackend::Config)?,
            project.local_sources(),
        )
        .with_backend_project_path(project.backend_path::<Self>())
        .with_project_root_path(project.root().to_path_buf())
        .with_android_permissions(android_permissions);

        if manifest.package.embedded {
            let ctx = ctx.with_crate_version(
                crate::android::embedded::read_crate_version(project.root())
                    .await
                    .map_err(crate::backend::FailToInitBackend::Config)?,
            );
            templates::android_embedded::scaffold(&project.backend_path::<Self>(), &ctx)
                .await
                .map_err(crate::backend::FailToInitBackend::Io)?;
        } else {
            templates::android::scaffold(&project.backend_path::<Self>(), &ctx)
                .await
                .map_err(crate::backend::FailToInitBackend::Io)?;
        }

        Ok(Self {
            project_path: default_android_project_path(),
        })
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_android_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<crate::build::BuiltTarget> {
        debug_assert_eq!(platform, TargetPlatform::Android);
        project
            .browser_runtime_plan(platform, TargetBackend::Android)
            .await?;
        AndroidPlatform::arm64().build(project, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
        built: &crate::build::BuiltTarget,
    ) -> eyre::Result<Artifact> {
        debug_assert_eq!(platform, TargetPlatform::Android);
        let prepared = crate::android::signing::PreparedSigning::resolve(project, &options)?;
        AndroidPlatform::package_with_abis(
            project,
            options,
            &[AndroidAbi::Arm64V8a],
            built,
            &prepared,
        )
        .await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_android(project).await
    }
}

fn default_android_project_path() -> PathBuf {
    PathBuf::from("android")
}

/// The `<uses-permission>` entries the project manifest enables, for any
/// backend that scaffolds an `AndroidManifest.xml` — the View-based backend
/// and the Hydrolysis host alike.
pub(crate) fn manifest_permissions(
    manifest: &crate::project::Manifest,
) -> Vec<templates::AndroidPermissionTemplateEntry> {
    manifest
        .permissions
        .iter()
        .filter(|(_, entry)| entry.is_enabled())
        .filter_map(|(key, _)| {
            key.android_permission_name()
                .map(|name| templates::AndroidPermissionTemplateEntry { name })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::AndroidBackend;
    use crate::{
        backend::Backend,
        project::{CreateOptions, Project},
        project_types::BundleIdentifier,
    };

    /// A hyphenated identifier — valid as `CFBundleIdentifier` but not a Java
    /// package name — fails the scaffold with an Android-named error before a
    /// single Gradle file is written.
    #[test]
    fn init_rejects_an_android_invalid_bundle_identifier() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("liquid-glass");
        let project = smol::block_on(Project::create(
            &root,
            CreateOptions {
                name: "Liquid Glass".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.liquid-glass")
                    .expect("the shared identifier grammar accepts hyphens"),
                waterui_path: None,
                channel: None,
                framework_manifest: None,
                framework: Some(crate::framework::test_fixtures::stable_framework()),
                framework_lock: None,
                author: "Lexo Liu".to_string(),
                web: None,
            },
        ))
        .expect("project creation must succeed");

        let error = smol::block_on(AndroidBackend::init(&project))
            .expect_err("a hyphenated identifier is not an Android package name");
        assert!(
            format!("{error}").contains("Android"),
            "the rejection names the platform: {error}"
        );
        assert!(
            !project
                .backend_path::<AndroidBackend>()
                .join("app")
                .exists(),
            "no Gradle module was scaffolded"
        );
    }
}

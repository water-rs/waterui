use std::path::{Path, PathBuf};

use eyre::WrapErr as _;

use crate::{
    apple::platform::{build_rust_lib, clean_apple, is_apple_platform, package_apple},
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::Project,
    project_types::CrateName,
    templates::{self, TemplateContext},
};

/// The generated Apple backend in a `WaterUI` project.
///
/// Runtime state only — nothing is persisted in `Water.toml`. The project
/// path and scheme describe the scaffold the CLI generates in the managed
/// build cache; a local runtime checkout is discovered at
/// `waterui_path/backends/apple`, never declared.
#[derive(Debug, Clone)]
pub struct AppleBackend {
    /// Path to the generated Apple project below the managed backends root.
    project_path: PathBuf,
    /// The scheme to use for building the Apple project.
    scheme: String,
}

/// What this project's built application bundle is called.
///
/// Deliberately not the scheme. The scheme is a fixed handle the CLI
/// addresses the generated scaffold with — every project shares one — while
/// the product name is the one a person reads. macOS takes `CFBundleName`,
/// and with it the menu bar, the Dock and
/// Force Quit, from `PRODUCT_NAME`, so a project that leaves the two equal
/// announces itself as the scaffold's target rather than as itself.
///
/// The scaffold writes this same name into `PRODUCT_NAME`, so this is also
/// where the built bundle is found afterwards; the two must agree.
///
/// # Errors
///
/// Returns an error when the name cannot be a bundle's: empty, or containing a
/// path separator that would place the bundle somewhere else entirely.
pub fn apple_product_name(project: &Project) -> Result<&str, eyre::Report> {
    let name = project.manifest().package().name();
    if name.is_empty() {
        eyre::bail!("This project has no name; `package.name` in Water.toml names the application");
    }
    if name.contains(std::path::MAIN_SEPARATOR) || name.contains('/') {
        eyre::bail!(
            "The project name {name:?} contains a path separator, so it cannot name an application bundle"
        );
    }
    Ok(name)
}

impl AppleBackend {
    /// Create a new Apple backend configuration with the given scheme.
    #[must_use]
    pub fn new(scheme: impl Into<String>) -> Self {
        Self {
            project_path: default_apple_project_path(),
            scheme: scheme.into(),
        }
    }

    /// Get the path to the Apple project within the `WaterUI` project.
    #[must_use]
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }

    /// The scheme to use for building the Apple project.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
}

fn default_apple_project_path() -> PathBuf {
    PathBuf::from("apple")
}

impl AppleBackend {
    /// The `(scheme, app name, crate name)` the scaffold renders with: every
    /// generated Apple project is the shared `WaterUIApp` host.
    fn scaffold_names() -> (String, String, CrateName) {
        (
            "WaterUIApp".to_string(),
            "WaterUIApp".to_string(),
            CrateName::try_from("WaterUIApp").expect("the Apple host crate name must be valid"),
        )
    }

    /// The template context [`init`] scaffolds with, rebuilt from the current
    /// manifest — what [`requires_regeneration`] diffs the generated project
    /// against.
    ///
    /// # Errors
    ///
    /// Returns an error when the application dependency graph or the
    /// framework cannot be resolved, or when `bundle_identifier` is not a
    /// valid Apple `CFBundleIdentifier`.
    pub(crate) async fn template_context(project: &Project) -> eyre::Result<TemplateContext> {
        let manifest = project.manifest();
        // The manifest's identifier becomes `CFBundleIdentifier` when the app
        // is packaged — check it against Apple's grammar now, at scaffold
        // time, rather than letting an invalid value surface inside codesign
        // or provisioning. An embedded project's identifier is the host
        // app's concern; it never reaches a `CFBundleIdentifier` here.
        if !manifest.package().embedded() {
            let _ = project
                .bundle_identifier()
                .apple_bundle_identifier()
                .map_err(|error| eyre::eyre!("{error}"))?;
        }
        let (_, app_name, crate_name_for_template) = Self::scaffold_names();
        let ios_permissions = manifest
            .permissions()
            .iter()
            .filter(|(_, entry)| entry.is_enabled())
            .filter_map(|(key, entry)| {
                key.ios_plist_key()
                    .map(|plist_key| templates::IosPermissionTemplateEntry {
                        plist_key,
                        description: entry.description().to_string(),
                    })
            })
            .collect();
        // The generated project names the launch assets the catalog will
        // hold, so the two are decided from the same resolution.
        let launch_entry = crate::apple::app_bundle::launch_template_entry(project)?;
        let framework = project.resolved_framework().await?;
        // The generated crates name the in-tree `waterui-apple` member — a
        // framework whose manifest declares no `apple-backend-path` ships the
        // backend as the external package it was before the move, and cannot
        // host this layout. Judge the manifest here, before a single scaffold
        // file is written, rather than inside `member_dependency` mid-render.
        framework.require_in_tree_member(crate::framework::APPLE_BACKEND)?;
        Ok(TemplateContext::for_project_manifest(
            project.host(),
            manifest,
            crate_name_for_template,
            app_name,
            framework,
            project.local_sources(),
        )
        .with_backend_project_path(project.backend_path::<Self>())
        .with_project_root_path(project.root().to_path_buf())
        .with_ios_permissions(ios_permissions)
        .with_launch(launch_entry))
    }

    /// Whether the generated Apple project differs from what the current
    /// templates and manifest would render — a `waterui_path` checkout,
    /// `branch` or `revision` change rewrites the backend
    /// dependency the companion crate's manifest pins.
    ///
    /// # Errors
    ///
    /// Returns an error when the template context or outputs cannot resolve.
    pub async fn requires_regeneration(project: &Project) -> eyre::Result<bool> {
        let backend_dir = project.backend_path::<Self>();
        let ctx = Self::template_context(project).await?;
        for (relative, expected) in templates::apple::rendered_outputs(&ctx)? {
            let path = backend_dir.join(&relative);
            match std::fs::read(&path) {
                Ok(existing) if existing == expected => {}
                Ok(_) => return Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(true);
                }
                Err(error) => {
                    return Err(error)
                        .wrap_err_with(|| format!("Failed to read {}", path.display()));
                }
            }
        }
        Ok(false)
    }
}

impl Backend for AppleBackend {
    const DEFAULT_PATH: &'static str = "apple";

    // Preserve build caches during re-scaffolding.
    const CACHE_PATHS: &'static [&'static str] = &["DerivedData"];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let (scheme, _, _) = Self::scaffold_names();
        let project_path = default_apple_project_path();

        let ctx = Self::template_context(project)
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;

        templates::apple::scaffold(&project.backend_path::<Self>(), &ctx)
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self {
            project_path,
            scheme,
        })
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_apple_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<crate::build::BuiltTarget> {
        project
            .browser_runtime_plan(
                platform,
                TargetBackend::Apple,
                &crate::apple::platform::apple_build_triple(platform, &options),
            )
            .await?;
        build_rust_lib(project, platform, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
        built: &crate::build::BuiltTarget,
    ) -> eyre::Result<Artifact> {
        package_apple(project, platform, options, built).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_apple(project).await
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::AppleBackend;
    use crate::{
        backend::reinit_backend,
        platform::TargetBackend,
        project::{CreateOptions, ManagedBackends, Project},
        project_types::BundleIdentifier,
        toolchain::Host,
    };

    /// Stage a local framework checkout workspace and create a project pinned
    /// to it so tests create and open offline without resolving crates.io.
    fn create_local_project(
        host: &Host,
        dir: &Path,
        project_dir: &str,
        name: &str,
        bundle_id: &str,
    ) -> Project {
        let checkout = dir.join("waterui");
        crate::framework::test_fixtures::write_local_checkout_workspace(&checkout)
            .expect("local checkout");
        let root = dir.join(project_dir);
        smol::block_on(Project::create(
            host,
            &root,
            CreateOptions::new(
                name.to_string(),
                BundleIdentifier::try_from(bundle_id).expect("bundle identifier"),
                "Lexo Liu".to_string(),
            )
            .with_waterui_path(Some(checkout)),
        ))
        .expect("project creation must succeed")
    }

    /// The scaffold emits only the entitlements file — no Xcode project
    /// exists to regenerate — yet the staleness check still detects an edit
    /// and re-renders without losing the packaging cache.
    #[test]
    fn scaffold_without_xcode_project_still_detects_staleness() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host = crate::toolchain::testing::real_toolchain_host(dir.path());
        let project = create_local_project(
            &host,
            dir.path(),
            "water-example",
            "Water Example",
            "dev.waterui.waterexample",
        );

        let project = smol::block_on(Project::open(
            &host,
            project.root(),
            ManagedBackends::for_backend(TargetBackend::Apple),
        ))
        .expect("opening the project scaffolds the Apple backend");

        let backend_dir = project.backend_path::<AppleBackend>();
        assert!(
            !backend_dir
                .join("WaterUIApp.xcodeproj/project.pbxproj")
                .exists(),
            "the entry-owning scaffold produces no Xcode project"
        );
        let entitlements = backend_dir
            .join("WaterUIApp")
            .join("WaterUIApp.entitlements");
        assert!(entitlements.exists(), "the entitlements scaffolded");
        assert!(
            !smol::block_on(AppleBackend::requires_regeneration(&project))
                .expect("staleness check"),
            "a freshly scaffolded backend is not stale"
        );

        // The packaging cache must survive the re-render like it does for
        // the other generated backends.
        let derived_data = backend_dir.join("DerivedData/stale.txt");
        fs::create_dir_all(derived_data.parent().expect("parent")).expect("DerivedData");
        fs::write(&derived_data, "cache").expect("cache file");

        fs::write(&entitlements, "<plist/>").expect("edit entitlements");
        assert!(
            smol::block_on(AppleBackend::requires_regeneration(&project)).expect("staleness check"),
            "an edited scaffold file is stale"
        );
        smol::block_on(reinit_backend::<AppleBackend>(&project)).expect("reinit");
        assert!(
            !smol::block_on(AppleBackend::requires_regeneration(&project))
                .expect("staleness check"),
            "the re-rendered backend is fresh again"
        );
        assert!(derived_data.exists(), "reinit preserves DerivedData");
    }

    /// An identifier Android accepts but `CFBundleIdentifier` rejects —
    /// an underscore — fails the scaffold with an Apple-named error before
    /// the generated project, codesign or provisioning ever see it.
    #[test]
    fn template_context_rejects_an_apple_invalid_bundle_identifier() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host = crate::toolchain::testing::real_toolchain_host(dir.path());
        let project = create_local_project(
            &host,
            dir.path(),
            "menu-example",
            "Menu Example",
            "com.waterui.menu_example",
        );

        let error = smol::block_on(AppleBackend::template_context(&project))
            .expect_err("an underscored identifier is not a CFBundleIdentifier");
        assert!(
            format!("{error:#}").contains("Apple"),
            "the rejection names the platform: {error:#}"
        );
    }

    /// A hyphenated identifier — invalid in a Java package name but valid as
    /// `CFBundleIdentifier` — scaffolds the Apple backend cleanly and comes
    /// back verbatim: the Apple path neither rejects nor rewrites it.
    #[test]
    fn apple_backend_accepts_and_preserves_a_hyphenated_bundle_identifier() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host = crate::toolchain::testing::real_toolchain_host(dir.path());
        let project = create_local_project(
            &host,
            dir.path(),
            "liquid-glass",
            "Liquid Glass",
            "dev.waterui.liquid-glass",
        );

        let project = smol::block_on(Project::open(
            &host,
            project.root(),
            ManagedBackends::for_backend(TargetBackend::Apple),
        ))
        .expect("the Apple backend scaffolds a hyphenated identifier");
        assert_eq!(
            project
                .bundle_identifier()
                .apple_bundle_identifier()
                .expect("Apple accepts a hyphenated CFBundleIdentifier")
                .as_str(),
            "dev.waterui.liquid-glass",
            "the identifier is preserved verbatim, never rewritten"
        );
        assert!(
            !smol::block_on(AppleBackend::requires_regeneration(&project))
                .expect("staleness check"),
            "a freshly scaffolded backend is not stale"
        );
    }
    /// Both hosts launch through `cocoa_ui`'s `SceneDelegate`; the
    /// single-scene Hydrolysis host differs only in not supporting multiple
    /// scenes.
    fn assert_scene_manifests(
        ctx: &crate::templates::TemplateContext,
        project: &Project,
        bundle_id: &crate::project_types::AppleBundleIdentifier,
    ) {
        for (multiple_scenes, host) in [(true, "apple"), (false, "hydrolysis")] {
            let dict = crate::apple::app_bundle::apple_info_plist(
                &crate::apple::app_bundle::PlistApp {
                    multiple_scenes,
                    ..crate::apple::app_bundle::PlistApp::apple_backend(ctx)
                },
                project,
                crate::platform::TargetPlatform::IOS,
                "17.0",
                "Water Example",
                bundle_id,
            );
            let manifest = dict
                .get("UIApplicationSceneManifest")
                .and_then(plist::Value::as_dictionary)
                .unwrap_or_else(|| panic!("the {host} host declares a scene manifest"));
            assert_eq!(
                manifest.get("UIApplicationSupportsMultipleScenes"),
                Some(&plist::Value::Boolean(multiple_scenes)),
                "{host} host scene support"
            );
            let delegate = manifest
                .get("UISceneConfigurations")
                .and_then(plist::Value::as_dictionary)
                .and_then(|configurations| {
                    configurations.get("UIWindowSceneSessionRoleApplication")
                })
                .and_then(plist::Value::as_array)
                .and_then(|roles| roles.first())
                .and_then(plist::Value::as_dictionary)
                .and_then(|role| role.get("UISceneDelegateClassName"));
            assert_eq!(
                delegate,
                Some(&plist::Value::String("SceneDelegate".to_string())),
                "{host} host delegate class"
            );
            assert!(dict.contains_key("UILaunchScreen"));
            assert!(dict.contains_key("UISupportedInterfaceOrientations"));
        }
    }

    /// The generated iOS `Info.plist` declares
    /// `CADisableMinimumFrameDurationOnPhone`: iOS caps `CADisplayLink` and
    /// `CAMetalLayer` presentation at 60 Hz on `ProMotion` iPhones without it
    /// (#1632), whatever frame-rate range the app requests.
    #[test]
    fn ios_info_plist_opts_out_of_the_promotion_frame_cap() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host = crate::toolchain::testing::real_toolchain_host(dir.path());
        let project = create_local_project(
            &host,
            dir.path(),
            "water-example",
            "Water Example",
            "dev.waterui.waterexample",
        );

        let project = smol::block_on(Project::open(
            &host,
            project.root(),
            ManagedBackends::for_backend(TargetBackend::Apple),
        ))
        .expect("the Apple backend scaffolds");
        let ctx =
            smol::block_on(AppleBackend::template_context(&project)).expect("template context");
        let bundle_id = project
            .bundle_identifier()
            .apple_bundle_identifier()
            .expect("a CFBundleIdentifier-legal id");

        for platform in [
            crate::platform::TargetPlatform::IOS,
            crate::platform::TargetPlatform::IOSSimulator,
        ] {
            let dict = crate::apple::app_bundle::apple_info_plist(
                &crate::apple::app_bundle::PlistApp::apple_backend(&ctx),
                &project,
                platform,
                "17.0",
                "Water Example",
                &bundle_id,
            );
            assert_eq!(
                dict.get("CADisableMinimumFrameDurationOnPhone"),
                Some(&plist::Value::Boolean(true)),
                "generated {platform:?} apps opt out of the ProMotion 60 Hz cap"
            );
            assert!(
                dict.contains_key("UIApplicationSceneManifest"),
                "the Apple backend drives {platform:?} through its scene delegate"
            );
        }

        assert_scene_manifests(&ctx, &project, &bundle_id);

        let macos = crate::apple::app_bundle::apple_info_plist(
            &crate::apple::app_bundle::PlistApp::apple_backend(&ctx),
            &project,
            crate::platform::TargetPlatform::MacOS,
            "14.0",
            "Water Example",
            &bundle_id,
        );
        assert!(
            !macos.contains_key("CADisableMinimumFrameDurationOnPhone"),
            "macOS has no phone frame cap"
        );
    }
}

use std::path::{Path, PathBuf};

use eyre::WrapErr as _;
use waterui_assets_planner::ColorScheme;

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
    pub project_path: PathBuf,
    /// The scheme to use for building the Apple project.
    pub scheme: String,
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
    let name = project.manifest().package.name.as_str();
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
        if !manifest.package.embedded {
            let _ = project
                .bundle_identifier()
                .apple_bundle_identifier()
                .map_err(|error| eyre::eyre!("{error}"))?;
        }
        let (_, app_name, crate_name_for_template) = Self::scaffold_names();
        let ios_permissions = manifest
            .permissions
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
        let launch = crate::assets::project_launch_assets(project)?;
        let launch_entry = templates::LaunchTemplateEntry {
            has_background: launch.plan().background(ColorScheme::Light).is_some(),
            has_image: launch.has_artwork(),
        };
        Ok(TemplateContext::for_project_manifest(
            project.host(),
            manifest,
            crate_name_for_template,
            app_name,
            project.resolved_framework().await?,
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
    /// dependency the ffi crate's manifest pins.
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

    /// The channel's pins resolve without a network: `waterui` and
    /// `waterui-ffi` ride `[patch.crates-io]` onto vendor stubs (the only
    /// source `[patch]` can redirect offline), and `waterui-apple` is the
    /// canonical `backends/apple` slot in the stub checkout `waterui_path`
    /// names — a path dependency — so the ffi companion's feature-table
    /// probe resolves entirely locally.
    fn vendor_offline_resolution(host: &Host, root: &Path, vendor_dir: &Path) {
        // The vendored checkout mirrors the real framework layout: the
        // `waterui` facade is the root package, `waterui-ffi` lives at
        // `ffi`, and the Apple backend is the canonical `backends/apple`
        // slot the generated manifest resolves as a path dependency.
        let stubs = [
            (
                "waterui",
                vendor_dir.to_path_buf(),
                &["dynamic_linking", "media"][..],
            ),
            (
                "waterui-ffi",
                vendor_dir.join("ffi"),
                &[
                    "android-jni",
                    "c-api",
                    "chromium",
                    "dev",
                    "gpu",
                    "inspector",
                    "map",
                    "media",
                    "video",
                    "webview",
                    "webview-cef",
                ][..],
            ),
            (
                "waterui-apple",
                vendor_dir.join("backends/apple"),
                &["map", "media", "webview"][..],
            ),
        ];
        for (name, stub, features) in stubs {
            std::fs::create_dir_all(stub.join("src")).expect("stub crate dir");
            let mut stub_manifest = toml_edit::DocumentMut::new();
            stub_manifest["package"]["name"] = toml_edit::value(name);
            stub_manifest["package"]["version"] = toml_edit::value("0.4.1");
            stub_manifest["package"]["edition"] = toml_edit::value("2021");
            for feature in features {
                stub_manifest["features"][feature] = toml_edit::value(toml_edit::Array::new());
            }
            if name == "waterui" {
                stub_manifest["workspace"]["members"] =
                    toml_edit::value(toml_edit::Array::from_iter(["ffi", "backends/apple"]));
            }
            std::fs::write(stub.join("Cargo.toml"), stub_manifest.to_string())
                .expect("stub manifest");
            std::fs::write(stub.join("src/lib.rs"), "").expect("stub lib");
        }
        let manifest_path = root.join("Cargo.toml");
        let mut document: toml_edit::DocumentMut = std::fs::read_to_string(&manifest_path)
            .expect("project Cargo.toml exists")
            .parse()
            .expect("project Cargo.toml parses");
        for (name, dir) in [
            ("waterui", vendor_dir.to_path_buf()),
            ("waterui-ffi", vendor_dir.join("ffi")),
        ] {
            document["patch"]["crates-io"][name]["path"] =
                toml_edit::value(dir.to_string_lossy().as_ref());
        }
        std::fs::write(&manifest_path, document.to_string()).expect("write the patch table");
        let water_toml = root.join("Water.toml");
        let mut water_document: toml_edit::DocumentMut = std::fs::read_to_string(&water_toml)
            .expect("Water.toml exists")
            .parse()
            .expect("Water.toml parses");
        water_document["waterui_path"] = toml_edit::value(vendor_dir.to_string_lossy().as_ref());
        std::fs::write(&water_toml, water_document.to_string())
            .expect("name the vendored framework checkout");

        // `Project::open` resolves the project's layout with `cargo metadata
        // --locked`; a plain offline resolve records the patched sources in
        // the lock first.
        let mut command = cargo_metadata::MetadataCommand::new();
        command
            .manifest_path(&manifest_path)
            .other_options(vec!["--offline".to_string()]);
        smol::block_on(host.cargo_metadata(&command))
            .expect("offline metadata resolves the patched project");
    }

    /// The scaffold emits only the entitlements file — no Xcode project
    /// exists to regenerate — yet the staleness check still detects an edit
    /// and re-renders without losing the packaging cache.
    #[test]
    fn scaffold_without_xcode_project_still_detects_staleness() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host = crate::toolchain::testing::real_toolchain_host(dir.path());
        let root = dir.path().join("water-example");
        smol::block_on(Project::create(
            &host,
            &root,
            CreateOptions {
                name: "Water Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                    .expect("bundle identifier"),
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

        vendor_offline_resolution(
            &crate::toolchain::Host::current(),
            &root,
            &dir.path().join("vendor"),
        );

        let project = smol::block_on(Project::open(
            &host,
            &root,
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
        let root = dir.path().join("menu-example");
        let project = smol::block_on(Project::create(
            &host,
            &root,
            CreateOptions {
                name: "Menu Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("com.waterui.menu_example")
                    .expect("the shared identifier grammar accepts underscores"),
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
        let root = dir.path().join("liquid-glass");
        smol::block_on(Project::create(
            &host,
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

        vendor_offline_resolution(
            &crate::toolchain::Host::current(),
            &root,
            &dir.path().join("vendor"),
        );

        let project = smol::block_on(Project::open(
            &host,
            &root,
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
    /// The generated iOS `Info.plist` declares
    /// `CADisableMinimumFrameDurationOnPhone`: iOS caps `CADisplayLink` and
    /// `CAMetalLayer` presentation at 60 Hz on `ProMotion` iPhones without it
    /// (#1632), whatever frame-rate range the app requests.
    #[test]
    fn ios_info_plist_opts_out_of_the_promotion_frame_cap() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host = crate::toolchain::testing::real_toolchain_host(dir.path());
        let root = dir.path().join("water-example");
        smol::block_on(Project::create(
            &host,
            &root,
            CreateOptions {
                name: "Water Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                    .expect("bundle identifier"),
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

        vendor_offline_resolution(
            &crate::toolchain::Host::current(),
            &root,
            &dir.path().join("vendor"),
        );

        let project = smol::block_on(Project::open(
            &host,
            &root,
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
                &ctx,
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
        }

        let macos = crate::apple::app_bundle::apple_info_plist(
            &ctx,
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

//! Asset and font management for `WaterUI` projects.
//!
//! This module provides functionality to:
//! - Scan the project manifest (`[[assets.font]]` in `Water.toml`) and
//!   dependency crates (`[[package.metadata.waterui.assets.font]]`) for font
//!   declarations
//! - Resolve fonts from local paths, the font cache, or the built-in registry
//! - Copy assets to platform-specific locations

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use cargo_metadata::PackageId;
use eyre::{Context, OptionExt};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use smol::fs;
use target_lexicon::Triple;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use waterui_assets_planner::{
    BundleManifest, FontDeclaration, FontPlatform, FontSource, GraphScope,
    dependency_font_declarations, font_platform_scope,
};
use zenwave::{Client as _, Method};

use crate::project::Project;
use crate::project_model::project_types::PermissionKey;

mod android_manifest;
mod app_values;
mod apple_metadata;
mod gradle_plugins;
pub mod icon;
mod unified;
mod web;

pub use android_manifest::ManifestComponents;
#[cfg(test)]
pub use android_manifest::assert_component_markers_inside_application;
pub use app_values::{AppValuesConfig, RequiredAppValues};
pub use apple_metadata::{AppleDeclarations, SigningEnvironment};
pub use gradle_plugins::GradlePlugins;
#[cfg(test)]
pub use gradle_plugins::{assert_module_plugin_markers, assert_settings_plugin_markers};

/// A font the CLI can fetch when a crate names it and nothing else.
#[derive(Debug, Clone, Deserialize)]
struct RegistryFont {
    /// Family name a crate declares.
    name: String,
    /// Where the face, or an archive containing it, is fetched from.
    url: String,
    /// The SHA-256 of what `url` serves; a download that does not match is
    /// refused.
    sha256: String,
}

/// Where a remote font is fetched from, and the digest a download from there
/// must match when the source pins one.
#[derive(Debug, Clone, Copy)]
struct FontOrigin<'a> {
    url: &'a str,
    /// Pinned by every built-in registry entry; a crate's `remote_path`
    /// declaration carries none.
    sha256: Option<&'a str>,
}

/// The built-in font registry.
///
/// The list is data, so it lives in `assets/fonts.toml` rather than in Rust
/// literals, and is parsed rather than compiled into a table. Fonts it offers
/// can be declared by name alone — in a crate's Cargo.toml:
///
/// ```toml
/// [[package.metadata.waterui.assets.font]]
/// name = "Inter"
/// ```
///
/// or in the project's `Water.toml`:
///
/// ```toml
/// [[assets.font]]
/// name = "Inter"
/// ```
///
/// Icon pack fonts (Font Awesome, Material Icons, Lucide, ...) do NOT belong
/// here. They declare their own faces in their own Cargo.toml with
/// `remote_path` and `required-feature`.
#[derive(Debug, Clone, Deserialize)]
struct FontRegistry {
    #[serde(rename = "font")]
    fonts: Vec<RegistryFont>,
}

impl FontRegistry {
    /// The registry shipped with this CLI.
    fn builtin() -> eyre::Result<Self> {
        toml::from_str(include_str!("assets/fonts.toml"))
            .wrap_err("built-in font registry `src/project_model/assets/fonts.toml` is malformed")
    }

    /// Where `name` is fetched from, if the registry offers it.
    fn origin(&self, name: &str) -> Option<FontOrigin<'_>> {
        self.fonts
            .iter()
            .find(|font| font.name == name)
            .map(|font| FontOrigin {
                url: &font.url,
                sha256: Some(&font.sha256),
            })
    }
}
const HYDROLYSIS_DEFAULT_FONT_FAMILY: &str = "Roboto";
const FONT_MANIFEST_FILE_NAME: &str = "waterui-fonts.json";

/// A resolved font with its absolute path.
#[derive(Debug, Clone)]
pub struct ResolvedFont {
    /// Font family name.
    pub name: String,
    /// Absolute path to the font file.
    pub path: PathBuf,
}

/// The `waterui-fonts.json` manifest staged beside bundled font files: the
/// declared-family → file-name map a runtime's font table loads at bootstrap.
/// `default_family` is emitted only by the Hydrolysis web runtime, which has
/// no system fonts to fall back on.
#[derive(Debug, Serialize)]
struct FontManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    default_family: Option<String>,
    fonts: Vec<FontManifestEntry>,
}

#[derive(Debug, Serialize)]
struct FontManifestEntry {
    name: String,
    file_name: String,
}

/// The CLI's own keys of a crate's `[package.metadata.waterui]`; font
/// declarations under `assets` are read by
/// [`waterui_assets_planner::dependency_font_declarations`].
#[derive(Debug, Deserialize)]
struct WaterUIMetadata {
    /// Permissions this crate cannot work without, keyed by logical permission.
    #[serde(default)]
    permissions: BTreeMap<PermissionKey, PermissionRequirement>,
    /// Kotlin sources and Maven dependencies to place on the Android
    /// application classpath, from `[package.metadata.waterui.android]`.
    #[serde(default)]
    android: AndroidMetadata,
    /// Entitlements and `Info.plist` keys for the packaged Apple app, from
    /// `[package.metadata.waterui.apple]`.
    #[serde(default)]
    apple: apple_metadata::AppleMetadata,
    /// Values the app supplies in `Water.toml`, from
    /// `[[package.metadata.waterui.app-value]]`.
    #[serde(default, rename = "app-value")]
    app_value: Vec<app_values::AppValueRequest>,
}

/// One crate's `[package.metadata.waterui.android]` table: an unconditional
/// base plus `feature.<cargo-feature>` subtables carrying the same keys.
type AndroidMetadata = FeatureTables<AndroidTable>;

/// A crate's `[package.metadata.waterui.<platform>]` table, split into its
/// unconditional `base` keys and `feature.<cargo-feature>` subtables that
/// carry the same keys but contribute only while the resolved dependency
/// graph enables that feature on the declaring crate.
#[derive(Debug, Default)]
struct FeatureTables<T> {
    /// The table's unconditional keys.
    base: T,
    /// `feature.<cargo-feature>` subtables, keyed by cargo feature name.
    feature: BTreeMap<String, T>,
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FeatureTables<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // `deny_unknown_fields` cannot see through `flatten`, so the keys
        // neither `base` nor `feature` claims land in a catch-all that fails
        // the parse itself.
        #[derive(Deserialize)]
        #[serde(bound(deserialize = "T: Deserialize<'de>"))]
        struct Wire<T> {
            #[serde(flatten)]
            base: T,
            #[serde(default)]
            feature: BTreeMap<String, T>,
            #[serde(flatten)]
            unknown: BTreeMap<String, serde_json::Value>,
        }
        let wire = Wire::<T>::deserialize(deserializer)?;
        if let Some(key) = wire.unknown.into_keys().next() {
            return Err(<D::Error as serde::de::Error>::custom(format!(
                "unknown field `{key}`"
            )));
        }
        Ok(Self {
            base: wire.base,
            feature: wire.feature,
        })
    }
}

/// The keys of one crate's `[package.metadata.waterui.android]` table and of
/// each `[package.metadata.waterui.android.feature.<cargo-feature>]` subtable.
///
/// A crate whose Rust side resolves helper classes through the application
/// class loader declares the `.kt` files that must be compiled into the app
/// dex and the Maven coordinates the helpers need. The generated Gradle
/// module performs the compile — the crate's build script does not. A crate
/// whose platform code needs an entry inside the manifest's `<application>`
/// declares it as an `[[activity]]`, `[[provider]]`, `[[service]]`,
/// `[[receiver]]` or `[[meta-data]]` table (see [`android_manifest`]). A crate whose platform
/// code needs a Gradle plugin applied to the application module declares it
/// as a `[[gradle-plugin]]` table (see [`gradle_plugins`]).
///
/// Every key is the CLI's, so an unknown one — a misspelling, or a key a
/// newer CLI understands — is an error rather than a declaration silently
/// left out of the app.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct AndroidTable {
    /// Crate-relative `.kt` files to stage into the generated module.
    #[serde(default)]
    kotlin_sources: Vec<PathBuf>,
    /// Maven `group:artifact:version` coordinates the helpers compile and
    /// run against.
    #[serde(default)]
    maven: Vec<String>,
    /// `<activity>` entries for the generated manifest.
    #[serde(default)]
    activity: Vec<android_manifest::Activity>,
    /// `<provider>` entries for the generated manifest.
    #[serde(default)]
    provider: Vec<android_manifest::Provider>,
    /// `<service>` entries for the generated manifest.
    #[serde(default)]
    service: Vec<android_manifest::Service>,
    /// `<receiver>` entries for the generated manifest.
    #[serde(default)]
    receiver: Vec<android_manifest::Receiver>,
    /// Application-level `<meta-data>` entries for the generated manifest.
    #[serde(default)]
    meta_data: Vec<android_manifest::MetaData>,
    /// Gradle plugins to apply to the generated application module.
    #[serde(default)]
    gradle_plugin: Vec<gradle_plugins::GradlePlugin>,
}

impl AndroidTable {
    /// Whether the table declares nothing.
    const fn is_empty(&self) -> bool {
        self.kotlin_sources.is_empty()
            && self.maven.is_empty()
            && self.activity.is_empty()
            && self.provider.is_empty()
            && self.service.is_empty()
            && self.receiver.is_empty()
            && self.meta_data.is_empty()
            && self.gradle_plugin.is_empty()
    }
}

/// One crate's declaration that it needs a permission to function.
#[derive(Debug, Deserialize)]
struct PermissionRequirement {
    /// Human-readable justification, shown to the application author.
    reason: String,
    /// Only required when this cargo feature is enabled on the declaring crate.
    #[serde(default, rename = "required-feature")]
    required_feature: Option<String>,
}

/// A permission some dependency needs, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredPermission {
    /// Crate that declared the requirement.
    pub package: String,
    /// The logical permission.
    pub key: PermissionKey,
    /// Why that crate needs it.
    pub reason: String,
    /// How the requirement was established.
    pub evidence: PermissionEvidence,
}

/// How confident the audit is that a permission is actually needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionEvidence {
    /// The crate declared the requirement in its own manifest metadata.
    Declared,
    /// Inferred from the shape of the dependency graph; may be a false
    /// positive, so the report is phrased as a suggestion.
    Inferred,
}

/// Fonts the project itself declares as `[[assets.font]]` tables in
/// `Water.toml`.
///
/// The fields mirror what a crate writes under
/// `[package.metadata.waterui.assets.font]`; `local_path` is resolved against
/// the project root rather than a crate root. A declaration is one source:
/// setting both paths — or an absolute `local_path` — is a manifest error.
fn manifest_font_declarations(
    manifest: &crate::project::Manifest,
    root: &Path,
) -> eyre::Result<Vec<FontDeclaration>> {
    let Some(assets) = manifest.assets.as_ref() else {
        return Ok(Vec::new());
    };
    let mut declarations = Vec::with_capacity(assets.font.len());
    for font in &assets.font {
        let source = FontSource::from_declaration(
            root,
            font.local_path.clone(),
            font.remote_path.clone(),
            &manifest.package.name,
            &font.name,
        )
        .wrap_err_with(|| format!("[[assets.font]] entry '{}' in Water.toml", font.name))?;
        let platforms =
            font_platform_scope(font.platforms.clone(), &manifest.package.name, &font.name)
                .wrap_err_with(|| format!("[[assets.font]] entry '{}' in Water.toml", font.name))?;
        declarations.push(FontDeclaration {
            name: font.name.clone(),
            source,
            crate_name: manifest.package.name.clone(),
            platforms,
        });
    }
    Ok(declarations)
}

/// Validate the project's own declarations without resolving a Cargo graph.
pub async fn validate_manifest_fonts(
    host: &crate::toolchain::Host,
    manifest: &crate::project::Manifest,
    root: &Path,
) -> eyre::Result<()> {
    let declarations = manifest_font_declarations(manifest, root)?;
    if !declarations.is_empty() {
        resolve_fonts(host, declarations).await?;
    }
    Ok(())
}

/// Scans the project manifest and the built crate's dependencies for font
/// declarations.
///
/// `[[assets.font]]` tables in `Water.toml` declare the app's own fonts and
/// rank ahead of dependency declarations between equal sources; the
/// dependency half comes from [`scan_crate_font_declarations`].
///
/// Only the declarations a build for one of `platforms` bundles are kept: a
/// declaration scoped to other platforms is left out. A build whose staged
/// fonts serve several platforms at once — an `XCFramework` for iOS and
/// macOS — names them all.
pub async fn scan_fonts(
    project: &Project,
    build_manifest: &Path,
    platforms: &[FontPlatform],
) -> eyre::Result<Vec<FontDeclaration>> {
    let mut declarations = manifest_font_declarations(project.manifest(), project.root())?;
    declarations.extend(scan_crate_font_declarations(project, build_manifest).await?);
    declarations.retain(|declaration| {
        platforms
            .iter()
            .any(|platform| declaration.bundled_on(*platform))
    });
    Ok(declarations)
}

/// Seed a managed crate's `Cargo.lock` before a `cargo metadata` call
/// resolves it.
///
/// Every managed crate the CLI resolves — the font scan, capability and
/// permission probes, `prepare_build` — must lead with the channel's
/// certified pins, or Cargo locks whatever the registry holds newest: an
/// `accesskit` generation `Water.lock` contradicts (#203). The project
/// manifest's own graph already resolves from its lock, so a manifest in the
/// project root takes no seed and returns `false`. The returned flag is
/// whether the manifest belongs to a managed crate.
///
/// # Errors
///
/// Returns an error naming the path when `build_manifest` does not exist —
/// every caller resolves a manifest it scaffolded first — and when the
/// certified lock or the seed cannot be read or written.
pub async fn seed_managed_crate_lock(
    project: &Project,
    build_manifest: &Path,
) -> eyre::Result<bool> {
    let Some(dir) = build_manifest.parent().filter(|dir| *dir != project.root()) else {
        return Ok(false);
    };
    if !build_manifest.exists() {
        return Err(eyre::eyre!(
            "no manifest to resolve at {}",
            build_manifest.display()
        ));
    }
    // `resolved_framework`, not `manifest().framework`: a `waterui_path`
    // project persists no framework record (the path is the record), but its
    // checkout's own `Cargo.lock` is still the canonical pin for the
    // transitive packages only the managed crate reaches.
    let has_framework =
        project.manifest().framework.is_some() || project.manifest().waterui_path.is_some();
    let canonical = if has_framework {
        project
            .resolved_framework()
            .await?
            .canonical_lock(project.root())
            .await?
    } else {
        None
    };
    crate::templates::seed_lockfile(dir, &project.lockfile_path().await?, canonical.as_ref())
        .await?;
    Ok(true)
}

/// `cargo metadata` on a manifest, run on `host`. `features` mirrors the
/// feature selection the build invokes with — optional dependencies (and the
/// metadata they declare) only enter the resolved graph under it; an empty
/// slice resolves the manifest's default feature set.
pub async fn crate_metadata(
    host: &crate::toolchain::Host,
    build_manifest: &Path,
    features: &[String],
) -> eyre::Result<cargo_metadata::Metadata> {
    let mut command = cargo_metadata::MetadataCommand::new();
    command.manifest_path(build_manifest);
    if !features.is_empty() {
        command.features(cargo_metadata::CargoOpt::SomeFeatures(features.to_vec()));
    }
    host.cargo_metadata(&command).await.map_err(Into::into)
}

/// The features cargo resolved on each package of `metadata`'s graph — what
/// every feature-gated declaration is checked against.
fn resolved_features(metadata: &cargo_metadata::Metadata) -> HashMap<&PackageId, HashSet<&str>> {
    metadata
        .resolve
        .as_ref()
        .map(|resolve| {
            resolve
                .nodes
                .iter()
                .map(|node| (&node.id, node.features.iter().map(|f| f.as_str()).collect()))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether cargo resolved `feature` on `package`.
fn feature_enabled(
    enabled: &HashMap<&PackageId, HashSet<&str>>,
    package: &cargo_metadata::Package,
    feature: &str,
) -> bool {
    enabled
        .get(&package.id)
        .is_some_and(|features| features.contains(feature))
}

/// `package`'s `[package.metadata.waterui]` table; `None` when it declares
/// none. A table that does not parse — an unknown or misspelt key, a value
/// of the wrong shape — is an error naming the crate.
fn parse_waterui_metadata(
    package: &cargo_metadata::Package,
) -> eyre::Result<Option<WaterUIMetadata>> {
    let Some(waterui) = package.metadata.get("waterui") else {
        return Ok(None);
    };
    serde_json::from_value(waterui.clone())
        .map(Some)
        .map_err(|error| {
            eyre::eyre!(
                "crate {} declares malformed `[package.metadata.waterui]`: {error}",
                package.name
            )
        })
}

/// Scans `build_manifest`'s dependency graph for
/// `[package.metadata.waterui.assets.font]` declarations via `cargo metadata`.
///
/// `build_manifest` is the `Cargo.toml` of the crate this build compiles,
/// i.e. the generated backend or FFI crate. That crate depends on the app, so
/// the graph carries both the app's authored dependencies and the backend's
/// own (the theme crate and friends); scanning the app manifest instead
/// would miss the backend's declarations entirely.
///
/// Fonts with a `required-feature` field will only be included if that feature
/// is enabled for the declaring package (checked via cargo metadata's resolved
/// graph, which is why the metadata is resolved rather than `--no-deps`). A
/// declaration that is malformed, roots its `local_path`, or sets both
/// `local_path` and `remote_path` fails the scan.
async fn scan_crate_font_declarations(
    project: &Project,
    build_manifest: &Path,
) -> eyre::Result<Vec<FontDeclaration>> {
    debug!(
        "Scanning fonts from dependencies via cargo metadata on {}",
        build_manifest.display()
    );

    let managed = seed_managed_crate_lock(project, build_manifest).await?;

    let metadata = crate_metadata(project.host(), build_manifest, &[])
        .await
        .wrap_err_with(|| {
        let mut message = format!(
            "Failed to run cargo metadata on {}",
            build_manifest.display()
        );
        if managed {
            // A lock committed before the channel's pins existed can pin a
            // generation they contradict; regeneration is the fix, not a
            // retry (#203).
            let dir = build_manifest.parent().unwrap_or_else(|| project.root());
            let _ = write!(
                message,
                "; if its Cargo.lock predates the channel's pins, remove it and `Cargo.lock.seed` in {} and run again to regenerate them",
                dir.display()
            );
        }
        message
    })?;

    let fonts = dependency_font_declarations(&metadata, GraphScope::Build).wrap_err_with(|| {
        format!(
            "Invalid font declaration in the dependency graph of {}",
            build_manifest.display()
        )
    })?;

    info!("Found {} font declarations from dependencies", fonts.len());
    Ok(fonts)
}

/// Everything the dependency graph's `[package.metadata.waterui.android]`
/// tables contribute to a generated Gradle module, after
/// `feature.<cargo-feature>` gating.
#[derive(Debug, Default)]
pub struct AndroidDeclarations {
    /// Kotlin sources and Maven coordinates for the module's classpath.
    pub classpath: AndroidClasspath,
    /// Components for the `<application>` element of the module's manifest.
    pub manifest: ManifestComponents,
    /// Gradle plugins for the application module and the project settings.
    pub gradle_plugins: GradlePlugins,
    /// Values the graph requests from the app's `Water.toml`.
    pub app_values: RequiredAppValues,
}

/// Kotlin sources and Maven coordinates the dependency graph asks to place on
/// the Android application classpath, after `feature.<cargo-feature>` gating.
#[derive(Debug, Default)]
pub struct AndroidClasspath {
    /// Absolute paths of `.kt` files to stage into the generated module.
    pub kotlin_sources: Vec<PathBuf>,
    /// `group:artifact:version` coordinates to emit as Gradle dependencies.
    pub maven: BTreeSet<String>,
}

/// Which Gradle configuration a module's classpath dependencies land on.
///
/// `implementation` hides them from consumers — correct for an application
/// module. `api` exports them through the published POM — required for the
/// embedded AAR, whose host app resolves the classes at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidDependencyScope {
    /// `implementation(...)`: app module and hydrolysis app.
    Implementation,
    /// `api(...)`: the embedded `waterui` module consumers depend on.
    Api,
}

impl AndroidDependencyScope {
    const fn gradle_keyword(self) -> &'static str {
        match self {
            Self::Implementation => "implementation",
            Self::Api => "api",
        }
    }
}

/// Scans `build_manifest`'s dependency graph for
/// `[package.metadata.waterui.android]` declarations via `cargo metadata` —
/// the same channel the font and permission scans read.
///
/// A declared source that cannot be read is an error: it is a class the app
/// would miss in its dex and fail to resolve at runtime. Conflicting manifest
/// components are an error too: the manifest can ship only one of them.
pub async fn scan_android_declarations(
    project: &Project,
    build_manifest: &Path,
    features: &[String],
) -> eyre::Result<AndroidDeclarations> {
    seed_managed_crate_lock(project, build_manifest).await?;
    let metadata = crate_metadata(project.host(), build_manifest, features)
        .await
        .wrap_err_with(|| {
            format!(
                "Failed to run cargo metadata on {}",
                build_manifest.display()
            )
        })?;
    collect_android_declarations(&metadata)
}

/// A `feature.<name>` subtable must name a cargo feature the declaring crate
/// actually has: a misspelt one would gate its declarations behind a switch
/// nothing can turn on.
fn check_feature_tables<'a>(
    package: &cargo_metadata::Package,
    table: &str,
    features: impl Iterator<Item = &'a String>,
) -> eyre::Result<()> {
    for feature in features {
        eyre::ensure!(
            package.features.contains_key(feature.as_str()),
            "{} declares `[package.metadata.waterui.{table}.feature.{feature}]` but has no cargo feature `{feature}`",
            package.name
        );
    }
    Ok(())
}

/// The graph walk of [`scan_android_declarations`], split from the
/// `cargo metadata` call so collection runs against any resolved graph.
fn collect_android_declarations(
    metadata: &cargo_metadata::Metadata,
) -> eyre::Result<AndroidDeclarations> {
    let enabled_features = resolved_features(metadata);

    let mut declarations = AndroidDeclarations::default();
    for package in &metadata.packages {
        let Some(parsed) = parse_waterui_metadata(package)? else {
            continue;
        };
        request_app_values(
            &mut declarations.app_values,
            &enabled_features,
            package,
            parsed.app_value,
        );
        let android = parsed.android;
        check_feature_tables(package, "android", android.feature.keys())?;
        let mut tables = Vec::with_capacity(android.feature.len() + 1);
        tables.push(android.base);
        for (feature, table) in android.feature {
            if feature_enabled(&enabled_features, package, &feature) {
                tables.push(table);
            } else {
                debug!(
                    "Skipping `[package.metadata.waterui.android.feature.{feature}]` of {}: the feature is not enabled",
                    package.name
                );
            }
        }
        if tables.iter().all(AndroidTable::is_empty) {
            continue;
        }
        let crate_root = package
            .manifest_path
            .parent()
            .ok_or_eyre("Package has no parent directory")?
            .as_std_path()
            .to_path_buf();
        for table in tables {
            let components = android_manifest::DeclaredComponents {
                activities: table.activity,
                providers: table.provider,
                services: table.service,
                receivers: table.receiver,
                meta_data: table.meta_data,
            };
            for source in table.kotlin_sources {
                let path = crate_root.join(&source);
                if !path.is_file() {
                    eyre::bail!(
                        "{} declares Kotlin source `{}`: no such file at {}",
                        package.name,
                        source.display(),
                        path.display()
                    );
                }
                declarations.classpath.kotlin_sources.push(path);
            }
            for coordinate in table.maven {
                let parts: Vec<&str> = coordinate.split(':').collect();
                eyre::ensure!(
                    parts.len() == 3 && parts.iter().all(|part| !part.is_empty()),
                    "{} declares Maven coordinate `{coordinate}`: expected `group:artifact:version`",
                    package.name
                );
                declarations.classpath.maven.insert(coordinate);
            }
            declarations
                .manifest
                .merge(package.name.as_str(), components)?;
            declarations
                .gradle_plugins
                .merge(package.name.as_str(), table.gradle_plugin)?;
        }
    }
    Ok(declarations)
}

/// Scans `build_manifest`'s dependency graph for
/// `[package.metadata.waterui.apple]` declarations via `cargo metadata` —
/// the same channel the Android declarations travel.
///
/// # Errors
///
/// Returns an error when `cargo metadata` fails, a table is malformed, or two
/// crates declare one key with values that cannot merge.
pub async fn scan_apple_declarations(
    project: &Project,
    build_manifest: &Path,
    features: &[String],
) -> eyre::Result<AppleDeclarations> {
    seed_managed_crate_lock(project, build_manifest).await?;
    let metadata = crate_metadata(project.host(), build_manifest, features)
        .await
        .wrap_err_with(|| {
            format!(
                "Failed to run cargo metadata on {}",
                build_manifest.display()
            )
        })?;
    collect_apple_declarations(&metadata)
}

/// The graph walk of [`scan_apple_declarations`], split from the
/// `cargo metadata` call so collection runs against any resolved graph.
fn collect_apple_declarations(
    metadata: &cargo_metadata::Metadata,
) -> eyre::Result<AppleDeclarations> {
    let enabled_features = resolved_features(metadata);
    let mut declarations = AppleDeclarations::default();
    for package in &metadata.packages {
        let Some(parsed) = parse_waterui_metadata(package)? else {
            continue;
        };
        request_app_values(
            &mut declarations.app_values,
            &enabled_features,
            package,
            parsed.app_value,
        );
        let apple = parsed.apple;
        check_feature_tables(package, "apple", apple.feature.keys())?;
        let mut tables = Vec::with_capacity(apple.feature.len() + 1);
        tables.push(apple.base);
        for (feature, table) in apple.feature {
            if feature_enabled(&enabled_features, package, &feature) {
                tables.push(table);
            } else {
                debug!(
                    "Skipping `[package.metadata.waterui.apple.feature.{feature}]` of {}: the feature is not enabled",
                    package.name
                );
            }
        }
        for table in tables {
            if table.is_empty() {
                continue;
            }
            declarations.merge(package.name.as_str(), table)?;
        }
    }
    Ok(declarations)
}

/// Records the app values `package` requests whose `required-feature` cargo
/// resolved.
fn request_app_values(
    required: &mut RequiredAppValues,
    enabled_features: &HashMap<&PackageId, HashSet<&str>>,
    package: &cargo_metadata::Package,
    requests: Vec<app_values::AppValueRequest>,
) {
    for request in requests {
        if let Some(gate) = &request.required_feature
            && !feature_enabled(enabled_features, package, gate)
        {
            debug!(
                "Skipping app value `{:?}` requested by {}: feature `{gate}` is not enabled",
                request.key, package.name
            );
            continue;
        }
        required.request(package.name.as_str(), request.key);
    }
}

/// Markers bracketing the R8 keep block [`stage_android_declarations`] maintains
/// in a module's `proguard-rules.pro`. The block is rewritten wholesale on
/// every stage so keeps track the dependency graph exactly.
const ANDROID_KEEPS_BEGIN: &str = "# --- begin waterui android classpath keeps ---";
const ANDROID_KEEPS_END: &str = "# --- end waterui android classpath keeps ---";

/// Stages a dependency graph's Android declarations into a Gradle module: the
/// manifest components into the managed block inside `<application>` of the
/// module's `AndroidManifest.xml`, `.kt` files under `src/main/java/waterui/` — a directory the
/// module's Kotlin compile picks up — and the Maven coordinates the helpers
/// compile and run against, emitted into the module's managed dependencies
/// block as `implementation(...)` for application modules or `api(...)` for
/// the embedded AAR so the published POM propagates them to consumers.
///
/// Declared Gradle plugins are pinned in the project's `settings.gradle.kts`
/// and applied in the module's `plugins {}` block for application modules;
/// the embedded AAR is a library, so its host application must apply them
/// and the stage names each one.
///
/// Both destinations are managed: whatever an earlier stage left is removed
/// first, so a dependency or feature that is no longer in the graph stops
/// shipping its classes in the package.
///
/// Staged classes only ever run through name-based lookups — JNI reaches them
/// via `context.getClassLoader().loadClass` — which R8 cannot see, so release
/// builds would shrink or rename them away. The stage therefore also rewrites
/// a managed keep block in the module's `proguard-rules.pro`, one rule per
/// package a staged source carries. Maven artifacts stay untouched: their
/// classes are referenced statically from the helpers, which R8 sees.
pub async fn stage_android_declarations(
    project: &Project,
    build_manifest: &Path,
    module_dir: &Path,
    scope: AndroidDependencyScope,
    features: &[String],
) -> eyre::Result<()> {
    let declarations = scan_android_declarations(project, build_manifest, features).await?;
    stage_classpath_files(&declarations.classpath, module_dir, scope).await?;
    android_manifest::write_manifest_components(module_dir, &declarations.manifest).await?;
    app_values::stage_android_app_values(
        module_dir,
        project.root(),
        &declarations.app_values,
        &project.manifest().app_values,
        scope,
    )
    .await?;
    match scope {
        AndroidDependencyScope::Implementation => {
            let project_dir = module_dir
                .parent()
                .ok_or_eyre("the Gradle module has no parent project directory")?;
            gradle_plugins::write_gradle_plugins(
                project_dir,
                module_dir,
                &declarations.gradle_plugins,
            )
            .await
        }
        AndroidDependencyScope::Api => {
            warn_host_applies_gradle_plugins(&declarations.gradle_plugins);
            Ok(())
        }
    }
}

/// A Gradle plugin acts on the application module — `google-services`
/// matches its configuration against the application id — so the embedded
/// AAR, a library, cannot apply one for its host. The host application
/// module must apply each plugin the graph declares itself.
fn warn_host_applies_gradle_plugins(plugins: &GradlePlugins) {
    for (crate_name, id, version) in plugins.iter() {
        warn!(
            "{crate_name} needs Gradle plugin `{id}` version `{version}`; the embedded library cannot apply it, so the host application module must declare `id(\"{id}\") version \"{version}\"`"
        );
    }
}

/// The classpath half of [`stage_android_declarations`], split from the cargo-metadata
/// scan so the staging itself is exercised without a project.
async fn stage_classpath_files(
    classpath: &AndroidClasspath,
    module_dir: &Path,
    scope: AndroidDependencyScope,
) -> eyre::Result<()> {
    let mut keep_packages = BTreeSet::new();

    let java_dir = module_dir.join("src/main/java/waterui");
    let mut staged = HashSet::new();
    if classpath.kotlin_sources.is_empty() {
        if java_dir.exists() {
            fs::remove_dir_all(&java_dir).await?;
        }
    } else {
        fs::create_dir_all(&java_dir).await?;
        for source in &classpath.kotlin_sources {
            let Some(name) = source.file_name() else {
                eyre::bail!("Kotlin source {} has no file name", source.display());
            };
            eyre::ensure!(
                staged.insert(name.to_owned()),
                "two crates declare a Kotlin source named `{}`",
                name.to_string_lossy()
            );
            let package = fs::read_to_string(source)
                .await?
                .lines()
                .find_map(|line| line.strip_prefix("package "))
                .map(|rest| rest.trim().trim_end_matches(';').to_owned())
                .ok_or_else(|| {
                    eyre::eyre!(
                        "Kotlin source {} has no `package` declaration; the staged-class keep rule cannot name it",
                        source.display()
                    )
                })?;
            keep_packages.insert(package);
            crate::utils::copy_file_if_changed(source, &java_dir.join(name)).await?;
        }
        // Files an earlier classpath staged that no source still declares
        // would keep a stale `keep` rule — drop them. The rest write only
        // on change, so an unchanged build leaves Gradle's inputs alone.
        let mut entries = fs::read_dir(&java_dir).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            if staged.contains(&entry.file_name()) {
                continue;
            }
            let path = entry.path();
            if entry.metadata().await?.is_dir() {
                fs::remove_dir_all(&path).await?;
            } else {
                fs::remove_file(&path).await?;
            }
        }
        info!(
            "Staged {} Kotlin sources into {}",
            classpath.kotlin_sources.len(),
            module_dir.display()
        );
    }

    // A stale `libs/` belongs to the vendored-jar era the classpath staging
    // replaced; drop it so an earlier stage's output cannot linger.
    let libs_dir = module_dir.join("libs");
    if libs_dir.exists() {
        fs::remove_dir_all(&libs_dir).await?;
    }

    write_android_dependencies(module_dir, &classpath.maven, scope).await?;
    write_android_keeps(module_dir, &keep_packages).await?;
    Ok(())
}

/// Markers bracketing the dependency block [`stage_android_declarations`]
/// maintains inside a module's `build.gradle.kts` `dependencies` block.
/// The templates emit the marker pair empty; the stage fills it.
const ANDROID_DEPS_BEGIN: &str = "    // --- begin waterui android classpath dependencies ---";
const ANDROID_DEPS_END: &str = "    // --- end waterui android classpath dependencies ---";

/// Positions of `begin`/`end` marker pairs for a managed block: exactly one
/// well-formed pair, or none. Anything else — a stray marker, a duplicated
/// block, an `end` ahead of its `begin` — is a corrupt managed region and an
/// error rather than a second appended block.
fn managed_block_span(
    existing: &str,
    path: &Path,
    begin_marker: &str,
    end_marker: &str,
) -> eyre::Result<Option<(usize, usize)>> {
    let begins: Vec<usize> = existing
        .match_indices(begin_marker)
        .map(|(i, _)| i)
        .collect();
    let ends: Vec<usize> = existing.match_indices(end_marker).map(|(i, _)| i).collect();
    eyre::ensure!(
        begins.len() <= 1 && ends.len() <= 1,
        "{} contains more than one managed block between `{begin_marker}` and `{end_marker}`; remove the duplicates",
        path.display()
    );
    match (begins.first(), ends.first()) {
        (Some(begin), Some(end)) if begin < end => Ok(Some((*begin, *end + end_marker.len()))),
        (None, None) => Ok(None),
        _ => eyre::bail!(
            "{} has a malformed managed block: `{begin_marker}` and `{end_marker}` must appear as one ordered pair",
            path.display()
        ),
    }
}

/// `existing` with its managed block between `begin_marker` and
/// `end_marker` replaced by `block`, which carries the markers itself;
/// `None` when `existing` has no such block.
fn splice_managed_block(
    existing: &str,
    path: &Path,
    begin_marker: &str,
    end_marker: &str,
    block: &str,
) -> eyre::Result<Option<String>> {
    Ok(
        managed_block_span(existing, path, begin_marker, end_marker)?.map(|(begin, end)| {
            let mut body = String::with_capacity(existing.len() + block.len());
            body.push_str(&existing[..begin]);
            body.push_str(block);
            body.push_str(&existing[end..]);
            body
        }),
    )
}

/// Rewrites the managed dependencies block inside `module_dir`'s
/// `build.gradle.kts`: one `<scope>("group:artifact:version")` line per
/// coordinate the classpath carries. The templates ship the marker pair
/// inside `dependencies {}`; a file missing it was hand-edited or generated
/// before this mechanism and is an error.
async fn write_android_dependencies(
    module_dir: &Path,
    maven: &BTreeSet<String>,
    scope: AndroidDependencyScope,
) -> eyre::Result<()> {
    let build_file = module_dir.join("build.gradle.kts");
    let existing = fs::read_to_string(&build_file)
        .await
        .wrap_err_with(|| format!("reading module build script {}", build_file.display()))?;
    let span = managed_block_span(&existing, &build_file, ANDROID_DEPS_BEGIN, ANDROID_DEPS_END)?
        .ok_or_else(|| {
            eyre::eyre!(
                "{} has no managed dependencies block ({ANDROID_DEPS_BEGIN} / {ANDROID_DEPS_END}); the module template must emit the marker pair inside `dependencies {{}}`",
                build_file.display()
            )
        })?;

    let mut body = String::with_capacity(existing.len());
    body.push_str(&existing[..span.0]);
    body.push_str(ANDROID_DEPS_BEGIN);
    body.push('\n');
    for coordinate in maven {
        body.push_str("        ");
        body.push_str(scope.gradle_keyword());
        body.push_str("(\"");
        body.push_str(coordinate);
        body.push_str("\")\n");
    }
    body.push_str(ANDROID_DEPS_END);
    body.push_str(&existing[span.1..]);

    super::templates::write_file_if_changed(&build_file, body.as_bytes())
        .await
        .map_err(Into::into)
}

/// Rewrites the managed keep block in `module_dir/proguard-rules.pro`: one
/// `-keep class <pkg>.** { *; }` per package the staged classpath carries, so
/// R8's release shrink cannot remove or rename classes JNI loads by name.
/// An empty set removes a block an earlier stage left; the file itself is
/// only written when its contents change.
async fn write_android_keeps(module_dir: &Path, packages: &BTreeSet<String>) -> eyre::Result<()> {
    let rules_path = module_dir.join("proguard-rules.pro");
    let existing = match fs::read_to_string(&rules_path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };

    let mut body = if let Some((begin, end)) = managed_block_span(
        &existing,
        &rules_path,
        ANDROID_KEEPS_BEGIN,
        ANDROID_KEEPS_END,
    )? {
        let mut body = existing[..begin].to_owned();
        body.push_str(existing[end..].trim_start_matches('\n'));
        body
    } else {
        existing
    };
    while body.ends_with("\n\n") {
        body.pop();
    }

    if !packages.is_empty() {
        body.push_str(ANDROID_KEEPS_BEGIN);
        body.push('\n');
        for package in packages {
            body.push_str("-keep class ");
            body.push_str(package);
            body.push_str(".** { *; }\n");
        }
        body.push_str(ANDROID_KEEPS_END);
        body.push('\n');
    }

    super::templates::write_file_if_changed(&rules_path, body.as_bytes())
        .await
        .map_err(Into::into)
}

/// Resolves and satisfies font declarations for a build.
///
/// Resolution is [`resolve_declarations`]: same `name` → keep only one font,
/// local > remote > built-in, `Water.toml` ahead of dependencies on a tie.
/// Satisfaction is [`satisfy_font`]: remote and built-in declarations resolve
/// to files already in the font cache — a build performs no network access,
/// so a declaration that is not already cached is an error naming the font,
/// its URL, and `water fetch`.
pub async fn resolve_fonts(
    host: &crate::toolchain::Host,
    declarations: Vec<FontDeclaration>,
) -> eyre::Result<Vec<ResolvedFont>> {
    let cache_dir = cache_dir(host)?;
    let registry = FontRegistry::builtin()?;

    let mut resolved = Vec::new();
    for decl in resolve_declarations(declarations) {
        let path = satisfy_font(host, &decl, &cache_dir, &registry).await?;
        debug!("Resolved font '{}' -> {}", decl.name, path.display());
        resolved.push(ResolvedFont {
            name: decl.name,
            path,
        });
    }

    info!("Resolved {} fonts", resolved.len());
    Ok(resolved)
}

/// Deduplicates font declarations into the set a build or a fetch then
/// satisfies — the resolution half of [`resolve_fonts`], shared so a fetch
/// can never disagree with the build that follows it.
///
/// Rules:
/// - Same `name` → keep only one font
/// - Priority: local > remote > built-in; between equal sources the earlier
///   declaration wins, so `Water.toml` overrides a dependency on a tie
///
/// Winners come out sorted by family name so reports read deterministically.
fn resolve_declarations(declarations: Vec<FontDeclaration>) -> Vec<FontDeclaration> {
    // Group by name
    let mut by_name: HashMap<String, Vec<FontDeclaration>> = HashMap::new();
    for decl in declarations {
        by_name.entry(decl.name.clone()).or_default().push(decl);
    }

    let mut resolved: Vec<FontDeclaration> = by_name
        .into_values()
        .map(|mut decls| {
            // Sort by priority: Local > Remote > BuiltIn, take the first.
            decls.sort_by_key(|d| match &d.source {
                FontSource::Local { .. } => 0,
                FontSource::Remote { .. } => 1,
                FontSource::BuiltIn => 2,
            });
            decls.into_iter().next().unwrap()
        })
        .collect();
    resolved.sort_by(|left, right| left.name.cmp(&right.name));
    resolved
}

/// Satisfies one resolved declaration against crate-local files and the font
/// cache — the build path, which never touches the network.
///
/// A declaration that cannot be satisfied is an error, never a skip:
/// skipping renders the app in whatever face the shaper falls back to, which
/// is the silent wrong-typeface this module exists to rule out.
async fn satisfy_font(
    host: &crate::toolchain::Host,
    decl: &FontDeclaration,
    cache_dir: &Path,
    registry: &FontRegistry,
) -> eyre::Result<PathBuf> {
    let name = &decl.name;
    match &decl.source {
        FontSource::Local {
            crate_root,
            relative_path,
        } => match resolve_local_font_path(crate_root, relative_path) {
            Ok(Some(full_path)) => Ok(full_path),
            // A crate-local font missing at build time means the crate did
            // not package what it declares — say so, with the path that was
            // expected.
            Ok(None) => Err(unsatisfiable_local_font(
                decl,
                &crate_root.join(relative_path),
            )),
            Err(e) => Err(e).wrap_err_with(|| {
                format!(
                    "font '{name}' has an invalid local path '{}' (declared by {})",
                    relative_path.display(),
                    decl.crate_name
                )
            }),
        },
        FontSource::Remote { url } => cached_font(host, name, url, cache_dir).await,
        FontSource::BuiltIn => {
            let Some(FontOrigin { url, .. }) = registry.origin(name) else {
                // Declared by name alone and the registry has no such
                // family: nothing can satisfy it, so this fails here rather
                // than at the first glyph the shaper draws in some other
                // face.
                return Err(unsatisfiable_builtin_font(decl));
            };
            cached_font(host, name, url, cache_dir).await
        }
    }
}

/// The report for a crate-local declaration whose file is absent — used by
/// both the build and `water fetch`, which reports the same thing it cannot
/// fix.
fn unsatisfiable_local_font(decl: &FontDeclaration, expected: &Path) -> eyre::Report {
    eyre::eyre!(
        "font '{}' is declared by {} at '{}', which does not exist in that crate",
        decl.name,
        decl.crate_name,
        expected.display(),
    )
}

/// The report for a by-name declaration the built-in registry does not know —
/// used by both the build and `water fetch`.
fn unsatisfiable_builtin_font(decl: &FontDeclaration) -> eyre::Report {
    eyre::eyre!(
        "font '{}' is declared by {} by name alone, but no font of that name \
         is in the built-in registry — give the declaration a `local_path` or a \
         `remote_path`",
        decl.name,
        decl.crate_name
    )
}

/// Gets the cache directory holding fonts fetched out of band.
fn cache_dir(host: &crate::toolchain::Host) -> eyre::Result<PathBuf> {
    let cache = host
        .cache_dir()
        .map(|root| root.join("waterui").join("fonts"))
        .ok_or_eyre("Could not determine cache directory")?;
    Ok(cache)
}

fn resolve_local_font_path(
    crate_root: &Path,
    relative_path: &Path,
) -> eyre::Result<Option<PathBuf>> {
    let full_path = crate_root.join(relative_path);
    if !full_path.exists() {
        return Ok(None);
    }

    let canonical_root = crate_root
        .canonicalize()
        .wrap_err_with(|| format!("Failed to canonicalize crate root {}", crate_root.display()))?;
    let canonical_path = full_path
        .canonicalize()
        .wrap_err_with(|| format!("Failed to canonicalize font path {}", full_path.display()))?;

    if !canonical_path.starts_with(&canonical_root) {
        eyre::bail!(
            "path escapes crate root ({} -> {})",
            full_path.display(),
            canonical_path.display()
        );
    }

    Ok(Some(canonical_path))
}

/// Resolves a remotely-declared font to a file already in the font cache.
///
/// A build performs no network access: when the declaration is not already
/// cached, this fails naming the font, its URL and the cache directory, so the
/// user can run `water fetch` and retry the build.
async fn cached_font(
    host: &crate::toolchain::Host,
    name: &str,
    url: &str,
    cache_dir: &Path,
) -> eyre::Result<PathBuf> {
    cached_font_entry(host, name, url, cache_dir)
        .await?
        .ok_or_else(|| uncached_font_error(name, url, cache_dir))
}

/// Probes the font cache for the face `url` declares.
///
/// `Some` is exactly the path [`cached_font`] resolves to; `None` means
/// nothing usable is cached and `water fetch` can place it. A zero-length
/// entry is dropped and reported absent.
async fn cached_font_entry(
    host: &crate::toolchain::Host,
    name: &str,
    url: &str,
    cache_dir: &Path,
) -> eyre::Result<Option<PathBuf>> {
    // Use URL hash as filename to avoid conflicts
    let hash = sha256_hex(url);
    if is_zip_url(url) {
        return cached_zip_font(host, name, cache_dir, &hash).await;
    }

    cached_file_font(name, cache_dir, &hash).await
}

fn is_zip_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("zip"))
}

async fn cached_zip_font(
    host: &crate::toolchain::Host,
    name: &str,
    cache_dir: &Path,
    hash: &str,
) -> eyre::Result<Option<PathBuf>> {
    let extract_dir = cache_dir.join(hash);
    if extract_dir.exists() {
        debug!(
            "Font '{}' already extracted at {}",
            name,
            extract_dir.display()
        );
        return find_font_file(&extract_dir, name).await.map(Some);
    }

    let cache_file = cache_dir.join(format!("{hash}.zip"));
    if let Some(cache_len) = cached_file_len(name, &cache_file).await? {
        if cache_len == 0 {
            warn!(
                "Ignoring empty cached font '{}' at {}",
                name,
                cache_file.display()
            );
            let _ = fs::remove_file(&cache_file).await;
        } else {
            debug!("Font '{}' already cached at {}", name, cache_file.display());
            return find_font_in_extracted_zip(host, &cache_file, name)
                .await
                .map(Some);
        }
    }

    Ok(None)
}

async fn cached_file_font(
    name: &str,
    cache_dir: &Path,
    hash: &str,
) -> eyre::Result<Option<PathBuf>> {
    let cache_file = cache_dir.join(format!("{hash}.ttf"));

    if let Some(cache_len) = cached_file_len(name, &cache_file).await? {
        if cache_len == 0 {
            warn!(
                "Ignoring empty cached font '{}' at {}",
                name,
                cache_file.display()
            );
            let _ = fs::remove_file(&cache_file).await;
        } else {
            debug!("Font '{}' already cached at {}", name, cache_file.display());
            return Ok(Some(cache_file));
        }
    }

    Ok(None)
}

/// The error for a remote font declaration that is not already in the font
/// cache. Builds perform no network access, so the download is a separate,
/// opt-in command — the message names the font, the URL it is fetched from,
/// the cache directory, and `water fetch`.
fn uncached_font_error(name: &str, url: &str, cache_dir: &Path) -> eyre::Report {
    eyre::eyre!(
        "font '{name}' is declared remote ({url}) but is not in the font cache at {}; \
         builds never access the network — run `water fetch` to download the project's \
         fonts and retry the build",
        cache_dir.display(),
    )
}

async fn cached_file_len(name: &str, cache_file: &Path) -> eyre::Result<Option<u64>> {
    match fs::metadata(cache_file).await {
        Ok(metadata) => Ok(Some(metadata.len())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).wrap_err_with(|| {
            format!(
                "Failed to read cached font metadata for '{}' at {}",
                name,
                cache_file.display()
            )
        }),
    }
}

/// What seeding the font cache did for one resolved font declaration.
///
/// `water fetch` and `water create` report these; a build never produces
/// them because it never fetches — it turns the same states into errors.
#[derive(Debug)]
pub enum FetchOutcome {
    /// Already satisfied — a crate-local file that exists, or a cache entry
    /// — so nothing was downloaded.
    Satisfied {
        /// Font family name.
        name: String,
        /// The file a build resolves this font to.
        path: PathBuf,
    },
    /// The declaration's URL was downloaded into the font cache.
    Fetched {
        /// Font family name.
        name: String,
        /// The file a build resolves this font to — the `.ttf` cache entry
        /// itself, or the one face extracted out of a downloaded archive.
        path: PathBuf,
    },
    /// No download can satisfy the declaration: a crate-local file that does
    /// not exist, or a family name the built-in registry does not know.
    /// `error` is the same report a build gives it — saying so is the point.
    Unsatisfiable {
        /// Font family name.
        name: String,
        /// What a build reports for this declaration.
        error: eyre::Report,
    },
}

/// Why seeding the font cache failed.
#[derive(Debug, thiserror::Error)]
pub enum SeedFontCacheError {
    /// A generated crate whose manifest a build scans could not be scaffolded.
    #[error("{0:#}")]
    Scaffold(eyre::Report),
    /// The font declarations could not be read, or a declared font could not be fetched.
    #[error("{0:#}")]
    Fonts(eyre::Report),
}

/// Seeds the font cache with every font a build of `project` would demand.
///
/// `water fetch` and the tail of `water create` share this one path:
/// declarations resolve exactly as a build resolves them —
/// `manifest_font_declarations` plus `scan_crate_font_declarations` over
/// the manifest of each crate a build compiles, then `resolve_declarations`
/// — and whatever the cache does not already hold is downloaded into the
/// entry the build then looks for. Builds keep their no-network guarantee;
/// this is the explicit, opt-in network step.
///
/// # Errors
///
/// Returns an error when a backend crate cannot be scaffolded, a manifest
/// cannot be scanned, or a download fails — the error names the backend, or
/// the font and its URL. Declarations fetching can never satisfy arrive as
/// [`FetchOutcome::Unsatisfiable`] instead, carrying the same report the
/// build gives them.
pub async fn seed_font_cache(project: &Project) -> Result<Vec<FetchOutcome>, SeedFontCacheError> {
    seed_font_cache_scoped(project, None).await
}

/// [`seed_font_cache`] restricted to the crates a `backend` build scans.
///
/// `water fetch --backend android` is the caller: it seeds the cache for the
/// Android build alone — the manifest set that build's font resolution reads
/// — so a backend crate no Android build can ever compile is never
/// scaffolded or scanned here.
///
/// # Errors
///
/// Same as [`seed_font_cache`], narrowed to the scoped crate set.
pub async fn seed_font_cache_for_backend(
    project: &Project,
    backend: crate::platform::TargetBackend,
) -> Result<Vec<FetchOutcome>, SeedFontCacheError> {
    seed_font_cache_scoped(project, Some(backend)).await
}

async fn seed_font_cache_scoped(
    project: &Project,
    scope: Option<crate::platform::TargetBackend>,
) -> Result<Vec<FetchOutcome>, SeedFontCacheError> {
    let host = project.host();
    let mut declarations = manifest_font_declarations(project.manifest(), project.root())
        .map_err(SeedFontCacheError::Fonts)?;
    let manifests = ensure_font_scan_manifests(project, scope)
        .await
        .map_err(SeedFontCacheError::Scaffold)?;
    for manifest in manifests {
        declarations.extend(
            scan_crate_font_declarations(project, &manifest)
                .await
                .map_err(SeedFontCacheError::Fonts)?,
        );
    }
    let cache_dir = cache_dir(host).map_err(SeedFontCacheError::Fonts)?;
    fetch_fonts(host, declarations, &cache_dir, download_font)
        .await
        .map_err(SeedFontCacheError::Fonts)
}

/// The crate manifests a build of `project` scans for font declarations —
/// produced before they are scanned, the same way the build that compiles
/// them produces them.
///
/// The project crate is always scanned; beyond it, the manifests that matter
/// are the generated crates': the theme crate's
/// `[package.metadata.waterui.assets.font]` entries are reachable only
/// through the backend manifest that depends on it. Apple and Android builds
/// scan the FFI companion, which the build itself scaffolds — the Apple
/// companion render and the Android `prepare_for_build` prologue — so if it
/// is still absent it is scaffolded here. Each of
/// the GTK4, Hydrolysis and `WinUI` crates is re-scaffolded with the
/// current templates when missing or stale, exactly as the build and preview
/// paths regenerate it. Scaffolding writes template files — nothing
/// compiles.
///
/// The scanned set is the crates for the backends this host can run —
/// Hydrolysis anywhere, GTK4 on Linux, `WinUI` on Windows — since the CLI
/// manages them all. `scope` narrows the set to one backend's
/// crates — the manifests that backend's builds read — so a fetch preparing
/// a single-backend build never touches crates that build cannot compile. A
/// crate that cannot be produced is an error naming its backend — silently
/// dropping it is how a build's font demand and a fetch's scan disagree.
async fn ensure_font_scan_manifests(
    project: &Project,
    scope: Option<crate::platform::TargetBackend>,
) -> eyre::Result<Vec<PathBuf>> {
    let mut manifests = vec![project.root().join("Cargo.toml")];

    // Apple and Android builds scan the FFI companion; no other backend's
    // build does, and a scoped fetch includes it only for those two.
    let ffi_scanned = scope.is_none_or(|backend| {
        matches!(
            backend,
            crate::platform::TargetBackend::Apple | crate::platform::TargetBackend::Android
        )
    });
    if ffi_scanned {
        let manifest = project.ffi_crate_path().join("Cargo.toml");
        // The companion on disk is re-rendered for the scan — the manifest
        // a build produces, so the font declarations the scan sees are the
        // ones the build's resolve carries.
        project.scaffold_ffi_companion().await.map_err(|error| {
            eyre::eyre!("could not scaffold the Apple/Android FFI companion crate: {error}")
        })?;
        manifests.push(manifest);
    }

    ensure_backend_manifest::<crate::gtk4::backend::Gtk4Backend>(project, scope, &mut manifests)
        .await?;
    ensure_backend_manifest::<crate::hydrolysis::backend::HydrolysisBackend>(
        project,
        scope,
        &mut manifests,
    )
    .await?;
    ensure_backend_manifest::<crate::winui::backend::WinUiBackend>(project, scope, &mut manifests)
        .await?;

    Ok(manifests)
}

/// A generated backend crate — GTK4, Hydrolysis or `WinUI` — whose manifest
/// a build scans for font declarations.
trait FontScanCrate: crate::backend::Backend {
    /// The backend's display name.
    const NAME: &'static str;
    /// The backend whose builds scan this crate — a scoped fetch covers the
    /// crate only for that backend.
    const TARGET: crate::platform::TargetBackend;
    /// Whether a build on this host can ever compile this crate: the CLI
    /// manages every backend the host supports.
    fn wanted() -> bool;
    /// Whether `scope` — `water fetch --backend`'s value — selects this
    /// crate: an unscoped fetch covers every wanted crate, a scoped one only
    /// the selected backend's own.
    fn in_scope(scope: Option<crate::platform::TargetBackend>) -> bool {
        scope.is_none_or(|backend| backend == Self::TARGET)
    }
    /// Whether the crate on disk is missing or behind the current templates
    /// — the check the build and preview paths apply before regenerating.
    fn stale(project: &Project) -> impl Future<Output = eyre::Result<bool>> + Send;
}

/// Scaffolds `B`'s crate the way the build that compiles it would —
/// [`crate::backend::reinit_backend`] when it is missing or stale — and
/// pushes its `Cargo.toml` onto `manifests`. A crate that cannot be produced
/// is an error naming the backend, never a skip.
async fn ensure_backend_manifest<B: FontScanCrate>(
    project: &Project,
    scope: Option<crate::platform::TargetBackend>,
    manifests: &mut Vec<PathBuf>,
) -> eyre::Result<()> {
    if !B::in_scope(scope) || !B::wanted() {
        return Ok(());
    }
    let stale = B::stale(project)
        .await
        .wrap_err_with(|| format!("could not inspect the {} backend crate", B::NAME))?;
    if stale {
        crate::backend::reinit_backend::<B>(project)
            .await
            .map_err(|error| {
                eyre::eyre!("could not scaffold the {} backend crate: {error}", B::NAME)
            })?;
    }
    manifests.push(project.backend_path::<B>().join("Cargo.toml"));
    Ok(())
}

impl FontScanCrate for crate::gtk4::backend::Gtk4Backend {
    const NAME: &'static str = "GTK4";
    const TARGET: crate::platform::TargetBackend = crate::platform::TargetBackend::Gtk4;
    fn wanted() -> bool {
        // GTK4 compiles on Linux hosts only.
        cfg!(target_os = "linux")
    }
    async fn stale(project: &Project) -> eyre::Result<bool> {
        Self::requires_regeneration(project).await
    }
}

impl FontScanCrate for crate::hydrolysis::backend::HydrolysisBackend {
    const NAME: &'static str = "hydrolysis";
    const TARGET: crate::platform::TargetBackend = crate::platform::TargetBackend::Hydrolysis;
    fn wanted() -> bool {
        true
    }
    async fn stale(project: &Project) -> eyre::Result<bool> {
        Self::requires_regeneration(project).await
    }
}

impl FontScanCrate for crate::winui::backend::WinUiBackend {
    const NAME: &'static str = "WinUI";
    const TARGET: crate::platform::TargetBackend = crate::platform::TargetBackend::WinUi;
    fn wanted() -> bool {
        // `WinUI` compiles on Windows hosts only.
        cfg!(target_os = "windows")
    }
    async fn stale(project: &Project) -> eyre::Result<bool> {
        Self::requires_regeneration(project).await
    }
}

/// How `fetch_fonts` downloads one URL to a path — a seam a test closes
/// with a stub so it never reaches the network.
type FontFetch =
    for<'a> fn(&'a str, &'a Path) -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send + 'a>>;

/// Fetches into `cache_dir` every font `declarations` resolves to.
///
/// Resolution is the same [`resolve_fonts`] applies —
/// [`resolve_declarations`] — so a fetch can never disagree with the build
/// that follows it, and a declaration already satisfied reports
/// [`FetchOutcome::Satisfied`] without being downloaded again.
///
/// # Errors
///
/// A failed download aborts the run with an error naming the font and its
/// URL; a quiet skip would leave the next build failing on a font this run
/// was supposed to place.
async fn fetch_fonts(
    host: &crate::toolchain::Host,
    declarations: Vec<FontDeclaration>,
    cache_dir: &Path,
    fetch: FontFetch,
) -> eyre::Result<Vec<FetchOutcome>> {
    let registry = FontRegistry::builtin()?;
    let mut outcomes = Vec::new();
    for decl in resolve_declarations(declarations) {
        let outcome = match &decl.source {
            // Whatever fetching cannot fix — a crate-local file that is not
            // there — is reported exactly as the build reports it.
            FontSource::Local { .. } => match satisfy_font(host, &decl, cache_dir, &registry).await
            {
                Ok(path) => FetchOutcome::Satisfied {
                    name: decl.name.clone(),
                    path,
                },
                Err(error) => FetchOutcome::Unsatisfiable {
                    name: decl.name.clone(),
                    error,
                },
            },
            FontSource::Remote { .. } | FontSource::BuiltIn => {
                match declaration_origin(&decl, &registry) {
                    Some(origin) => {
                        if let Some(path) =
                            cached_font_entry(host, &decl.name, origin.url, cache_dir).await?
                        {
                            FetchOutcome::Satisfied {
                                name: decl.name.clone(),
                                path,
                            }
                        } else {
                            let path =
                                fetch_remote_font(host, &decl.name, origin, cache_dir, fetch)
                                    .await?;
                            FetchOutcome::Fetched {
                                name: decl.name.clone(),
                                path,
                            }
                        }
                    }
                    // Only a `BuiltIn` declaration reaches this arm.
                    None => FetchOutcome::Unsatisfiable {
                        name: decl.name.clone(),
                        error: unsatisfiable_builtin_font(&decl),
                    },
                }
            }
        };
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

/// The URL a declaration is fetched from, when it has one: `Remote`
/// declarations carry their own; `BuiltIn` ones resolve through the
/// registry; `Local` ones have none.
fn declaration_origin<'a>(
    decl: &'a FontDeclaration,
    registry: &'a FontRegistry,
) -> Option<FontOrigin<'a>> {
    match &decl.source {
        FontSource::Remote { url } => Some(FontOrigin { url, sha256: None }),
        FontSource::BuiltIn => registry.origin(&decl.name),
        FontSource::Local { .. } => None,
    }
}

/// Downloads one remote font into the cache entry a build looks for.
///
/// `url` lands at `{sha256(url)}.ttf` — or `.zip` for an archive — the same
/// name the build probes, via a `.partial` sibling so an interrupted
/// transfer never reads as a cached font. An archive is extracted the way
/// the build's first resolution extracts it. A download that does not match
/// the digest the origin pins is deleted and refused.
async fn fetch_remote_font(
    host: &crate::toolchain::Host,
    name: &str,
    origin: FontOrigin<'_>,
    cache_dir: &Path,
    fetch: FontFetch,
) -> eyre::Result<PathBuf> {
    let url = origin.url;
    waterui_assets_core::ensure_http_allowed(url)
        .map_err(|error| eyre::eyre!("font '{name}' cannot be fetched from {url}: {error}"))?;
    fs::create_dir_all(cache_dir).await?;

    let hash = sha256_hex(url);
    let extension = if is_zip_url(url) { "zip" } else { "ttf" };
    let cache_file = cache_dir.join(format!("{hash}.{extension}"));
    let partial = cache_dir.join(format!("{hash}.{extension}.partial"));
    fetch(url, &partial)
        .await
        .wrap_err_with(|| format!("font '{name}' could not be fetched from {url}"))?;
    if fs::metadata(&partial).await?.len() == 0 {
        let _ = fs::remove_file(&partial).await;
        eyre::bail!("font '{name}' fetched from {url} is empty");
    }
    if let Some(expected) = origin.sha256 {
        let actual = crate::utils::file_sha256(&partial).await?;
        if actual != expected {
            let _ = fs::remove_file(&partial).await;
            eyre::bail!(
                "font '{name}' fetched from {url} has SHA-256 {actual}, but the registry pins \
                 {expected}; the download was refused"
            );
        }
    }
    fs::rename(&partial, &cache_file).await.wrap_err_with(|| {
        format!(
            "failed to place font '{name}' fetched from {url} at {}",
            cache_file.display()
        )
    })?;

    if is_zip_url(url) {
        find_font_in_extracted_zip(host, &cache_file, name)
            .await
            .wrap_err_with(|| format!("font '{name}' fetched from {url}"))
    } else {
        Ok(cache_file)
    }
}

/// `GET`s `url` into `dest` — the same request shape `framework.rs` and
/// `browser_runtime.rs` send for their own downloads: a `zenwave` client, a
/// `GET` under the crate's user agent, streamed to the path.
fn download_font<'a>(
    url: &'a str,
    dest: &'a Path,
) -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let mut client = zenwave::client();
        client
            .method(Method::GET, url)?
            .header("User-Agent", env!("CARGO_PKG_NAME"))?
            .download_to_path(dest)
            .await?;
        Ok(())
    })
}

/// Finds a font file in an extracted zip archive.
async fn find_font_in_extracted_zip(
    host: &crate::toolchain::Host,
    zip_path: &Path,
    name: &str,
) -> eyre::Result<PathBuf> {
    let extract_dir = zip_path.with_extension("");

    // Extract if not already done
    if !extract_dir.exists() {
        fs::create_dir_all(&extract_dir).await?;

        let zip_path = zip_path.to_path_buf();
        let extract_dir_clone = extract_dir.clone();
        let zip_path_for_extraction = zip_path.clone();

        smol::unblock(move || {
            let file = std::fs::File::open(&zip_path_for_extraction)?;
            let mut archive = zip::ZipArchive::new(file)?;
            archive.extract(&extract_dir_clone)?;
            Ok::<_, eyre::Report>(())
        })
        .await?;

        // Font Awesome's generated Rust bindings require the archive metadata.
        // Other font ZIPs do not contain icons.json and must not be treated as
        // damaged Font Awesome distributions.
        if name.to_ascii_lowercase().contains("fontawesome") {
            copy_fontawesome_icons_json(host, &extract_dir).await?;
        }
    }

    remove_extracted_font_archive(zip_path).await?;

    // Find a font file (.ttf or .otf)
    let font_file = find_font_file(&extract_dir, name).await?;
    Ok(font_file)
}

async fn remove_extracted_font_archive(zip_path: &Path) -> eyre::Result<()> {
    match fs::remove_file(zip_path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).wrap_err_with(|| {
            format!(
                "Failed to remove extracted font archive at {}",
                zip_path.display()
            )
        }),
    }
}

/// Copies Font Awesome icons.json to the fontawesome cache directory.
///
/// This is needed by the fontawesome7 crate's build.rs to generate icon definitions.
async fn copy_fontawesome_icons_json(
    host: &crate::toolchain::Host,
    extract_dir: &Path,
) -> eyre::Result<()> {
    // Look for metadata/icons.json in the extracted archive
    let icons_json = find_file_recursive(extract_dir, "icons.json").await?;

    // Determine version from parent directory name (e.g., "fontawesome-free-7.1.0-desktop")
    let version = extract_fontawesome_version(extract_dir);

    // Copy to fontawesome cache directory
    let fontawesome_cache = host
        .cache_dir()
        .map(|root| root.join("waterui").join("fontawesome"))
        .ok_or_eyre("Could not determine cache directory")?;

    fs::create_dir_all(&fontawesome_cache).await?;

    let dest = fontawesome_cache.join(format!("fontawesome-{version}-icons.json"));
    crate::utils::copy_file_if_changed(&icons_json, &dest).await?;

    debug!("Copied icons.json to {}", dest.display());
    Ok(())
}

/// Recursively finds a file by name in a directory.
async fn find_file_recursive(dir: &Path, filename: &str) -> eyre::Result<PathBuf> {
    let dir = dir.to_path_buf();
    let filename = filename.to_string();
    smol::unblock(move || {
        for entry in WalkDir::new(&dir) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name == filename)
            {
                return Ok(entry.into_path());
            }
        }
        eyre::bail!("File '{}' not found in {}", filename, dir.display());
    })
    .await
}

/// Extracts Font Awesome version from directory structure.
fn extract_fontawesome_version(extract_dir: &Path) -> String {
    // Try to find version from directory names like "fontawesome-free-7.1.0-desktop"
    if let Ok(entries) = std::fs::read_dir(extract_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("fontawesome-") {
                // Extract version: "fontawesome-free-7.1.0-desktop" -> "7.1.0"
                let parts: Vec<&str> = name.split('-').collect();
                for (i, part) in parts.iter().enumerate() {
                    if part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        return parts[i..]
                            .join("-")
                            .split('-')
                            .next()
                            .unwrap_or("7.1.0")
                            .to_string();
                    }
                }
            }
        }
    }
    "7.1.0".to_string() // Default fallback
}

/// Recursively finds a font file in a directory.
///
/// Supports TTF and OTF formats.
async fn find_font_file(dir: &Path, name: &str) -> eyre::Result<PathBuf> {
    let dir = dir.to_path_buf();
    let name = name.to_string();
    smol::unblock(move || {
        let mut candidates = Vec::new();
        let name_lower = name.to_lowercase();

        let style_keyword =
            extract_style_keyword(&name_lower).unwrap_or_else(|| "regular".to_string());

        for entry in WalkDir::new(&dir) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.into_path();
            let Some(ext) = path.extension() else {
                continue;
            };
            let ext = ext.to_string_lossy().to_lowercase();
            if ext != "ttf" && ext != "otf" {
                continue;
            }

            let file_name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            if file_name.contains(&style_keyword) {
                let name_parts: Vec<&str> = name_lower.split_whitespace().collect();
                let matches_base = name_parts
                    .iter()
                    .take(3)
                    .all(|part| file_name.contains(part));
                if matches_base {
                    return Ok(path);
                }
            }

            candidates.push(path);
        }

        candidates
            .into_iter()
            .next()
            .ok_or_else(|| eyre::eyre!("No font file found in zip for '{}'", name))
    })
    .await
}

/// Extract style keyword from font family name for matching.
///
/// Handles patterns like:
/// - "FontAwesome7Free-Solid" -> "solid"
/// - "FontAwesome7Free-Regular" -> "regular"
/// - "FontAwesome7Free-Brands" -> "brands"
fn extract_style_keyword(name: &str) -> Option<String> {
    // Check for common style suffixes
    let styles = [
        "solid", "regular", "brands", "light", "thin", "bold", "medium",
    ];
    for style in styles {
        if name.ends_with(style) || name.contains(&format!("-{style}")) {
            return Some(style.to_string());
        }
    }
    None
}

/// Computes SHA256 hash of a string as hex.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

pub use unified::{build_manifest as plan_library_resources, write_library_resources};

/// The directory name the staged asset bundle carries inside an Android
/// `src/main/assets/` root.
pub use unified::ASSET_ROOT_DIR as ANDROID_ASSET_BUNDLE_DIR;

/// Stage project assets for Apple packaging (Asset Catalog + raw resources).
///
/// `symbols` is the library artifact the target build already produced
/// (`crate::build::BuiltTarget::app_symbols`) — its `waterui_meta_bundle_*`
/// statics declare the `include_bundle!` mounts. Returns the staged manifest
/// so callers can scan it (fonts, for example) without replanning.
pub async fn stage_project_assets_for_apple(
    project: &Project,
    dest_dir: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
) -> eyre::Result<BundleManifest> {
    unified::stage_for_apple(project, dest_dir, symbols, dev_server).await
}

/// Stage project assets for Android packaging (res + assets/raw).
/// `symbols` is the target build's app library — see
/// [`stage_project_assets_for_apple`].
pub async fn stage_project_assets_for_android(
    project: &Project,
    backend_path: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
    theme_parent: unified::AndroidThemeParent,
) -> eyre::Result<BundleManifest> {
    unified::stage_for_android(project, backend_path, symbols, dev_server, theme_parent).await
}

/// Stage project assets for an embedded-mode Android library module
/// (`waterui_assets` + sync stamp only — no app-level `res` files).
/// `symbols` is the target build's app library — see
/// [`stage_project_assets_for_apple`].
///
/// Returns the manifest and the staged `waterui_assets` directory.
pub async fn stage_project_assets_for_android_library(
    project: &Project,
    module_dir: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
) -> eyre::Result<(BundleManifest, PathBuf)> {
    unified::stage_for_android_library(project, module_dir, symbols, dev_server).await
}

/// Render the project's macOS `.icns` app icon for hand-assembled bundles.
///
/// Gated to macOS along with the rest of the `.icns` chain, whose only caller is
/// the macOS packaging path.
#[cfg(target_os = "macos")]
pub fn project_macos_icns(project: &Project) -> eyre::Result<Vec<u8>> {
    unified::macos_icns(project)
}

/// Render the project's Windows `.ico` app icon for resource embedding.
pub fn project_windows_ico(project: &Project) -> eyre::Result<Vec<u8>> {
    unified::windows_ico(project)
}

/// Install the app icon into a hicolor icon-theme tree for Linux desktops.
pub async fn stage_hicolor_icons(project: &Project, icons_root: &Path) -> eyre::Result<()> {
    unified::stage_hicolor_icons(project, icons_root).await
}

/// Stage project assets for GTK4 packaging (resources + gresource bundle).
/// `symbols` is the target build's app library — see
/// [`stage_project_assets_for_apple`].
pub async fn stage_project_assets_for_gtk(
    project: &Project,
    resources_dir: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
) -> eyre::Result<BundleManifest> {
    unified::stage_for_gtk(project, resources_dir, symbols, dev_server).await
}

/// Resolves the fonts declared inside an already-staged bundle manifest.
pub fn scan_project_font_assets(manifest: &BundleManifest) -> eyre::Result<Vec<ResolvedFont>> {
    unified::scan_project_fonts(manifest)
}

pub use unified::{AndroidThemeParent, LaunchAssets};

/// Resolve the project's launch screen and load its artwork.
pub fn project_launch_assets(project: &Project) -> eyre::Result<LaunchAssets> {
    unified::launch_assets(project)
}

/// Stage project assets for web packaging.
pub async fn stage_project_assets_for_web(project: &Project, site_root: &Path) -> eyre::Result<()> {
    web::stage_for_web(project, site_root).await
}

/// Copies fonts to a destination directory.
pub async fn copy_fonts(fonts: &[ResolvedFont], dest: &Path) -> eyre::Result<()> {
    fs::create_dir_all(dest).await?;

    for font in fonts {
        let file_name = font
            .path
            .file_name()
            .ok_or_eyre("Font path has no filename")?;
        let dest_path = dest.join(file_name);

        debug!(
            "Copying font {} -> {}",
            font.path.display(),
            dest_path.display()
        );
        crate::utils::copy_file_if_changed(&font.path, &dest_path).await?;
    }

    Ok(())
}

/// Stage fonts for the Hydrolysis web runtime using the existing CLI font pipeline.
///
/// The web runtime discovers no system fonts — `load_web_fonts` fetches exactly
/// what `waterui-fonts.json` lists and asserts its `default_family` registered
/// — so the bundled set is whatever the project declared through
/// `[[assets.font]]` in `Water.toml` or a dependency's
/// `[package.metadata.waterui.assets.font]` entries. A project that declares no
/// fonts cannot render text on the web at all, so staging fails fast rather
/// than shipping a site that panics on startup.
pub async fn stage_hydrolysis_web_fonts(
    project: &Project,
    backend_path: &Path,
    site_root: &Path,
) -> eyre::Result<()> {
    let mut resolved_fonts = resolve_fonts(
        project.host(),
        scan_fonts(
            project,
            &backend_path.join("Cargo.toml"),
            &[FontPlatform::Web],
        )
        .await?,
    )
    .await?;
    resolved_fonts.sort_by(|left, right| left.name.cmp(&right.name));

    // `default_family` must name a face the manifest actually carries. Roboto
    // stays the default when the project declares it — the same family the
    // runtime's Material baseline was written against — otherwise the first
    // declared family takes the slot.
    let Some(default_family) = resolved_fonts
        .iter()
        .find(|font| font.name == HYDROLYSIS_DEFAULT_FONT_FAMILY)
        .or_else(|| resolved_fonts.first())
    else {
        eyre::bail!(
            "the Hydrolysis web runtime has no system fonts to fall back on, so a web \
             build must bundle at least one declared font; declare one in Water.toml:\n\n\
             \x20   [[assets.font]]\n\x20   name = \"{HYDROLYSIS_DEFAULT_FONT_FAMILY}\"\n\n\
             or through `[package.metadata.waterui.assets.font]` in a dependency's \
             Cargo.toml"
        );
    };
    let default_family = default_family.name.clone();

    let fonts_dest = site_root.join("fonts");
    copy_fonts(&resolved_fonts, &fonts_dest).await?;
    write_font_manifest(&resolved_fonts, &fonts_dest, Some(&default_family)).await?;
    Ok(())
}

/// Write `waterui-fonts.json` beside the font files `copy_fonts` staged:
/// the declared-family → file-name map the consuming runtime loads at
/// bootstrap (`WaterUiFontTable` on Android, `load_web_fonts` on the
/// Hydrolysis web runtime). `default_family` is passed only for web, whose
/// manifest carries the key.
pub async fn write_font_manifest(
    fonts: &[ResolvedFont],
    fonts_dest: &Path,
    default_family: Option<&str>,
) -> eyre::Result<()> {
    let manifest_fonts = fonts
        .iter()
        .map(|font| {
            font.path
                .file_name()
                .ok_or_eyre("font path has no file name")
                .map(|file_name| FontManifestEntry {
                    name: font.name.clone(),
                    file_name: file_name.to_string_lossy().into_owned(),
                })
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    let payload = serde_json::to_vec_pretty(&FontManifest {
        default_family: default_family.map(str::to_string),
        fonts: manifest_fonts,
    })?;
    super::templates::write_file_if_changed(&fonts_dest.join(FONT_MANIFEST_FILE_NAME), &payload)
        .await?;
    Ok(())
}

/// Returns whether `feature` is enabled on `package` in the dependency graph
/// of `build_manifest` resolved for `target` — `cargo metadata
/// --filter-platform <target>`, cached per manifest and triple by the
/// project.
///
/// `build_manifest` is the `Cargo.toml` of the crate the build actually
/// compiles — the generated FFI crate for Apple and Android, the generated
/// backend crate for the self-drawn backends. That crate depends on the app,
/// so its graph carries both the app's authored dependencies and the backend's
/// own; resolving any other manifest misses declarations only the backend's
/// dependencies make.
///
/// Optional `WaterUI` capabilities are cargo features on the FFI crate, and the
/// native backends must compile the matching component only when the app turned
/// that capability on. The resolved graph is the single source of truth for
/// that, so backends never guess from the manifest text.
///
/// # Errors
///
/// Returns an error when `cargo metadata` cannot resolve the manifest.
pub async fn package_feature_enabled(
    project: &Project,
    build_manifest: &Path,
    package: &str,
    feature: &str,
    target: &Triple,
) -> eyre::Result<bool> {
    let enabled = project
        .generated_manifest_features(build_manifest, target)
        .await
        .wrap_err_with(|| {
            format!(
                "Failed to run cargo metadata on {}",
                build_manifest.display()
            )
        })?
        .get(package)
        .is_some_and(|features| features.contains(feature));
    debug!("resolved feature {package}/{feature} for {target}: {enabled}");
    Ok(enabled)
}

/// An optional `WaterUI` capability, and what in the app's resolved graph says
/// the app has it.
///
/// The name is also the feature the FFI crate exports the capability's C
/// surface under, so one entry drives both the FFI build and the native
/// backend's conditional compilation.
struct Capability {
    /// The capability's name, shared with `waterui-ffi`'s feature of the same
    /// name.
    name: &'static str,
    /// The crate that actually provides the capability.
    package: &'static str,
    /// The feature on that crate that carries it, or `None` when depending on
    /// the crate at all is the opt-in.
    feature: Option<&'static str>,
}

/// Optional `WaterUI` capabilities, each keyed on the crate that actually
/// provides it.
///
/// The source of truth is the *providing* crate, never a facade toggle. An app
/// that sets `waterui = { default-features = false }` can still pull the GPU
/// stack in through a side door — every SVG icon pack renders through
/// `waterui-svg`, which enables `waterui-graphics/gpu` on its own — and such an
/// app emits `GpuSurface` views at runtime. Keying on the facade's `gpu` there
/// pruned the FFI exports and the native backend while the authoring layer kept
/// producing GPU views, which is a guaranteed panic on first render. Reading
/// the resolved graph's `waterui-graphics/gpu` instead makes the exported
/// surface follow what the app can actually express.
///
/// `gpu` is default-on for the facade, but the generated FFI crate must set
/// `default-features = false` (the `c-api` and `android-jni` ABIs are mutually
/// exclusive), which drops it. Forwarding it here is what keeps the GPU C
/// surface present for apps whose graphs carry the GPU stack.
///
/// `map` has no feature at all: `waterui-map` is a component crate an app
/// depends on directly, exactly like an icon pack or a browser engine, so
/// linking it *is* the opt-in.
const OPTIONAL_CAPABILITIES: &[Capability] = &[
    Capability {
        name: "gpu",
        package: "waterui-graphics",
        feature: Some("gpu"),
    },
    Capability {
        name: "map",
        package: "waterui-map",
        feature: None,
    },
    // The `Video`/`Media` playback FFI surface and the `waterkit_audio`
    // keep-alive behind it. `waterui-video` is an optional dependency of the
    // facade (`media`/`video` features), so linking it *is* the opt-in — an app
    // that never plays media stops rooting the codec/streaming graph through
    // `waterui_video_*` exports.
    Capability {
        name: "media",
        package: "waterui-video",
        feature: None,
    },
    // The `WebView` FFI surface (bridge script, JS replies, cookie jar).
    // `waterui-webview` is optional on the facade and the browser-cef crate
    // reaches it through its own `webview` feature, so a plain `links` check
    // covers both entry points.
    Capability {
        name: "webview",
        package: "waterui-webview",
        feature: None,
    },
];

/// Returns whether this app's resolved graph carries the named capability.
///
/// This is the one predicate every consumer of a capability must share: the
/// FFI build forwards the capability's feature, and the native backend build
/// compiles the matching components, from this same answer. The FFI features
/// are passed on the build command line rather than written into the
/// generated manifest, so re-resolving `waterui-ffi`'s own features from the
/// manifest graph would always read them as off — the backend then prunes
/// components whose symbols the dylib does export.
///
/// `build_manifest` is the manifest of the crate being built — the resolved
/// graph the feature check reads — and `target` the triple that build
/// resolves it for: the feature branch runs `cargo metadata
/// --filter-platform` through the project's host, cached per manifest and
/// target, and the link branch reads the application's own `cargo tree
/// --target` evaluation.
///
/// # Errors
///
/// Returns an error when `cargo metadata` cannot be read.
pub async fn capability_enabled(
    project: &Project,
    build_manifest: &Path,
    capability: &str,
    target: &Triple,
) -> eyre::Result<bool> {
    let capability = OPTIONAL_CAPABILITIES
        .iter()
        .find(|candidate| candidate.name == capability)
        .unwrap_or_else(|| panic!("unknown WaterUI capability: {capability}"));
    match capability.feature {
        Some(feature) => {
            seed_managed_crate_lock(project, build_manifest).await?;
            package_feature_enabled(project, build_manifest, capability.package, feature, target)
                .await
        }
        None => {
            project
                .links_runtime_package(target, capability.package)
                .await
        }
    }
}

/// Returns the generated FFI crate's features to enable for this app's
/// capabilities.
///
/// Each entry is a feature the generated manifest forwards to `waterui-ffi`
/// of the same name (`FORWARDED_FFI_FEATURES`), keeping the resolve inside
/// the seeded lockfile.
///
/// An app opts into a capability through its dependency graph — a component
/// crate it depends on (`waterui-map`), or a crate that carries the capability
/// with it (an SVG icon pack carries `waterui-graphics/gpu`). The generated FFI
/// crate is what exports that capability's C surface, so the resolved graph's
/// choice has to reach its build; reading it back out keeps one declaration in
/// the app's manifest.
///
/// `build_manifest` is the manifest of the crate being built — the FFI
/// companion for Apple and Android builds — and `target` the triple that
/// build resolves its graph for.
///
/// # Errors
///
/// Returns an error when `cargo metadata` cannot be read.
pub async fn capability_ffi_features(
    project: &Project,
    build_manifest: &Path,
    target: &Triple,
) -> eyre::Result<Vec<String>> {
    let mut features = Vec::new();
    for capability in OPTIONAL_CAPABILITIES {
        if capability_enabled(project, build_manifest, capability.name, target).await? {
            features.push(capability.name.to_string());
        }
    }
    Ok(features)
}

/// Returns the generated FFI crate features — forwarded to `waterui-ffi` —
/// that select `WaterUI`'s own realizations of the semantic components the
/// facade carries, for a platform with no native primitive to bridge.
///
/// Apple bridges `AVPlayer`, so an Apple build asks for none of these and links
/// no player. Every other platform draws the video itself, and the
/// application's composition root — `waterui::app::App` — is what installs it,
/// so the choice travels as a facade feature rather than as a backend
/// dependency. The realization is opt-in: linking it pulls decoders such as
/// rav1d and symphonia into the artifact, so the FFI build selects it only when
/// the application declared `waterui`'s `video-gpu` feature (or the
/// `waterui-video-gpu` crate) in its own dependency graph.
///
/// Realizations that live in their own crates — `waterui-map-gpu` — are not
/// here. The application depends on such a crate directly and installs it from
/// its own `app(env)`, the way it installs a browser engine, so no build flag
/// selects it.
///
/// `build_manifest` is the manifest of the crate being built — the FFI
/// companion whose `video` feature this list feeds — and `target` the
/// triple that build resolves its graph for.
///
/// # Errors
///
/// Returns an error when `cargo metadata` cannot be read.
pub async fn self_drawn_realization_features(
    project: &Project,
    build_manifest: &Path,
    target: &Triple,
) -> eyre::Result<Vec<String>> {
    let mut features = Vec::new();
    seed_managed_crate_lock(project, build_manifest).await?;
    let opted_in = package_feature_enabled(project, build_manifest, "waterui", "video-gpu", target)
        .await?
        || project
            .links_runtime_package(target, "waterui-video-gpu")
            .await?;
    if opted_in {
        features.push("video".to_string());
    }
    Ok(features)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// A `FontFetch` stub for declarations that must never be downloaded.
    fn must_not_download<'a>(
        _url: &'a str,
        _dest: &'a Path,
    ) -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send + 'a>> {
        panic!("this declaration has no download to perform")
    }

    /// A `FontFetch` stub that writes a fake font file to `dest`.
    fn place_font<'a>(
        url: &'a str,
        dest: &'a Path,
    ) -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send + 'a>> {
        assert_eq!(url, "https://example.com/inter.ttf");
        let dest = dest.to_path_buf();
        Box::pin(async move {
            std::fs::write(dest, b"font-bytes")?;
            Ok(())
        })
    }

    /// A `FontFetch` stub that writes bytes no registry digest matches.
    fn place_unpinned_bytes<'a>(
        _url: &'a str,
        dest: &'a Path,
    ) -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send + 'a>> {
        let dest = dest.to_path_buf();
        Box::pin(async move {
            std::fs::write(dest, b"not-the-pinned-font")?;
            Ok(())
        })
    }

    /// A `FontFetch` stub whose download always fails.
    fn fail_download<'a>(
        _url: &'a str,
        _dest: &'a Path,
    ) -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send + 'a>> {
        Box::pin(async { Err(eyre::eyre!("connection refused")) })
    }

    fn manifest_with_fonts(toml_fonts: &str) -> crate::project::Manifest {
        let mut document = toml_fonts
            .parse::<toml_edit::DocumentMut>()
            .expect("the font fixture parses as TOML");
        document["package"]["name"] = toml_edit::value("Demo");
        document["package"]["bundle_identifier"] = toml_edit::value("dev.example.demo");
        toml::from_str(&document.to_string()).expect("manifest parses")
    }

    #[test]
    fn water_toml_font_with_a_name_alone_uses_the_registry() {
        let manifest = manifest_with_fonts("[[assets.font]]\nname = \"Inter\"");
        let declarations =
            manifest_font_declarations(&manifest, Path::new("/project")).expect("declarations");
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].name, "Inter");
        assert_eq!(declarations[0].crate_name, "Demo");
        assert!(matches!(declarations[0].source, FontSource::BuiltIn));
    }

    #[test]
    fn water_toml_font_platforms_scope_the_builds_that_bundle_it() {
        let manifest =
            manifest_with_fonts("[[assets.font]]\nname = \"Inter\"\nplatforms = [\"web\"]");
        let declarations =
            manifest_font_declarations(&manifest, Path::new("/project")).expect("declarations");
        assert!(declarations[0].bundled_on(FontPlatform::Web));
        assert!(!declarations[0].bundled_on(FontPlatform::Ios));
    }

    #[test]
    fn water_toml_font_with_an_unknown_platform_does_not_parse() {
        let mut document = "[[assets.font]]\nname = \"Inter\"\nplatforms = [\"browser\"]"
            .parse::<toml_edit::DocumentMut>()
            .expect("the font fixture parses as TOML");
        document["package"]["name"] = toml_edit::value("Demo");
        document["package"]["bundle_identifier"] = toml_edit::value("dev.example.demo");
        let error = toml::from_str::<crate::project::Manifest>(&document.to_string())
            .expect_err("an unknown platform must not parse");
        assert!(error.to_string().contains("browser"), "{error}");
    }

    #[test]
    fn water_toml_font_local_path_resolves_against_the_project_root() {
        let manifest = manifest_with_fonts(
            "[[assets.font]]\nname = \"My Font\"\nlocal_path = \"fonts/my.ttf\"",
        );
        let declarations =
            manifest_font_declarations(&manifest, Path::new("/project")).expect("declarations");
        let FontSource::Local {
            crate_root,
            relative_path,
        } = &declarations[0].source
        else {
            panic!("expected a local font source");
        };
        assert_eq!(crate_root, Path::new("/project"));
        assert_eq!(relative_path, Path::new("fonts/my.ttf"));
    }

    #[test]
    fn water_toml_font_rejects_conflicting_sources() {
        let manifest = manifest_with_fonts(
            "[[assets.font]]\nname = \"X\"\nlocal_path = \"a.ttf\"\nremote_path = \"https://x\"",
        );
        assert!(manifest_font_declarations(&manifest, Path::new("/project")).is_err());
    }

    #[test]
    fn water_toml_font_rejects_an_absolute_local_path() {
        let manifest =
            manifest_with_fonts("[[assets.font]]\nname = \"X\"\nlocal_path = \"/abs/x.ttf\"");
        assert!(manifest_font_declarations(&manifest, Path::new("/project")).is_err());
    }

    #[test]
    fn a_manifest_without_assets_declares_no_fonts() {
        let manifest = manifest_with_fonts("");
        assert!(
            manifest_font_declarations(&manifest, Path::new("/project"))
                .expect("declarations")
                .is_empty()
        );
    }

    /// A remote declaration that is not already cached must fail naming the
    /// font, the URL, and the cache directory — and name `water fetch`, the
    /// command that downloads it, since builds never access the network.
    #[test]
    fn an_uncached_remote_font_names_the_font_url_and_cache_dir() {
        let cache_dir = tempdir().expect("temp cache dir");
        let error = smol::block_on(cached_font(
            &crate::toolchain::Host::current(),
            "Inter",
            "https://example.com/inter.ttf",
            cache_dir.path(),
        ))
        .expect_err("an uncached remote font must surface as an error");
        let message = error.to_string();
        assert!(message.contains("Inter"), "{message}");
        assert!(
            message.contains("https://example.com/inter.ttf"),
            "{message}"
        );
        assert!(
            message.contains(&cache_dir.path().display().to_string()),
            "{message}"
        );
        assert!(message.contains("water fetch"), "{message}");
    }

    /// `water fetch` downloads a missing remote font into the cache entry
    /// the build looks for — `{sha256(url)}.ttf` — and the build's own probe
    /// then resolves it. The downloader is a stub: a test never reaches the
    /// real network.
    #[test]
    fn fetch_places_a_missing_remote_font_where_the_build_looks() {
        let cache_dir = tempdir().expect("temp cache dir");
        let url = "https://example.com/inter.ttf";
        let expected = cache_dir.path().join(format!("{}.ttf", sha256_hex(url)));

        let outcomes = smol::block_on(fetch_fonts(
            &crate::toolchain::Host::current(),
            vec![FontDeclaration {
                name: "Inter".to_string(),
                source: FontSource::Remote {
                    url: url.to_string(),
                },
                crate_name: "some-theme".to_string(),
                platforms: None,
            }],
            cache_dir.path(),
            place_font,
        ))
        .expect("fetch succeeds");

        let [FetchOutcome::Fetched { name, path }] = outcomes.as_slice() else {
            panic!("expected one Fetched outcome, got {outcomes:?}");
        };
        assert_eq!(name, "Inter");
        assert_eq!(
            path, &expected,
            "the fetched font lands at the cache entry the build probes"
        );
        assert_eq!(
            fs::read(&expected).expect("read cached font"),
            b"font-bytes"
        );

        let resolved = smol::block_on(cached_font(
            &crate::toolchain::Host::current(),
            "Inter",
            url,
            cache_dir.path(),
        ))
        .expect("the build resolves the font the fetch placed");
        assert_eq!(resolved, expected);
    }

    /// `water fetch` is a no-op for a font that is already cached: the
    /// downloader is never invoked and the outcome reports it satisfied.
    #[test]
    fn fetch_is_a_no_op_for_a_font_already_in_the_cache() {
        let cache_dir = tempdir().expect("temp cache dir");
        let url = "https://example.com/inter.ttf";
        let cached = cache_dir.path().join(format!("{}.ttf", sha256_hex(url)));
        fs::write(&cached, b"cached-font").expect("seed the cache entry");

        let outcomes = smol::block_on(fetch_fonts(
            &crate::toolchain::Host::current(),
            vec![FontDeclaration {
                name: "Inter".to_string(),
                source: FontSource::Remote {
                    url: url.to_string(),
                },
                crate_name: "some-theme".to_string(),
                platforms: None,
            }],
            cache_dir.path(),
            must_not_download,
        ))
        .expect("fetch succeeds");

        let [FetchOutcome::Satisfied { name, path }] = outcomes.as_slice() else {
            panic!("expected one Satisfied outcome, got {outcomes:?}");
        };
        assert_eq!(name, "Inter");
        assert_eq!(path, &cached);
    }

    /// A failed download is an error naming the font and its URL — never a
    /// quiet skip that leaves the next build failing on the same font.
    #[test]
    fn a_failed_download_is_an_error_naming_the_font_and_url() {
        let cache_dir = tempdir().expect("temp cache dir");

        let error = smol::block_on(fetch_fonts(
            &crate::toolchain::Host::current(),
            vec![FontDeclaration {
                name: "Inter".to_string(),
                source: FontSource::Remote {
                    url: "https://example.com/inter.ttf".to_string(),
                },
                crate_name: "some-theme".to_string(),
                platforms: None,
            }],
            cache_dir.path(),
            fail_download,
        ))
        .expect_err("a failed download must be an error");
        let message = format!("{error:#}");
        assert!(message.contains("Inter"), "{message}");
        assert!(
            message.contains("https://example.com/inter.ttf"),
            "{message}"
        );
        assert!(message.contains("connection refused"), "{message}");
    }

    /// A registry font whose download does not match the digest the registry
    /// pins is refused, and nothing is left where the build looks.
    #[test]
    fn a_registry_download_with_the_wrong_digest_is_refused() {
        let cache_dir = tempdir().expect("temp cache dir");

        let error = smol::block_on(fetch_fonts(
            &crate::toolchain::Host::current(),
            vec![FontDeclaration {
                name: "Noto Color Emoji".to_string(),
                source: FontSource::BuiltIn,
                crate_name: "some-app".to_string(),
                platforms: None,
            }],
            cache_dir.path(),
            place_unpinned_bytes,
        ))
        .expect_err("a digest mismatch must be an error");
        let message = format!("{error:#}");
        assert!(message.contains("Noto Color Emoji"), "{message}");
        assert!(message.contains("SHA-256"), "{message}");
        assert_eq!(
            std::fs::read_dir(cache_dir.path())
                .expect("read the cache dir")
                .count(),
            0,
            "a refused download leaves nothing in the cache"
        );
    }

    /// A font declared by a name the registry does not know is reported
    /// exactly as the build reports it — fetching cannot fix it.
    #[test]
    fn fetch_reports_an_unknown_builtin_name_the_way_the_build_does() {
        let cache_dir = tempdir().expect("temp cache dir");

        let outcomes = smol::block_on(fetch_fonts(
            &crate::toolchain::Host::current(),
            vec![FontDeclaration {
                name: "No Such Family".to_string(),
                source: FontSource::BuiltIn,
                crate_name: "some-theme".to_string(),
                platforms: None,
            }],
            cache_dir.path(),
            must_not_download,
        ))
        .expect("an unsatisfiable declaration is an outcome, not a fetch error");

        let [FetchOutcome::Unsatisfiable { name, error }] = outcomes.as_slice() else {
            panic!("expected one Unsatisfiable outcome, got {outcomes:?}");
        };
        assert_eq!(name, "No Such Family");
        let message = format!("{error:#}");
        assert!(message.contains("No Such Family"), "{message}");
        assert!(message.contains("some-theme"), "{message}");
        assert!(message.contains("built-in registry"), "{message}");
    }

    /// A crate-local font that is missing is reported exactly as the build
    /// reports it — fetching cannot fix it, and saying so is the point.
    #[test]
    fn fetch_reports_a_missing_local_font_the_way_the_build_does() {
        let cache_dir = tempdir().expect("temp cache dir");
        let root = tempdir().expect("temp root");

        let outcomes = smol::block_on(fetch_fonts(
            &crate::toolchain::Host::current(),
            vec![FontDeclaration {
                name: "Roboto".to_string(),
                source: FontSource::Local {
                    crate_root: root.path().to_path_buf(),
                    relative_path: PathBuf::from("assets/fonts/Roboto-Variable.ttf"),
                },
                crate_name: "some-theme".to_string(),
                platforms: None,
            }],
            cache_dir.path(),
            must_not_download,
        ))
        .expect("an unsatisfiable declaration is an outcome, not a fetch error");

        let [FetchOutcome::Unsatisfiable { name, error }] = outcomes.as_slice() else {
            panic!("expected one Unsatisfiable outcome, got {outcomes:?}");
        };
        assert_eq!(name, "Roboto");
        let message = format!("{error:#}");
        assert!(message.contains("Roboto"), "{message}");
        assert!(message.contains("Roboto-Variable.ttf"), "{message}");
    }

    #[test]
    fn test_font_registry_has_entries() {
        let registry = FontRegistry::builtin().expect("registry parses");
        assert!(!registry.fonts.is_empty());
        assert!(registry.origin("Inter").is_some());
        assert!(registry.origin("Roboto").is_some());
        assert!(registry.origin("Noto Sans CJK SC").is_some());
        assert!(registry.origin("Noto Color Emoji").is_some());
        // Every entry pins the digest of what its URL serves.
        assert!(registry.fonts.iter().all(|font| font.sha256.len() == 64
            && font.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())));
        // Icon pack fonts should NOT be in the built-in registry
        assert!(
            !registry
                .fonts
                .iter()
                .any(|font| font.name.contains("Font Awesome"))
        );
        assert!(
            !registry
                .fonts
                .iter()
                .any(|font| font.name.contains("Material Design"))
        );
    }

    /// The registry must offer a face carrying an OpenType `MATH` table, and it
    /// must be reached through the archive code path.
    #[test]
    fn font_registry_offers_a_math_face() {
        let registry = FontRegistry::builtin().expect("registry parses");
        let origin = registry
            .origin("STIX Two Math")
            .expect("registry must offer an OpenType MATH font");
        assert!(
            is_zip_url(origin.url),
            "the math font must resolve through the archive path so the single \
             matching face is extracted rather than the whole distribution"
        );
    }

    /// `STIX Two Math` ships in the same archive as eight `STIX Two Text` faces,
    /// none of which carry a `MATH` table. Picking a Text face would leave
    /// formula layout with no table to read, and nothing downstream would say
    /// so — the font would simply be there and be useless.
    #[test]
    fn math_face_wins_over_its_text_siblings_in_the_same_archive() {
        let extracted = tempdir().expect("temp dir");
        let root = extracted.path().join("static_otf");
        fs::create_dir_all(&root).expect("create extract dir");
        for face in [
            "STIXTwoMath-Regular.otf",
            "STIXTwoText-Bold.otf",
            "STIXTwoText-BoldItalic.otf",
            "STIXTwoText-Italic.otf",
            "STIXTwoText-Medium.otf",
            "STIXTwoText-MediumItalic.otf",
            "STIXTwoText-Regular.otf",
            "STIXTwoText-SemiBold.otf",
            "STIXTwoText-SemiBoldItalic.otf",
        ] {
            fs::write(root.join(face), []).expect("write face");
        }

        let selected = smol::block_on(find_font_file(extracted.path(), "STIX Two Math"))
            .expect("math face must be found");

        assert_eq!(
            selected.file_name().expect("selected face has a name"),
            "STIXTwoMath-Regular.otf",
            "selected {} instead of the Math face",
            selected.display()
        );
    }

    #[test]
    fn test_sha256_hex() {
        let hash = sha256_hex("hello");
        assert_eq!(hash.len(), 64); // SHA256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn test_http_allowlist() {
        for url in [
            "http://localhost/font.ttf",
            "http://127.0.0.1:8080/font.ttf",
            "http://[::1]/font.ttf",
            "https://example.com/font.ttf",
        ] {
            assert!(
                waterui_assets_core::ensure_http_allowed(url).is_ok(),
                "expected to allow {url}"
            );
        }
    }

    #[test]
    fn test_http_rejects_non_loopback_and_prefix_bypass() {
        for url in [
            "http://example.com/font.ttf",
            "http://localhost.evil.com/font.ttf",
            "http://127.0.0.1.evil.com/font.ttf",
        ] {
            assert!(
                waterui_assets_core::ensure_http_allowed(url).is_err(),
                "expected to reject {url}"
            );
        }
    }

    #[test]
    fn zip_font_cache_uses_extracted_directory_without_archive() {
        let cache_dir = tempdir().expect("temp cache dir");
        let url = "https://example.com/inter.zip";
        let extracted_dir = cache_dir.path().join(sha256_hex(url));
        fs::create_dir_all(&extracted_dir).expect("create extracted dir");
        let extracted_font = extracted_dir.join("inter-regular.ttf");
        fs::write(&extracted_font, b"font").expect("write extracted font");

        let resolved = smol::block_on(cached_font(
            &crate::toolchain::Host::current(),
            "Inter",
            url,
            cache_dir.path(),
        ))
        .expect("reuse extracted cache");

        assert_eq!(resolved, extracted_font);
    }

    /// A crate that declares a font it does not ship must stop the build.
    /// Skipping it renders the app in whatever face the shaper falls back to
    /// — the silent wrong typeface, which is worse than a failed build
    /// because nothing in the output says the declaration went unmet.
    #[test]
    fn a_declared_local_font_that_is_missing_fails_the_build() {
        let root = tempdir().expect("temp root");
        let error = smol::block_on(resolve_fonts(
            &crate::toolchain::Host::current(),
            vec![FontDeclaration {
                name: "Roboto".to_string(),
                source: FontSource::Local {
                    crate_root: root.path().to_path_buf(),
                    relative_path: PathBuf::from("assets/fonts/Roboto-Variable.ttf"),
                },
                crate_name: "some-theme".to_string(),
                platforms: None,
            }],
        ))
        .expect_err("a missing crate-local font must be an error");

        let message = format!("{error:#}");
        assert!(
            message.contains("Roboto") && message.contains("some-theme"),
            "the error must name the font and the crate that declared it: {message}"
        );
        assert!(
            message.contains("Roboto-Variable.ttf"),
            "the error must name the path that was expected: {message}"
        );
    }

    #[test]
    fn test_resolve_local_font_path_rejects_escape() {
        let root = tempdir().expect("temp root");
        let outside = tempdir().expect("temp outside");
        let outside_font = outside.path().join("outside.ttf");
        fs::write(&outside_font, b"font").expect("write outside font");

        let rel_escape = Path::new("..").join(outside.path().file_name().expect("outside name"));
        let rel_escape = rel_escape.join("outside.ttf");

        let result = resolve_local_font_path(root.path(), &rel_escape);
        assert!(result.is_err(), "expected path traversal to be rejected");
    }

    #[test]
    fn test_resolve_local_font_path_accepts_inside_root() {
        let root = tempdir().expect("temp root");
        let inside_dir = root.path().join("fonts");
        fs::create_dir_all(&inside_dir).expect("create fonts dir");
        let font_path = inside_dir.join("inside.ttf");
        fs::write(&font_path, b"font").expect("write inside font");

        let resolved = resolve_local_font_path(root.path(), Path::new("fonts/inside.ttf"))
            .expect("resolve should succeed")
            .expect("font should exist");
        assert_eq!(resolved, font_path.canonicalize().expect("canonical path"));
    }

    /// The manifest the Hydrolysis web runtime fetches carries
    /// `default_family` first and one `{name, file_name}` entry per bundled
    /// face — the writer must serialize exactly this shape, since the
    /// runtime parses what it fetches.
    #[test]
    fn test_write_font_manifest_serializes_the_web_shape() {
        let dir = tempdir().expect("temp dir");
        let fonts = vec![
            ResolvedFont {
                name: "Roboto".to_string(),
                path: dir.path().join("Roboto-Regular.ttf"),
            },
            ResolvedFont {
                name: "Fixture Sans".to_string(),
                path: dir.path().join("FixtureSans.ttf"),
            },
        ];
        smol::block_on(write_font_manifest(&fonts, dir.path(), Some("Roboto")))
            .expect("manifest writes");
        let written =
            std::fs::read(dir.path().join(FONT_MANIFEST_FILE_NAME)).expect("manifest file exists");
        let expected = r#"{
  "default_family": "Roboto",
  "fonts": [
    {
      "name": "Roboto",
      "file_name": "Roboto-Regular.ttf"
    },
    {
      "name": "Fixture Sans",
      "file_name": "FixtureSans.ttf"
    }
  ]
}"#;
        assert_eq!(written, expected.as_bytes());
    }

    /// The same writer omits `default_family` for Android, whose font table
    /// reads only the entries.
    #[test]
    fn test_write_font_manifest_omits_default_family_for_android() {
        let dir = tempdir().expect("temp dir");
        let fonts = vec![ResolvedFont {
            name: "DejaVuSerif".to_string(),
            path: dir.path().join("DejaVuSerif.ttf"),
        }];
        smol::block_on(write_font_manifest(&fonts, dir.path(), None)).expect("manifest writes");
        let written = std::fs::read_to_string(dir.path().join(FONT_MANIFEST_FILE_NAME))
            .expect("manifest file exists");
        assert!(!written.contains("default_family"), "{written}");
        assert!(written.contains("\"name\": \"DejaVuSerif\""), "{written}");
        assert!(
            written.contains("\"file_name\": \"DejaVuSerif.ttf\""),
            "{written}"
        );
    }
}

/// Collects the permissions this app's dependencies declare they need.
///
/// A crate states its own requirement in its manifest, so the CLI never has to
/// know what any component does:
///
/// ```toml
/// [package.metadata.waterui.permissions]
/// internet = { reason = "downloads map styles and vector tiles" }
/// ```
///
/// Declarations gated behind a cargo feature are skipped unless the resolved
/// graph actually enabled that feature for the declaring crate.
///
/// The scan runs `cargo metadata` on `build_manifest` — the `Cargo.toml` of
/// the crate this build compiles, i.e. the generated backend or FFI crate.
/// That crate depends on the app, so its graph carries both the app's authored
/// dependencies and the backend's own (a theme crate and friends); resolving
/// the app's manifest instead would miss the backend's declarations entirely.
///
/// # Errors
///
/// Returns an error when `cargo metadata` cannot be read.
pub async fn scan_required_permissions(
    host: &crate::toolchain::Host,
    build_manifest: &Path,
) -> eyre::Result<Vec<RequiredPermission>> {
    let metadata = crate_metadata(host, build_manifest, &[])
        .await
        .wrap_err_with(|| {
            format!(
                "Failed to run cargo metadata on {}",
                build_manifest.display()
            )
        })?;

    let enabled_features = resolved_features(&metadata);

    let mut required = Vec::new();
    for package in &metadata.packages {
        let Some(parsed) = parse_waterui_metadata(package)? else {
            continue;
        };
        let features = enabled_features
            .get(&package.id)
            .cloned()
            .unwrap_or_default();
        for (key, requirement) in parsed.permissions {
            if let Some(gate) = &requirement.required_feature
                && !features.contains(gate.as_str())
            {
                debug!(
                    "Skipping {key:?} for {}: feature `{gate}` is not enabled",
                    package.name
                );
                continue;
            }
            required.push(RequiredPermission {
                package: package.name.to_string(),
                key,
                reason: requirement.reason.clone(),
                evidence: PermissionEvidence::Declared,
            });
        }
    }
    if let Some(inferred) = infer_internet_from_http_clients(&metadata.packages, &required) {
        required.push(inferred);
    }
    required.sort_by(|left, right| {
        (left.key, left.package.as_str()).cmp(&(right.key, right.package.as_str()))
    });
    required.dedup();
    Ok(required)
}

/// HTTP client crates whose presence almost always means the app talks to the
/// network at runtime. Presence is a hint, not proof — a client can sit behind
/// a disabled feature of some dependency — so hits are reported as
/// [`PermissionEvidence::Inferred`].
const HTTP_CLIENT_CRATES: &[&str] = &[
    "attohttpc",
    "curl",
    "hyper",
    "isahc",
    "reqwest",
    "surf",
    "ureq",
    "zenwave",
];

/// Suggests the `internet` permission when the dependency graph contains a
/// known HTTP client and nothing declared that permission outright.
///
/// A declared requirement always carries better evidence and a better message,
/// so the inference stays quiet as soon as one exists.
fn infer_internet_from_http_clients(
    packages: &[cargo_metadata::Package],
    declared: &[RequiredPermission],
) -> Option<RequiredPermission> {
    if declared
        .iter()
        .any(|requirement| requirement.key == PermissionKey::Internet)
    {
        return None;
    }
    let mut clients: Vec<&str> = packages
        .iter()
        .map(|package| package.name.as_str())
        .filter(|name| HTTP_CLIENT_CRATES.contains(name))
        .collect();
    clients.sort_unstable();
    clients.dedup();
    if clients.is_empty() {
        return None;
    }
    Some(RequiredPermission {
        package: clients.join(", "),
        key: PermissionKey::Internet,
        reason: String::from("the dependency graph contains an HTTP client"),
        evidence: PermissionEvidence::Inferred,
    })
}

/// Selects the declared requirements this app has not satisfied.
///
/// `relevant` decides whether a permission means anything on the platform being
/// built, so an iOS build stays quiet about `internet` (which iOS never
/// declares) while an Android build does not. Keeping this separate from the
/// reporting makes the selection itself testable.
fn missing_permissions<'a>(
    enabled: &HashSet<PermissionKey>,
    required: &'a [RequiredPermission],
    relevant: impl Fn(PermissionKey) -> bool,
) -> Vec<&'a RequiredPermission> {
    required
        .iter()
        .filter(|requirement| !enabled.contains(&requirement.key) && relevant(requirement.key))
        .collect()
}

/// Reports dependencies that need a permission this app has not enabled.
pub fn warn_missing_permissions(
    project: &Project,
    required: &[RequiredPermission],
    relevant: impl Fn(PermissionKey) -> bool,
) {
    let enabled: HashSet<PermissionKey> = project
        .manifest()
        .permissions
        .iter()
        .filter(|(_, entry)| entry.is_enabled())
        .map(|(key, _)| *key)
        .collect();

    for requirement in missing_permissions(&enabled, required, relevant) {
        let key = permission_toml_key(requirement.key);
        match requirement.evidence {
            PermissionEvidence::Declared => warn!(
                "{} needs the `{key}` permission ({}). Add it to Water.toml:\n\n    [permissions.{key}]\n    enable = true\n",
                requirement.package, requirement.reason
            ),
            PermissionEvidence::Inferred => warn!(
                "This app likely needs the `{key}` permission: {} ({}). If it talks to the network, add it to Water.toml:\n\n    [permissions.{key}]\n    enable = true\n",
                requirement.reason, requirement.package
            ),
        }
    }
}

/// Renders a permission key the way it is written in `Water.toml`.
fn permission_toml_key(key: PermissionKey) -> String {
    serde_json::to_value(key)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{key:?}"))
}

#[cfg(test)]
mod permission_audit_tests {
    use super::*;
    use tempfile::tempdir;

    fn requirement(key: PermissionKey) -> RequiredPermission {
        RequiredPermission {
            package: String::from("waterui-map-gpu"),
            key,
            reason: String::from("downloads map styles and vector tiles"),
            evidence: PermissionEvidence::Declared,
        }
    }

    fn package(name: &str) -> cargo_metadata::Package {
        let manifest = format!(
            r#"{{
                "name": "{name}",
                "version": "1.0.0",
                "id": "registry+https://github.com/rust-lang/crates.io-index#{name}@1.0.0",
                "dependencies": [],
                "targets": [],
                "features": {{}},
                "manifest_path": "/dev/null/Cargo.toml"
            }}"#
        );
        serde_json::from_str(&manifest).expect("synthesize a cargo package")
    }

    /// Writes a minimal compilable crate — `[package]` for `name`, an empty
    /// `src/lib.rs`, and `extra` verbatim manifest TOML — and returns the
    /// manifest path a scan can be pointed at.
    fn write_crate(dir: &Path, name: &str, extra: &str) -> PathBuf {
        std::fs::create_dir_all(dir.join("src")).expect("crate src dir");
        let manifest = dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n{extra}"
            ),
        )
        .expect("crate manifest");
        std::fs::write(dir.join("src/lib.rs"), "").expect("crate source");
        manifest
    }

    /// The scan resolves the graph of the manifest it is handed — the crate
    /// the build actually compiles. A permission declared only through the
    /// managed backend's dependency tree is reported from the backend's
    /// manifest and is invisible from the FFI companion's.
    #[test]
    fn a_permission_declared_in_the_built_crates_graph_is_scanned() {
        let project = tempdir().expect("temp project");
        // `theme` stands in for a managed backend's own dependency, such as
        // hydrolysis-m3: the backend crate links it, the FFI crate does not.
        write_crate(
            &project.path().join("theme"),
            "theme",
            "[package.metadata.waterui.permissions]\n\
             internet = { reason = \"downloads map styles and vector tiles\" }\n",
        );
        let ffi_manifest = write_crate(&project.path().join("ffi"), "app-ffi", "");
        let backend_manifest = write_crate(
            &project.path().join("hydrolysis"),
            "app-hydrolysis",
            "[dependencies]\ntheme = { path = \"../theme\" }\n",
        );

        let required = smol::block_on(scan_required_permissions(
            &crate::toolchain::Host::current(),
            &backend_manifest,
        ))
        .expect("scan the built crate's graph");
        assert!(
            required
                .iter()
                .any(|requirement| requirement.package == "theme"
                    && requirement.key == PermissionKey::Internet
                    && requirement.evidence == PermissionEvidence::Declared),
            "the backend graph must report the permission `theme` declares"
        );

        let ffi = smol::block_on(scan_required_permissions(
            &crate::toolchain::Host::current(),
            &ffi_manifest,
        ))
        .expect("scan the ffi crate's graph");
        assert!(
            ffi.is_empty(),
            "the ffi graph does not carry `theme` and must stay silent"
        );
    }

    /// `package_feature_enabled` reads the same graph: a feature a dependency
    /// carries only through the managed backend's manifest reports enabled
    /// there and absent from the FFI companion's.
    #[test]
    fn a_feature_enabled_in_the_built_crates_graph_is_seen() {
        let directory = tempdir().expect("temp project");
        write_crate(
            &directory.path().join("theme"),
            "theme",
            "[features]\nextra = []\n",
        );
        let ffi_manifest = write_crate(&directory.path().join("ffi"), "app-ffi", "");
        let backend_manifest = write_crate(
            &directory.path().join("hydrolysis"),
            "app-hydrolysis",
            "[dependencies]\ntheme = { path = \"../theme\", features = [\"extra\"] }\n",
        );

        // `package_feature_enabled` resolves through the project's
        // per-triple cache, so the fixture needs a project of its own.
        std::fs::write(
            directory.path().join("Water.toml"),
            "[package]\nname = \"fixture\"\nbundle_identifier = \"dev.waterui.fixture\"\n",
        )
        .expect("Water.toml");
        std::fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("Cargo.toml");
        std::fs::create_dir_all(directory.path().join("src")).expect("src dir");
        std::fs::write(directory.path().join("src/lib.rs"), "").expect("src lib");
        let project = smol::block_on(crate::project::Project::open(
            &crate::toolchain::testing::real_toolchain_host(directory.path()),
            directory.path(),
            crate::project::ManagedBackends::NONE,
        ))
        .expect("fixture project opens");
        let target = crate::platform::TargetPlatform::MacOS.triple();

        assert!(
            smol::block_on(package_feature_enabled(
                &project,
                &backend_manifest,
                "theme",
                "extra",
                &target,
            ))
            .expect("scan the built crate's graph")
        );
        assert!(
            !smol::block_on(package_feature_enabled(
                &project,
                &ffi_manifest,
                "theme",
                "extra",
                &target,
            ))
            .expect("scan the ffi crate's graph")
        );
    }

    #[test]
    fn an_http_client_in_the_graph_suggests_internet() {
        let packages = vec![package("serde"), package("zenwave")];

        let inferred = infer_internet_from_http_clients(&packages, &[])
            .expect("zenwave should trigger the suggestion");

        assert_eq!(inferred.key, PermissionKey::Internet);
        assert_eq!(inferred.evidence, PermissionEvidence::Inferred);
        assert!(inferred.package.contains("zenwave"));
    }

    #[test]
    fn a_declared_internet_requirement_silences_the_inference() {
        let packages = vec![package("reqwest")];
        let declared = vec![requirement(PermissionKey::Internet)];

        assert!(infer_internet_from_http_clients(&packages, &declared).is_none());
    }

    #[test]
    fn a_graph_without_http_clients_suggests_nothing() {
        let packages = vec![package("serde"), package("tracing")];

        assert!(infer_internet_from_http_clients(&packages, &[]).is_none());
    }

    #[test]
    fn a_declared_permission_the_app_enabled_is_not_reported() {
        let enabled = HashSet::from([PermissionKey::Internet]);
        let required = vec![requirement(PermissionKey::Internet)];

        assert_eq!(
            missing_permissions(&enabled, &required, |_| true),
            [] as [&RequiredPermission; 0]
        );
    }

    #[test]
    fn a_missing_permission_is_reported_once() {
        let required = vec![requirement(PermissionKey::Internet)];

        let missing = missing_permissions(&HashSet::new(), &required, |_| true);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].key, PermissionKey::Internet);
    }

    /// iOS never declares network access, so warning about it there would be
    /// noise — and a warning that cries wolf stops being read.
    #[test]
    fn a_permission_the_platform_does_not_declare_stays_quiet() {
        let required = vec![requirement(PermissionKey::Internet)];

        let android = missing_permissions(&HashSet::new(), &required, |key| {
            key.android_permission_name().is_some()
        });
        let ios = missing_permissions(&HashSet::new(), &required, |key| {
            key.ios_plist_key().is_some()
        });

        assert_eq!(android.len(), 1, "Android must ask for INTERNET");
        assert!(ios.is_empty(), "iOS declares no network permission");
    }

    #[test]
    fn the_reported_key_matches_the_water_toml_spelling() {
        assert_eq!(permission_toml_key(PermissionKey::Internet), "internet");
        assert_eq!(
            permission_toml_key(PermissionKey::CoarseLocation),
            "coarse_location"
        );
    }

    /// `water fetch --backend android` must never touch the GTK4 scaffold:
    /// the scoped scan covers only the crates an Android build reads, so a
    /// backend crate whose dependency graph does not resolve on this host
    /// cannot fail a fetch meant for another backend.
    #[test]
    fn a_backend_scope_selects_only_that_backends_crate() {
        use crate::platform::TargetBackend;

        let scope = Some(TargetBackend::Android);
        assert!(!crate::gtk4::backend::Gtk4Backend::in_scope(scope));
        assert!(!crate::hydrolysis::backend::HydrolysisBackend::in_scope(
            scope
        ));
        assert!(!crate::winui::backend::WinUiBackend::in_scope(scope));

        let gtk4 = Some(TargetBackend::Gtk4);
        assert!(crate::gtk4::backend::Gtk4Backend::in_scope(gtk4));
        assert!(!crate::hydrolysis::backend::HydrolysisBackend::in_scope(
            gtk4
        ));
        assert!(!crate::winui::backend::WinUiBackend::in_scope(gtk4));

        // An unscoped fetch keeps scanning every wanted crate.
        assert!(crate::gtk4::backend::Gtk4Backend::in_scope(None));
        assert!(crate::hydrolysis::backend::HydrolysisBackend::in_scope(
            None
        ));
        assert!(crate::winui::backend::WinUiBackend::in_scope(None));
    }

    /// Manifest components are collected across the resolved graph the way
    /// Kotlin sources are: a `feature.<name>` table whose cargo feature is
    /// disabled contributes nothing, the same table with the feature on
    /// contributes its components, and two crates declaring one component
    /// differently fail the collection naming both.
    #[test]
    fn manifest_components_are_collected_across_the_graph() {
        let project = tempdir().expect("temp project");
        let provider = "name = \"waterkit.clipboard.ClipboardFileProvider\"\n\
                        authorities = [\"${applicationId}.waterkit.clipboard\"]\n\
                        exported = false\n\
                        grant-uri-permissions = true\n";
        write_crate(
            &project.path().join("clipboard"),
            "clipboard",
            &format!(
                "[features]\nfiles = []\n\n\
                 [[package.metadata.waterui.android.feature.files.provider]]\n{provider}"
            ),
        );
        let gated = write_crate(
            &project.path().join("gated"),
            "app-gated",
            "[dependencies]\nclipboard = { path = \"../clipboard\" }\n",
        );
        let enabled = write_crate(
            &project.path().join("enabled"),
            "app-enabled",
            "[dependencies]\nclipboard = { path = \"../clipboard\", features = [\"files\"] }\n",
        );

        let collect = |manifest: &Path| {
            let metadata = smol::block_on(crate_metadata(
                &crate::toolchain::Host::current(),
                manifest,
                &[],
            ))
            .expect("resolve the fixture graph");
            collect_android_declarations(&metadata)
        };
        let block = |declarations: AndroidDeclarations| {
            declarations
                .manifest
                .render_block()
                .expect("render the collected components")
        };

        let gated = block(collect(&gated).expect("collect the gated graph"));
        assert!(!gated.contains("<provider"), "{gated}");
        let enabled = block(collect(&enabled).expect("collect the enabled graph"));
        assert!(
            enabled.contains("android:name=\"waterkit.clipboard.ClipboardFileProvider\""),
            "{enabled}"
        );

        // A second crate exporting the same provider differently.
        write_crate(
            &project.path().join("other"),
            "other-clipboard",
            &format!(
                "[[package.metadata.waterui.android.provider]]\n{}",
                provider.replace("exported = false", "exported = true")
            ),
        );
        let conflicting = write_crate(
            &project.path().join("conflicting"),
            "app-conflicting",
            "[dependencies]\n\
             clipboard = { path = \"../clipboard\", features = [\"files\"] }\n\
             other-clipboard = { path = \"../other\" }\n",
        );
        let error = collect(&conflicting).expect_err("conflicting providers must fail");
        let message = error.to_string();
        assert!(message.contains("`clipboard`"), "{message}");
        assert!(message.contains("`other-clipboard`"), "{message}");

        // A feature table naming a cargo feature the crate does not declare.
        write_crate(
            &project.path().join("typo"),
            "typo",
            "[package.metadata.waterui.android.feature.phantom]\nmaven = []\n",
        );
        let mistyped = write_crate(
            &project.path().join("mistyped"),
            "app-mistyped",
            "[dependencies]\ntypo = { path = \"../typo\" }\n",
        );
        let message = collect(&mistyped)
            .expect_err("a feature table the crate does not declare must fail")
            .to_string();
        assert!(message.contains("typo"), "{message}");
        assert!(message.contains("phantom"), "{message}");
    }

    /// Gradle plugins are collected across the resolved graph like every
    /// other Android declaration: a `feature.<name>` table whose cargo
    /// feature is disabled contributes nothing, the same table with the
    /// feature on contributes its plugin, and two crates pinning one plugin
    /// at different versions fail the collection naming both.
    #[test]
    fn gradle_plugins_are_collected_across_the_graph() {
        let project = tempdir().expect("temp project");
        let plugin = "id = \"com.google.gms.google-services\"\n\
                      version = \"4.4.2\"\n";
        write_crate(
            &project.path().join("push"),
            "push",
            &format!(
                "[features]\nremote = []\n\n\
                 [[package.metadata.waterui.android.feature.remote.gradle-plugin]]\n{plugin}"
            ),
        );
        let gated = write_crate(
            &project.path().join("gated"),
            "app-gated",
            "[dependencies]\npush = { path = \"../push\" }\n",
        );
        let enabled = write_crate(
            &project.path().join("enabled"),
            "app-enabled",
            "[dependencies]\npush = { path = \"../push\", features = [\"remote\"] }\n",
        );
        let collect = |manifest: &Path| {
            let metadata = smol::block_on(crate_metadata(
                &crate::toolchain::Host::current(),
                manifest,
                &[],
            ))
            .expect("resolve the fixture graph");
            collect_android_declarations(&metadata)
        };

        let gated = collect(&gated).expect("collect the gated graph");
        assert_eq!(gated.gradle_plugins.iter().count(), 0);
        let enabled = collect(&enabled).expect("collect the enabled graph");
        let plugins: Vec<String> = enabled
            .gradle_plugins
            .iter()
            .map(|(crate_name, id, version)| format!("{crate_name} {id} {version}"))
            .collect();
        assert_eq!(plugins, ["push com.google.gms.google-services 4.4.2"]);

        write_crate(
            &project.path().join("other"),
            "other-push",
            &format!(
                "[[package.metadata.waterui.android.gradle-plugin]]\n{}",
                plugin.replace("4.4.2", "4.3.0")
            ),
        );
        let conflicting = write_crate(
            &project.path().join("conflicting"),
            "app-conflicting",
            "[dependencies]\n\
             push = { path = \"../push\", features = [\"remote\"] }\n\
             other-push = { path = \"../other\" }\n",
        );
        let message = collect(&conflicting)
            .expect_err("two versions of one plugin must fail")
            .to_string();
        assert!(message.contains("`push`"), "{message}");
        assert!(message.contains("`other-push`"), "{message}");
    }

    /// The iOS development entitlements `declarations` merge into an empty
    /// table.
    fn signed_entitlements(declarations: &AppleDeclarations) -> plist::Dictionary {
        let mut entitlements = plist::Dictionary::new();
        declarations
            .merge_into_entitlements(
                &mut entitlements,
                crate::platform::TargetPlatform::IOS,
                SigningEnvironment::Development,
            )
            .expect("merge entitlements");
        entitlements
    }

    /// Apple declarations are collected across the resolved graph: a
    /// `feature.<name>` table whose cargo feature is disabled contributes
    /// nothing, the same table with the feature on reaches the entitlements
    /// and `Info.plist`, and two crates giving one entitlement different
    /// values fail naming both.
    #[test]
    fn apple_declarations_are_collected_across_the_graph() {
        let project = tempdir().expect("temp project");
        write_crate(
            &project.path().join("push"),
            "push",
            "[features]\nremote = []\n\n\
             [package.metadata.waterui.apple.feature.remote]\n\
             environment-entitlements = [\"aps-environment\"]\n\n\
             [package.metadata.waterui.apple.feature.remote.entitlements]\n\
             \"com.apple.developer.usernotifications.time-sensitive\" = true\n\n\
             [package.metadata.waterui.apple.feature.remote.info-plist]\n\
             UIBackgroundModes = [\"remote-notification\"]\n",
        );
        let gated = write_crate(
            &project.path().join("gated"),
            "app-gated",
            "[dependencies]\npush = { path = \"../push\" }\n",
        );
        let enabled = write_crate(
            &project.path().join("enabled"),
            "app-enabled",
            "[dependencies]\npush = { path = \"../push\", features = [\"remote\"] }\n",
        );
        let host = crate::toolchain::Host::current();
        let collect = |manifest: &Path| {
            let metadata = smol::block_on(crate_metadata(&host, manifest, &[]))
                .expect("resolve the fixture graph");
            collect_apple_declarations(&metadata)
        };
        let gated = collect(&gated).expect("collect the gated graph");
        assert!(signed_entitlements(&gated).is_empty());
        let mut plist = plist::Dictionary::new();
        gated
            .merge_into_info_plist(&mut plist)
            .expect("merge Info.plist");
        assert!(plist.is_empty());

        let enabled = collect(&enabled).expect("collect the enabled graph");
        let entitlements = signed_entitlements(&enabled);
        assert_eq!(
            entitlements.get("aps-environment"),
            Some(&plist::Value::String("development".to_owned()))
        );
        assert_eq!(
            entitlements.get("com.apple.developer.usernotifications.time-sensitive"),
            Some(&plist::Value::Boolean(true))
        );
        enabled
            .merge_into_info_plist(&mut plist)
            .expect("merge Info.plist");
        assert_eq!(
            plist.get("UIBackgroundModes"),
            Some(&plist::Value::Array(vec![plist::Value::String(
                "remote-notification".to_owned()
            )]))
        );

        write_crate(
            &project.path().join("other"),
            "other-push",
            "[package.metadata.waterui.apple.entitlements]\n\
             \"com.apple.developer.usernotifications.time-sensitive\" = false\n",
        );
        let conflicting = write_crate(
            &project.path().join("conflicting"),
            "app-conflicting",
            "[dependencies]\n\
             push = { path = \"../push\", features = [\"remote\"] }\n\
             other-push = { path = \"../other\" }\n",
        );
        let message = collect(&conflicting)
            .expect_err("two values of one entitlement must fail")
            .to_string();
        assert!(message.contains("`push`"), "{message}");
        assert!(message.contains("`other-push`"), "{message}");
    }

    /// A `feature.<name>` metadata table naming a cargo feature the crate
    /// does not declare is a typo the collection must surface, naming both
    /// the crate and the feature.
    #[test]
    fn a_feature_table_for_an_undeclared_feature_fails_naming_both() {
        let project = tempdir().expect("temp project");
        write_crate(
            &project.path().join("typo"),
            "typo",
            "[package.metadata.waterui.apple.feature.phantom.entitlements]\n\
             \"com.apple.developer.applesignin\" = [\"Default\"]\n",
        );
        let mistyped = write_crate(
            &project.path().join("mistyped"),
            "app-mistyped",
            "[dependencies]\ntypo = { path = \"../typo\" }\n",
        );
        let metadata = smol::block_on(crate_metadata(
            &crate::toolchain::Host::current(),
            &mistyped,
            &[],
        ))
        .expect("resolve the fixture graph");
        let message = collect_apple_declarations(&metadata)
            .expect_err("a feature table the crate does not declare must fail")
            .to_string();
        assert!(message.contains("typo"), "{message}");
        assert!(message.contains("phantom"), "{message}");
    }

    /// App-value requests are collected across the resolved graph and gated
    /// per entry: a request behind a disabled `required-feature` asks the
    /// app for nothing.
    #[test]
    fn app_value_requests_are_collected_across_the_graph() {
        let project = tempdir().expect("temp project");
        write_crate(
            &project.path().join("push"),
            "push",
            "[features]\nremote = []\n\n\
             [[package.metadata.waterui.app-value]]\n\
             key = \"firebase_config\"\n\
             required-feature = \"remote\"\n\n\
             [[package.metadata.waterui.app-value]]\n\
             key = \"cast_receiver_app_id\"\n",
        );
        let gated = write_crate(
            &project.path().join("gated"),
            "app-gated",
            "[dependencies]\npush = { path = \"../push\" }\n",
        );
        let enabled = write_crate(
            &project.path().join("enabled"),
            "app-enabled",
            "[dependencies]\npush = { path = \"../push\", features = [\"remote\"] }\n",
        );
        let collect = |manifest: &Path| {
            let metadata = smol::block_on(crate_metadata(
                &crate::toolchain::Host::current(),
                manifest,
                &[],
            ))
            .expect("resolve the fixture graph");
            collect_android_declarations(&metadata).expect("collect the graph")
        };
        let push = ["push".to_owned()];

        let gated = collect(&gated);
        assert_eq!(
            gated
                .app_values
                .requesters(app_values::AppValueKey::FirebaseConfig),
            None
        );
        assert_eq!(
            gated
                .app_values
                .requesters(app_values::AppValueKey::CastReceiverAppId),
            Some(&push[..])
        );

        let enabled = collect(&enabled);
        assert_eq!(
            enabled
                .app_values
                .requesters(app_values::AppValueKey::FirebaseConfig),
            Some(&push[..])
        );
    }

    /// An unknown key in `[package.metadata.waterui.apple]` fails the
    /// collection instead of silently dropping a declaration from the app.
    #[test]
    fn unknown_apple_metadata_keys_fail_the_scan() {
        let project = tempdir().expect("temp project");
        write_crate(
            &project.path().join("typo"),
            "typo",
            "[package.metadata.waterui.apple]\nentitlement = { \"com.apple.developer.applesignin\" = [\"Default\"] }\n",
        );
        let app = write_crate(
            &project.path().join("app"),
            "app",
            "[dependencies]\ntypo = { path = \"../typo\" }\n",
        );
        let metadata = smol::block_on(crate_metadata(
            &crate::toolchain::Host::current(),
            &app,
            &[],
        ))
        .expect("resolve the fixture graph");
        let message = collect_apple_declarations(&metadata)
            .expect_err("an unknown key must fail")
            .to_string();
        assert!(message.contains("typo"), "{message}");
    }

    /// Every key of `[package.metadata.waterui.android]` is the CLI's: a
    /// misspelt one fails the collection instead of silently dropping a
    /// declaration from the app.
    #[test]
    fn an_unknown_android_metadata_key_fails() {
        let project = tempdir().expect("temp project");
        let manifest = write_crate(
            project.path(),
            "typo",
            "[[package.metadata.waterui.android.providers]]\nname = \"a.B\"\n",
        );
        let metadata = smol::block_on(crate_metadata(
            &crate::toolchain::Host::current(),
            &manifest,
            &[],
        ))
        .expect("resolve the fixture graph");
        let error = collect_android_declarations(&metadata).expect_err("unknown key must fail");
        assert!(error.to_string().contains("providers"), "{error}");
    }

    /// The Gradle `dependencies` block markers a generated module ships.
    const DEPS_MARKERS: &str = "dependencies {\n    // --- begin waterui android classpath dependencies ---\n    // --- end waterui android classpath dependencies ---\n}\n";

    /// Writes a module dir scaffold carrying the managed dependencies block
    /// the generated modules ship.
    fn module_dir_with_deps_block(root: &Path) -> PathBuf {
        let module_dir = root.join("app");
        std::fs::create_dir_all(&module_dir).expect("module dir");
        std::fs::write(module_dir.join("build.gradle.kts"), DEPS_MARKERS)
            .expect("module build script");
        module_dir
    }

    /// A staged classpath lands its Kotlin sources under `src/main/java/waterui/`,
    /// its Maven coordinates inside the managed dependencies block, and emits
    /// an R8 keep per package a staged source lives in — the only thing
    /// standing between `loadClass` and the release build's shrinker.
    #[test]
    fn staging_places_sources_maven_deps_and_keep_rules() {
        let root = tempdir().expect("temp root");
        let crate_dir = root.path().join("crate");
        std::fs::create_dir_all(&crate_dir).expect("crate dir");
        let source = crate_dir.join("DialogHelper.kt");
        std::fs::write(
            &source,
            "package waterkit.dialog\n\nobject DialogHelper {}\n",
        )
        .expect("kotlin source");
        let module_dir = module_dir_with_deps_block(root.path());

        smol::block_on(stage_classpath_files(
            &AndroidClasspath {
                kotlin_sources: vec![source],
                maven: BTreeSet::from(["androidx.health.connect:connect-client:1.1.0".to_string()]),
            },
            &module_dir,
            AndroidDependencyScope::Implementation,
        ))
        .expect("stage classpath");

        assert!(
            module_dir
                .join("src/main/java/waterui/DialogHelper.kt")
                .is_file()
        );
        let build =
            std::fs::read_to_string(module_dir.join("build.gradle.kts")).expect("build script");
        assert!(
            build.contains("implementation(\"androidx.health.connect:connect-client:1.1.0\")"),
            "{build}"
        );
        let rules =
            std::fs::read_to_string(module_dir.join("proguard-rules.pro")).expect("keep rules");
        assert!(
            rules.contains("-keep class waterkit.dialog.** { *; }"),
            "{rules}"
        );
    }

    /// The embedded AAR module exports its classpath dependencies as `api`
    /// so consumers resolve them through the published POM.
    #[test]
    fn embedded_scope_stages_maven_dependencies_as_api() {
        let root = tempdir().expect("temp root");
        let module_dir = module_dir_with_deps_block(root.path());

        smol::block_on(stage_classpath_files(
            &AndroidClasspath {
                kotlin_sources: Vec::new(),
                maven: BTreeSet::from(["androidx.health.connect:connect-client:1.1.0".to_string()]),
            },
            &module_dir,
            AndroidDependencyScope::Api,
        ))
        .expect("stage classpath");

        let build =
            std::fs::read_to_string(module_dir.join("build.gradle.kts")).expect("build script");
        assert!(
            build.contains("api(\"androidx.health.connect:connect-client:1.1.0\")"),
            "{build}"
        );
    }

    /// Restaging an empty classpath removes everything an earlier stage left:
    /// sources, Maven coordinates, and the managed keep block — never stale
    /// classes.
    #[test]
    fn restaging_an_empty_classpath_removes_stale_entries() {
        let root = tempdir().expect("temp root");
        let crate_dir = root.path().join("crate");
        std::fs::create_dir_all(&crate_dir).expect("crate dir");
        let source = crate_dir.join("PermissionHelper.kt");
        std::fs::write(&source, "package waterkit.permission\n").expect("kotlin source");
        let module_dir = module_dir_with_deps_block(root.path());

        smol::block_on(stage_classpath_files(
            &AndroidClasspath {
                kotlin_sources: vec![source],
                maven: BTreeSet::from(["androidx.health.connect:connect-client:1.1.0".to_string()]),
            },
            &module_dir,
            AndroidDependencyScope::Implementation,
        ))
        .expect("stage classpath");
        smol::block_on(stage_classpath_files(
            &AndroidClasspath::default(),
            &module_dir,
            AndroidDependencyScope::Implementation,
        ))
        .expect("restage empty");

        assert!(!module_dir.join("src/main/java/waterui").exists());
        let rules =
            std::fs::read_to_string(module_dir.join("proguard-rules.pro")).expect("keep rules");
        assert!(
            !rules.contains("-keep class waterkit.permission"),
            "{rules}"
        );
        assert!(!rules.contains(ANDROID_KEEPS_BEGIN), "{rules}");
        let build =
            std::fs::read_to_string(module_dir.join("build.gradle.kts")).expect("build script");
        assert!(!build.contains("connect-client"), "{build}");
        assert!(build.contains(ANDROID_DEPS_BEGIN), "{build}");
    }

    /// A declared Kotlin source without a `package` directive cannot be kept
    /// by R8 — it fails the stage rather than silently shipping an
    /// unresolvable class.
    #[test]
    fn a_kotlin_source_without_a_package_declaration_fails() {
        let root = tempdir().expect("temp root");
        let source = root.path().join("NoPackage.kt");
        std::fs::write(&source, "object NoPackage {}\n").expect("kotlin source");
        let module_dir = module_dir_with_deps_block(root.path());

        let error = smol::block_on(stage_classpath_files(
            &AndroidClasspath {
                kotlin_sources: vec![source],
                maven: BTreeSet::new(),
            },
            &module_dir,
            AndroidDependencyScope::Implementation,
        ))
        .expect_err("a package-less Kotlin source must fail the stage");
        assert!(error.to_string().contains("package"), "{error}");
    }

    /// A module build script without the managed markers was hand-edited or
    /// predates the mechanism — error rather than silently skipping the deps.
    #[test]
    fn a_module_build_script_without_the_deps_markers_fails() {
        let root = tempdir().expect("temp root");
        let module_dir = root.path().join("app");
        std::fs::create_dir_all(&module_dir).expect("module dir");
        std::fs::write(module_dir.join("build.gradle.kts"), "dependencies {\n}\n")
            .expect("module build script");

        let error = smol::block_on(stage_classpath_files(
            &AndroidClasspath {
                kotlin_sources: Vec::new(),
                maven: BTreeSet::from(["androidx.health.connect:connect-client:1.1.0".to_string()]),
            },
            &module_dir,
            AndroidDependencyScope::Implementation,
        ))
        .expect_err("a build script without the managed block must fail");
        assert!(error.to_string().contains("managed"), "{error}");
    }

    /// A keep file carrying two managed blocks or a dangling begin marker is
    /// corrupt — error rather than appending a third block.
    #[test]
    fn a_malformed_keep_block_fails() {
        let root = tempdir().expect("temp root");
        let module_dir = module_dir_with_deps_block(root.path());
        let rules = module_dir.join("proguard-rules.pro");
        std::fs::write(
            &rules,
            format!("{ANDROID_KEEPS_BEGIN}\n{ANDROID_KEEPS_BEGIN}\n"),
        )
        .expect("duplicate keep markers");

        let error = smol::block_on(stage_classpath_files(
            &AndroidClasspath {
                kotlin_sources: Vec::new(),
                maven: BTreeSet::new(),
            },
            &module_dir,
            AndroidDependencyScope::Implementation,
        ))
        .expect_err("duplicated keep markers must fail");
        assert!(error.to_string().contains("more than one"), "{error}");

        std::fs::write(
            &rules,
            format!("{ANDROID_KEEPS_BEGIN}\n-keep class x.** {{*;}}\n"),
        )
        .expect("unterminated keep marker");
        let error = smol::block_on(stage_classpath_files(
            &AndroidClasspath {
                kotlin_sources: Vec::new(),
                maven: BTreeSet::new(),
            },
            &module_dir,
            AndroidDependencyScope::Implementation,
        ))
        .expect_err("a begin marker without its end must fail");
        assert!(error.to_string().contains("malformed"), "{error}");
    }
}

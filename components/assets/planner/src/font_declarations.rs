//! Font declarations a resolved Cargo graph carries in its packages'
//! `[[package.metadata.waterui.assets.font]]` tables.
//!
//! The `water` CLI reads them to stage an application's fonts, and a test
//! host reads them to register the crate-local files a test binary would
//! otherwise never see; both go through [`dependency_font_declarations`].

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use cargo_metadata::{DependencyKind, Metadata, PackageId, Resolve};
use serde::Deserialize;
use thiserror::Error;

/// A font declaration — from `[[assets.font]]` in `Water.toml` or a crate's
/// `[package.metadata.waterui.assets.font]` Cargo.toml metadata.
#[derive(Debug, Clone)]
pub struct FontDeclaration {
    /// Font family name (used as `font_family` in Text).
    pub name: String,
    /// Source of the font file.
    pub source: FontSource,
    /// Crate or project that declared this font.
    pub crate_name: String,
}

/// Source of a font file.
#[derive(Debug, Clone)]
pub enum FontSource {
    /// Font bundled with the crate at a local path.
    Local {
        /// Absolute path to the crate root.
        crate_root: PathBuf,
        /// Relative path within the crate.
        relative_path: PathBuf,
    },
    /// Font that must be fetched out of band into the font cache.
    Remote {
        /// URL to fetch the font from when pre-seeding the cache.
        url: String,
    },
    /// Font from the built-in registry.
    BuiltIn,
}

impl FontSource {
    /// The source one declaration carries: the file `local_path` names —
    /// resolved against `root`, the declaring package's root — the remote
    /// font `remote_path` names, or the built-in registry when it sets
    /// neither.
    ///
    /// A declaration is one source, so setting both is an error, and so is a
    /// `local_path` rooted on the host platform. `Path::components`
    /// recognises only the host platform's rooted forms — `/x`, `\x`,
    /// `C:\x` and `C:x` on Windows, `/x` alone on Unix — so a `local_path`
    /// rooted only on another platform is not rejected here; it joins `root`
    /// like any relative path.
    ///
    /// # Errors
    ///
    /// [`FontDeclarationError::ConflictingSources`] if both paths are set,
    /// [`FontDeclarationError::AbsoluteLocalPath`] if `local_path` is rooted
    /// on this host.
    pub fn from_declaration(
        root: &Path,
        local_path: Option<String>,
        remote_path: Option<String>,
        crate_name: &str,
        family: &str,
    ) -> Result<Self, FontDeclarationError> {
        match (local_path, remote_path) {
            (Some(_), Some(_)) => Err(FontDeclarationError::ConflictingSources {
                crate_name: crate_name.to_owned(),
                family: family.to_owned(),
            }),
            (Some(local_path), None) => {
                let relative_path = PathBuf::from(local_path);
                if matches!(
                    relative_path.components().next(),
                    Some(Component::Prefix(_) | Component::RootDir)
                ) {
                    return Err(FontDeclarationError::AbsoluteLocalPath {
                        crate_name: crate_name.to_owned(),
                        family: family.to_owned(),
                        local_path: relative_path,
                    });
                }
                Ok(Self::Local {
                    crate_root: root.to_path_buf(),
                    relative_path,
                })
            }
            (None, Some(url)) => Ok(Self::Remote { url }),
            (None, None) => Ok(Self::BuiltIn),
        }
    }
}

/// A package's font declarations that cannot be read.
#[derive(Debug, Error)]
pub enum FontDeclarationError {
    /// The package's `[package.metadata.waterui]` table does not parse.
    #[error("crate {crate_name} declares malformed `[package.metadata.waterui]`: {source}")]
    Malformed {
        /// The declaring package.
        crate_name: String,
        /// Why the table does not parse.
        source: serde_json::Error,
    },
    /// A declaration's `local_path` is rooted instead of relative.
    #[error(
        "crate {crate_name} declares font `{family}` with local_path `{}`; local_path must be \
         relative to the declaring package's root",
        local_path.display()
    )]
    AbsoluteLocalPath {
        /// The declaring package.
        crate_name: String,
        /// The declared family.
        family: String,
        /// The rooted path it declares.
        local_path: PathBuf,
    },
    /// A declaration sets both `local_path` and `remote_path`.
    #[error(
        "crate {crate_name} declares font `{family}` with both local_path and remote_path; a \
         declaration has exactly one source"
    )]
    ConflictingSources {
        /// The declaring package.
        crate_name: String,
        /// The declared family.
        family: String,
    },
    /// The metadata was produced without dependency resolution (`--no-deps`),
    /// so a declaration's `required-feature` cannot be checked.
    #[error(
        "cargo metadata carries no resolved dependency graph (was it run with `--no-deps`?); \
         font declarations' `required-feature` cannot be checked without one"
    )]
    Unresolved,
    /// The metadata resolves no root package — `cargo metadata` on a virtual
    /// manifest produces none — so there is no package whose dependency
    /// closure `scope` could walk.
    #[error(
        "cargo metadata resolves no root package (was it run on a virtual manifest?); a \
         font-declaration scope walks the closure of one package"
    )]
    VirtualManifest,
}

/// The dependency closure [`dependency_font_declarations`] walks from the
/// resolved graph's root package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphScope {
    /// An application's graph: the root's normal dependencies transitively —
    /// the packages a build compiles. The `water` CLI resolves in this scope
    /// to stage an application's fonts.
    Build,
    /// A test binary's graph: [`Self::Build`] plus the root package's own
    /// dev-dependencies, which a test links and no other package's build
    /// does. `waterui-testing` resolves in this scope.
    Test,
}

/// The part of `[package.metadata.waterui]` this module reads; the table's
/// other keys belong to other readers.
#[derive(Debug, Deserialize)]
struct WaterUIMetadata {
    #[serde(default)]
    assets: AssetsMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct AssetsMetadata {
    #[serde(default)]
    font: Vec<FontMetadata>,
}

#[derive(Debug, Deserialize)]
struct FontMetadata {
    name: String,
    #[serde(default)]
    local_path: Option<String>,
    #[serde(default)]
    remote_path: Option<String>,
    /// Feature that must be enabled on the declaring package for the font to
    /// be declared.
    #[serde(default, rename = "required-feature")]
    required_feature: Option<String>,
}

/// The packages `scope` reaches from `resolve`'s root over `dep_kinds`
/// edges: normal dependencies transitively, plus — under
/// [`GraphScope::Test`] — the root's own dev-dependencies, which a test
/// links and no other package's build does. Build dependencies are never
/// followed.
///
/// # Errors
///
/// [`FontDeclarationError::VirtualManifest`] if `resolve` names no root
/// package, as `cargo metadata` produces for a virtual manifest.
///
/// # Panics
///
/// Panics if a resolved node's dependency names a package absent from
/// `resolve`'s nodes, which `cargo metadata` never produces.
fn scoped_packages(
    resolve: &Resolve,
    scope: GraphScope,
) -> Result<HashSet<&PackageId>, FontDeclarationError> {
    let root = resolve
        .root
        .as_ref()
        .ok_or(FontDeclarationError::VirtualManifest)?;
    let nodes: HashMap<&PackageId, &cargo_metadata::Node> =
        resolve.nodes.iter().map(|node| (&node.id, node)).collect();
    let mut in_scope = HashSet::from([root]);
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        let node = nodes
            .get(id)
            .expect("a resolved node's dependency names a node of the graph");
        for dep in &node.deps {
            let follows = dep.dep_kinds.iter().any(|dep_kind| match dep_kind.kind {
                DependencyKind::Normal => true,
                DependencyKind::Development => scope == GraphScope::Test && id == root,
                DependencyKind::Build | DependencyKind::Unknown => false,
            });
            if follows && in_scope.insert(&dep.pkg) {
                pending.push(&dep.pkg);
            }
        }
    }
    Ok(in_scope)
}

/// Every font declaration `scope` reaches in `metadata`'s packages.
///
/// A declaration whose `required-feature` is not enabled on its package in
/// the resolved graph is left out. A `local_path` is resolved against the
/// declaring package's manifest directory.
///
/// # Errors
///
/// [`FontDeclarationError::Unresolved`] if `metadata` carries no resolved
/// graph (`cargo metadata --no-deps`), [`FontDeclarationError::VirtualManifest`]
/// if it resolves no root package (a virtual manifest),
/// [`FontDeclarationError::Malformed`] if a package's
/// `[package.metadata.waterui]` does not parse,
/// [`FontDeclarationError::AbsoluteLocalPath`] if a declaration's
/// `local_path` is rooted, and [`FontDeclarationError::ConflictingSources`]
/// if a declaration sets both `local_path` and `remote_path`.
///
/// # Panics
///
/// Panics if `metadata` names a package manifest without a parent
/// directory, which `cargo metadata` never reports.
pub fn dependency_font_declarations(
    metadata: &Metadata,
    scope: GraphScope,
) -> Result<Vec<FontDeclaration>, FontDeclarationError> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or(FontDeclarationError::Unresolved)?;
    let in_scope = scoped_packages(resolve, scope)?;
    let enabled_features: HashMap<&PackageId, HashSet<&str>> = resolve
        .nodes
        .iter()
        .map(|node| (&node.id, node.features.iter().map(|f| f.as_str()).collect()))
        .collect();

    let mut fonts = Vec::new();
    for package in &metadata.packages {
        if !in_scope.contains(&package.id) {
            continue;
        }
        let Some(waterui) = package.metadata.get("waterui") else {
            continue;
        };
        let parsed: WaterUIMetadata =
            serde_json::from_value(waterui.clone()).map_err(|source| {
                FontDeclarationError::Malformed {
                    crate_name: package.name.to_string(),
                    source,
                }
            })?;
        if parsed.assets.font.is_empty() {
            continue;
        }
        let crate_root = package
            .manifest_path
            .parent()
            .expect("a package's manifest path names a file inside its directory")
            .as_std_path();
        let features = enabled_features.get(&package.id);
        for font in parsed.assets.font {
            if let Some(required) = &font.required_feature
                && !features.is_some_and(|features| features.contains(required.as_str()))
            {
                continue;
            }
            let source = FontSource::from_declaration(
                crate_root,
                font.local_path,
                font.remote_path,
                package.name.as_ref(),
                &font.name,
            )?;
            fonts.push(FontDeclaration {
                name: font.name,
                source,
                crate_name: package.name.to_string(),
            });
        }
    }
    Ok(fonts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "path+file:///ws/app#app@0.1.0";
    const LIB: &str = "path+file:///ws/lib#lib@0.1.0";
    const DEVLIB: &str = "path+file:///ws/devlib#devlib@0.1.0";
    const OTHER: &str = "path+file:///ws/other#other@0.1.0";
    const OTHERDEV: &str = "path+file:///ws/otherdev#otherdev@0.1.0";

    /// One package of the fixture graph, carrying `waterui` as its
    /// `[package.metadata]` value.
    fn package(name: &str, dir: &str, waterui: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "version": "0.1.0",
            "id": format!("path+file://{dir}#{name}@0.1.0"),
            "dependencies": [],
            "targets": [],
            "features": {},
            "manifest_path": format!("{dir}/Cargo.toml"),
            "metadata": waterui,
        })
    }

    /// A `[package.metadata]` value declaring `local_path` fonts, each
    /// `(name, local_path, required_feature)`.
    fn waterui_fonts(fonts: &[(&str, &str, Option<&str>)]) -> serde_json::Value {
        serde_json::json!({
            "waterui": {
                "assets": {
                    "font": fonts
                        .iter()
                        .map(|(name, local_path, required_feature)| {
                            serde_json::json!({
                                "name": name,
                                "local_path": local_path,
                                "required-feature": required_feature,
                            })
                        })
                        .collect::<Vec<_>>(),
                },
            },
        })
    }

    /// One resolved node: `deps` as `dep_kinds` entries (`null` kind is a
    /// normal dependency) and `features` as the features enabled on it.
    fn node(id: &str, deps: &[serde_json::Value], features: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "dependencies": deps
                .iter()
                .map(|dep| dep["pkg"].clone())
                .collect::<Vec<_>>(),
            "deps": deps,
            "features": features,
        })
    }

    /// One `deps` entry of a node: `kind` is `null` for a normal dependency
    /// or `"dev"` for a dev-dependency.
    fn dep(name: &str, pkg: &str, kind: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "pkg": pkg,
            "dep_kinds": [{"kind": kind, "target": null}],
        })
    }

    /// The fixture graph: `app` is the resolved root, depending normally on
    /// `lib` and dev-depending on `devlib`; `other` is a workspace member the
    /// root never reaches, and `otherdev` is `other`'s dev-dependency. `lib`'s
    /// `GatedFont` declares `required-feature = "extra"`, enabled only when
    /// `lib_features` carries it.
    fn fixture(lib_features: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "packages": [
                package("app", "/ws/app", &waterui_fonts(&[("AppFont", "fonts/app.ttf", None)])),
                package("lib", "/ws/lib", &waterui_fonts(&[
                    ("LibFont", "fonts/lib.ttf", None),
                    ("GatedFont", "fonts/gated.ttf", Some("extra")),
                ])),
                package("devlib", "/ws/devlib", &waterui_fonts(&[("DevFont", "fonts/dev.ttf", None)])),
                package("other", "/ws/other", &waterui_fonts(&[("OtherFont", "fonts/other.ttf", None)])),
                package("otherdev", "/ws/otherdev", &waterui_fonts(&[("OtherDevFont", "fonts/otherdev.ttf", None)])),
            ],
            "workspace_members": [APP, LIB, DEVLIB, OTHER, OTHERDEV],
            "resolve": {
                "nodes": [
                    node(APP, &[
                        dep("lib", LIB, &serde_json::Value::Null),
                        dep("devlib", DEVLIB, &serde_json::Value::from("dev")),
                    ], &[]),
                    node(LIB, &[], lib_features),
                    node(DEVLIB, &[], &[]),
                    node(OTHER, &[dep("otherdev", OTHERDEV, &serde_json::Value::from("dev"))], &[]),
                    node(OTHERDEV, &[], &[]),
                ],
                "root": APP,
            },
            "workspace_root": "/ws",
            "target_directory": "/ws/target",
            "version": 0,
        })
    }

    fn metadata(value: serde_json::Value) -> Metadata {
        serde_json::from_value(value).expect("the fixture parses as cargo metadata")
    }

    fn declared_names(metadata: &Metadata, scope: GraphScope) -> Vec<String> {
        dependency_font_declarations(metadata, scope)
            .expect("the fixture's declarations resolve")
            .into_iter()
            .map(|declaration| declaration.name)
            .collect()
    }

    #[test]
    fn build_scope_walks_the_roots_normal_dependencies() {
        assert_eq!(
            declared_names(&metadata(fixture(&["extra"])), GraphScope::Build),
            ["AppFont", "LibFont", "GatedFont"],
        );
    }

    #[test]
    fn test_scope_adds_the_roots_own_dev_dependencies() {
        assert_eq!(
            declared_names(&metadata(fixture(&["extra"])), GraphScope::Test),
            ["AppFont", "LibFont", "GatedFont", "DevFont"],
        );
    }

    #[test]
    fn a_required_feature_font_follows_the_resolved_features() {
        assert_eq!(
            declared_names(&metadata(fixture(&[])), GraphScope::Test),
            ["AppFont", "LibFont", "DevFont"],
        );
    }

    #[test]
    fn a_local_path_joins_the_declaring_crates_root() {
        let declarations =
            dependency_font_declarations(&metadata(fixture(&["extra"])), GraphScope::Build)
                .expect("the fixture's declarations resolve");
        let declaration = &declarations[0];
        assert_eq!(declaration.name, "AppFont");
        assert_eq!(declaration.crate_name, "app");
        let FontSource::Local {
            crate_root,
            relative_path,
        } = &declaration.source
        else {
            panic!("`AppFont` must resolve to a crate-local source")
        };
        assert_eq!(crate_root, &PathBuf::from("/ws/app"));
        assert_eq!(relative_path, &PathBuf::from("fonts/app.ttf"));
    }

    #[test]
    fn an_unresolved_metadata_is_an_error() {
        let mut value = fixture(&[]);
        value["resolve"] = serde_json::Value::Null;
        assert!(matches!(
            dependency_font_declarations(&metadata(value), GraphScope::Build),
            Err(FontDeclarationError::Unresolved)
        ));
    }

    #[test]
    fn a_virtual_manifest_is_an_error() {
        let mut value = fixture(&[]);
        value["resolve"]["root"] = serde_json::Value::Null;
        assert!(matches!(
            dependency_font_declarations(&metadata(value), GraphScope::Build),
            Err(FontDeclarationError::VirtualManifest)
        ));
    }

    #[test]
    fn a_malformed_waterui_table_is_an_error() {
        let mut value = fixture(&[]);
        value["packages"][1]["metadata"] =
            serde_json::json!({"waterui": {"assets": {"font": "none"}}});
        assert!(matches!(
            dependency_font_declarations(&metadata(value), GraphScope::Build),
            Err(FontDeclarationError::Malformed { .. })
        ));
    }

    #[test]
    fn a_rooted_local_path_is_an_error() {
        let mut value = fixture(&[]);
        value["packages"][1]["metadata"]["waterui"]["assets"]["font"][0]["local_path"] =
            serde_json::Value::from("/abs/font.ttf");
        assert!(matches!(
            dependency_font_declarations(&metadata(value), GraphScope::Build),
            Err(FontDeclarationError::AbsoluteLocalPath { .. })
        ));
    }

    #[test]
    fn a_declaration_with_two_sources_is_an_error() {
        let mut value = fixture(&[]);
        value["packages"][1]["metadata"]["waterui"]["assets"]["font"][0]["remote_path"] =
            serde_json::Value::from("https://example.com/font.ttf");
        assert!(matches!(
            dependency_font_declarations(&metadata(value), GraphScope::Build),
            Err(FontDeclarationError::ConflictingSources { .. })
        ));
    }
}

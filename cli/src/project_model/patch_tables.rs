//! The `[patch]` tables a project's build combines from two owners.
//!
//! Cargo honours `[patch]` only at the root of the workspace it builds, so the
//! CLI copies the framework's tables — a channel's pins or a local checkout's
//! own — into every manifest it roots a build at: the project's `Cargo.toml`
//! and each crate it generates in the build cache. The project adds entries of
//! its own beside them, to consume an unreleased component through a git pin
//! for one. Both owners name crates in the same tables, and the project's
//! entry wins, as it does in Cargo, where the root manifest's `[patch]` is
//! authoritative: an application may try a revision of a component the
//! framework pins differently.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cargo_toml::{Dependency, PatchSet};
use serde::Serialize as _;

/// Merge the project's `[patch]` entries into the framework's.
///
/// Every source key of either set is kept. A crate both name under the same
/// source takes the project's entry; each such override is logged with both
/// entries, since it builds a graph the framework was not tested against. An
/// identical entry (see [`same_entry`]) is no override and keeps the
/// framework's spelling. Both sets must express their `path` entries relative
/// to the same directory, or as absolute paths.
#[must_use]
pub fn merge(framework: PatchSet, project: PatchSet) -> PatchSet {
    let mut merged = framework;
    for (registry, dependencies) in project {
        let table = merged.entry(registry.clone()).or_default();
        for (name, dependency) in dependencies {
            if let Some(existing) = table.get(&name) {
                if same_entry(existing, &dependency) {
                    continue;
                }
                tracing::info!(
                    source = %registry,
                    package = %name,
                    framework = %inline_toml(existing),
                    project = %inline_toml(&dependency),
                    "the project's [patch] entry overrides the framework's"
                );
            }
            table.insert(name, dependency);
        }
    }
    merged
}

/// `set` without the crates `names` patches under the same source, whatever
/// their entries there.
#[must_use]
pub fn without(set: &PatchSet, names: &PatchSet) -> PatchSet {
    let mut remainder = PatchSet::new();
    for (registry, dependencies) in set {
        for (name, dependency) in dependencies {
            if !names
                .get(registry)
                .is_some_and(|table| table.contains_key(name))
            {
                remainder
                    .entry(registry.clone())
                    .or_default()
                    .insert(name.clone(), dependency.clone());
            }
        }
    }
    remainder
}

/// The entries of `current` that `written` does not hold verbatim.
///
/// `written` is the set the CLI wrote into a manifest earlier; whatever the
/// manifest holds beyond it — a crate the CLI never wrote, or one of its
/// entries the project has since changed — belongs to the project.
#[must_use]
pub fn beyond(current: &PatchSet, written: &PatchSet) -> PatchSet {
    let mut remainder = PatchSet::new();
    for (registry, dependencies) in current {
        for (name, dependency) in dependencies {
            let recorded = written
                .get(registry)
                .and_then(|table| table.get(name))
                .is_some_and(|entry| same_entry(entry, dependency));
            if !recorded {
                remainder
                    .entry(registry.clone())
                    .or_default()
                    .insert(name.clone(), dependency.clone());
            }
        }
    }
    remainder
}

/// Whether two patch entries select the same source.
///
/// `path` entries compare with their `.` and `..` segments resolved
/// lexically, as Cargo resolves a path dependency, so the same directory
/// reached through two spellings is one entry.
#[must_use]
pub fn same_entry(left: &Dependency, right: &Dependency) -> bool {
    match (left, right) {
        (Dependency::Detailed(left), Dependency::Detailed(right)) => {
            let mut left = left.as_ref().clone();
            let mut right = right.as_ref().clone();
            let left_path = left.path.take();
            let right_path = right.path.take();
            left == right
                && match (left_path, right_path) {
                    (Some(left), Some(right)) => {
                        crate::templates::collapse_dotdot(Path::new(&left))
                            == crate::templates::collapse_dotdot(Path::new(&right))
                    }
                    (left, right) => left == right,
                }
        }
        (left, right) => left == right,
    }
}

/// The rows of `patches` whose target package `metadata` resolved.
///
/// Cargo warns `patch ... was not used in the crate graph` for every row
/// the resolution left untouched, and a managed manifest inherits the
/// whole framework patch set whether or not the build needs each pin.
/// The rows a managed manifest writes are therefore only those a
/// `cargo metadata` resolve of the manifest itself actually selects:
/// a path row stays when its directory is the manifest directory of a
/// resolved path package of the same name (and declared version
/// requirement), a git row stays when a resolved package's source names
/// the same repository and reference, and a row carrying neither `path`
/// nor `git` — explicit registry configuration — stays unchanged: it is
/// a user's own choice, not a CLI-projected local override.
///
/// Two rows under different source keys pointing at the same used package
/// are both kept: retention follows resolved package identity, not the
/// source-table namespace.
///
/// # Errors
///
/// A patch path that cannot be canonicalized fails with the path named.
/// A resolved package missing from the graph only drops the row that
/// pointed at it.
pub async fn resolved_source_patches(
    patches: &PatchSet,
    metadata: &cargo_metadata::Metadata,
    manifest_dir: &Path,
) -> std::io::Result<PatchSet> {
    let packages = resolved_packages(metadata)?;
    let patches = patches.clone();
    let manifest_dir = manifest_dir.to_path_buf();
    smol::unblock(move || {
        // Canonicalize every distinct path once — a patch row's target
        // dir and each resolved package's manifest directory — then
        // compare canonical spellings, never raw text. A directory the
        // filesystem cannot canonicalize is a hard error, named.
        let mut canonical: HashMap<PathBuf, PathBuf> = HashMap::new();
        let mut canonical_dir = |dir: PathBuf| -> std::io::Result<PathBuf> {
            if let Some(canon) = canonical.get(&dir) {
                return Ok(canon.clone());
            }
            let canon = dunce::canonicalize(&dir).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("cannot canonicalize `{}`: {error}", dir.display()),
                )
            })?;
            canonical.insert(dir, canon.clone());
            Ok(canon)
        };
        let mut kept = PatchSet::new();
        for (registry, dependencies) in &patches {
            let mut table = None;
            for (name, dependency) in dependencies {
                let Some(detail) = detailed(dependency) else {
                    table
                        .get_or_insert_with(std::collections::BTreeMap::new)
                        .insert(name.clone(), dependency.clone());
                    continue;
                };
                if detail.path.is_none() && detail.git.is_none() {
                    table
                        .get_or_insert_with(std::collections::BTreeMap::new)
                        .insert(name.clone(), dependency.clone());
                    continue;
                }
                let package_name = detail.package.as_deref().unwrap_or(name);
                let row_dir = detail
                    .path
                    .as_ref()
                    .map(|path| canonical_dir(manifest_dir.join(path)))
                    .transpose()?;
                let mut used = false;
                for package in &packages {
                    if package.name != *package_name
                        || !detail
                            .version
                            .as_ref()
                            .is_none_or(|req| req.matches(&package.version))
                    {
                        continue;
                    }
                    let matches = match &row_dir {
                        Some(row_dir) => {
                            // `source: None` means a path package: a
                            // git-resolved package's checkout dir must
                            // never satisfy a path patch. The package's
                            // own dir is canonicalized only here — lazily,
                            // the first time a path row compares it —
                            // because Cargo already proved it exists.
                            package.source.is_none()
                                && canonical_dir(package.manifest_dir.clone().ok_or_else(
                                    || {
                                        std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            format!(
                                                "the resolved path package `{}` has no \
                                                 manifest directory",
                                                package.name,
                                            ),
                                        )
                                    },
                                )?)? == *row_dir
                        }
                        None => same_git_dependency(detail, package.source.as_ref()),
                    };
                    if matches {
                        used = true;
                        break;
                    }
                }
                if used {
                    table
                        .get_or_insert_with(std::collections::BTreeMap::new)
                        .insert(name.clone(), dependency.clone());
                }
            }
            if let Some(table) = table {
                kept.insert(registry.clone(), table);
            }
        }
        Ok(kept)
    })
    .await
}

/// A resolved package's identity for patch matching — the crate name, its
/// resolved version, the manifest directory to canonicalize, and the
/// parsed source when the package is not a path package.
struct ResolvedPackage {
    name: String,
    version: cargo_metadata::semver::Version,
    manifest_dir: Option<PathBuf>,
    source: Option<ResolvedSource>,
}

/// A resolved package's source id plus whether Cargo's source string
/// carried an explicit `branch`/`tag`/`rev`/`ref` query. `SourceId`
/// normalizes a missing query to the default branch, so the distinction
/// between `git+repo` and `git+repo?branch=master` — two different
/// package identities to Cargo — is preserved separately.
struct ResolvedSource {
    id: cargo_lock::SourceId,
    explicit_git_reference: bool,
}

/// The `metadata` packages the resolve actually selected, as
/// [`ResolvedPackage`]s. A metadata document with no resolve or a package
/// whose source Cargo emitted but [`cargo_lock::SourceId`] cannot parse is invalid
/// input, not an empty graph.
fn resolved_packages(metadata: &cargo_metadata::Metadata) -> std::io::Result<Vec<ResolvedPackage>> {
    let resolve = metadata.resolve.as_ref().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the generated-manifest metadata carries no resolve",
        )
    })?;
    let resolved: std::collections::HashSet<&str> = resolve
        .nodes
        .iter()
        .map(|node| node.id.repr.as_str())
        .collect();
    metadata
        .packages
        .iter()
        .filter(|package| resolved.contains(package.id.repr.as_str()))
        .map(|package| {
            Ok(ResolvedPackage {
                name: package.name.to_string(),
                version: package.version.clone(),
                manifest_dir: package
                    .manifest_path
                    .parent()
                    .map(|parent| PathBuf::from(parent.as_str())),
                source: package
                    .source
                    .as_ref()
                    .map(|source| {
                        let raw = source.to_string();
                        let id = cargo_lock::SourceId::from_url(&raw).map_err(|error| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!(
                                    "the resolved package `{}` carries a source Cargo \
                                     did not parse (`{source}`): {error}",
                                    package.name,
                                ),
                            )
                        })?;
                        let explicit_git_reference = if id.is_git() {
                            // `id.url()` drops the query — the presence of a
                            // declared reference is read from the raw source.
                            url::Url::parse(&raw)
                                .map_err(|error| {
                                    std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        format!(
                                            "the resolved package `{}` carries a source \
                                             URL that does not parse (`{source}`): {error}",
                                            package.name,
                                        ),
                                    )
                                })?
                                .query_pairs()
                                .any(|(key, _)| matches!(&*key, "branch" | "ref" | "tag" | "rev"))
                        } else {
                            false
                        };
                        Ok::<ResolvedSource, std::io::Error>(ResolvedSource {
                            id,
                            explicit_git_reference,
                        })
                    })
                    .transpose()?,
            })
        })
        .collect()
}

fn detailed(dependency: &Dependency) -> Option<&cargo_toml::DependencyDetail> {
    match dependency {
        Dependency::Detailed(detail) => Some(detail),
        _ => None,
    }
}

/// Whether a `git` patch row and a resolved package's source name the
/// same repository and reference. `source.id` already carries the
/// repository Cargo resolved — the query and locked-commit fragment do
/// not participate in identity — and a row without `branch`/`tag`/`rev`
/// matches only a source Cargo resolved without a declared reference:
/// `git+repo` and `git+repo?branch=master` are different package
/// identities, never interchangeable.
fn same_git_dependency(
    detail: &cargo_toml::DependencyDetail,
    source: Option<&ResolvedSource>,
) -> bool {
    let (Some(row_git), Some(source)) = (detail.git.as_deref(), source) else {
        return false;
    };
    if !source.id.is_git() {
        return false;
    }
    if crate::project_model::framework::canonical_git_url(row_git)
        != crate::project_model::framework::canonical_git_url(source.id.url().as_str())
    {
        return false;
    }
    let declared = detail
        .branch
        .as_ref()
        .map(|branch| cargo_lock::package::GitReference::Branch(branch.clone()))
        .or_else(|| {
            detail
                .tag
                .as_ref()
                .map(|tag| cargo_lock::package::GitReference::Tag(tag.clone()))
        })
        .or_else(|| {
            detail
                .rev
                .as_ref()
                .map(|rev| cargo_lock::package::GitReference::Rev(rev.clone()))
        });
    declared.map_or(!source.explicit_git_reference, |expected| {
        source.explicit_git_reference && source.id.git_reference() == Some(&expected)
    })
}

/// A patch entry as the inline table a manifest would spell it with.
fn inline_toml(dependency: &Dependency) -> String {
    dependency
        .serialize(toml_edit::ser::ValueSerializer::new())
        .expect("a patch entry serializes as a TOML value")
        .to_string()
}

#[cfg(test)]
mod tests {
    use cargo_toml::{Dependency, DependencyDetail, PatchSet};
    use std::collections::BTreeMap;

    /// A `cargo metadata` document with a distinct root package whose
    /// resolve edges reach `deps` — the root is never one of the patched
    /// packages itself.
    fn metadata_json(deps: &[serde_json::Value]) -> cargo_metadata::Metadata {
        let root = package_json("probe-root", "0.0.0", "/probe-root", None);
        let dep_ids: Vec<serde_json::Value> =
            deps.iter().map(|package| package["id"].clone()).collect();
        let mut nodes: Vec<serde_json::Value> = deps
            .iter()
            .map(|package| {
                serde_json::json!({
                    "id": package["id"], "dependencies": [],
                    "deps": [], "features": []
                })
            })
            .collect();
        let dep_edges: Vec<serde_json::Value> = dep_ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "name": "dep", "pkg": id,
                    "dep_kinds": [{ "kind": null, "target": null }]
                })
            })
            .collect();
        nodes.push(serde_json::json!({
            "id": root["id"], "dependencies": dep_ids,
            "deps": dep_edges,
            "features": []
        }));
        let mut packages = deps.to_vec();
        packages.push(root.clone());
        serde_json::from_value(serde_json::json!({
            "packages": packages,
            "workspace_members": [root["id"]],
            "workspace_default_members": [root["id"]],
            "resolve": { "nodes": nodes, "root": root["id"] },
            "target_directory": "/tmp/probe/target",
            "build_directory": "/tmp/probe/target",
            "version": 1,
            "workspace_root": "/tmp/probe",
            "metadata": null
        }))
        .expect("the metadata JSON decodes")
    }

    /// A `Package` JSON member — `source` is `null` for a path package or a
    /// `git+...`/`registry+...` string.
    fn package_json(
        name: &str,
        version: &str,
        dir: &str,
        source: Option<&str>,
    ) -> serde_json::Value {
        let manifest = format!("{dir}/Cargo.toml");
        let lib = format!("{dir}/src/lib.rs");
        serde_json::json!({
            "name": name,
            "version": version,
            "id": format!("path+file://{dir}#{name}@{version}"),
            "license": null, "license_file": null, "description": null,
            "source": source,
            "dependencies": [],
            "targets": [{ "kind": ["lib"], "crate_types": ["lib"], "name": name,
                         "src_path": lib, "edition": "2024", "doc": true,
                         "doctest": true, "test": true }],
            "features": {}, "manifest_path": manifest, "metadata": null,
            "publish": [], "authors": [], "categories": [], "keywords": [],
            "readme": null, "repository": null, "homepage": null,
            "documentation": null, "edition": "2024", "links": null,
            "default_run": null, "rust_version": null
        })
    }

    fn detail(fields: DependencyDetail) -> Dependency {
        Dependency::Detailed(Box::new(fields))
    }

    #[test]
    fn apple_source_patches_keeps_the_path_cargo_resolved() {
        let root = tempfile::tempdir().unwrap();
        let used = root.path().join("used");
        let other = root.path().join("other");
        std::fs::create_dir_all(&used).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let metadata = metadata_json(&[package_json("dep", "1.0.0", used.to_str().unwrap(), None)]);
        let patches = set(&[
            (
                "crates-io",
                "dep",
                detail(DependencyDetail {
                    path: Some(used.to_str().unwrap().to_owned()),
                    ..Default::default()
                }),
            ),
            (
                "crates-io",
                "unused-dep",
                detail(DependencyDetail {
                    path: Some(other.to_str().unwrap().to_owned()),
                    ..Default::default()
                }),
            ),
            // A second source table pointing at the same used package —
            // Cargo keeps both.
            (
                "https://github.com/water-rs/waterui",
                "dep",
                detail(DependencyDetail {
                    path: Some(used.to_str().unwrap().to_owned()),
                    ..Default::default()
                }),
            ),
        ]);
        let projected = smol::block_on(super::resolved_source_patches(
            &patches,
            &metadata,
            root.path(),
        ))
        .unwrap();
        let dep_rows: BTreeMap<_, _> = projected
            .iter()
            .flat_map(|(r, t)| t.keys().map(move |n| (r.as_str(), n.as_str())))
            .filter(|(_, name)| *name == "dep")
            .collect();
        assert_eq!(
            dep_rows.len(),
            2,
            "both mirrored rows keep the used package"
        );
        assert!(
            !projected
                .values()
                .any(|table| table.contains_key("unused-dep"))
        );
    }

    #[test]
    fn apple_source_patches_rejects_same_name_at_a_different_path() {
        let root = tempfile::tempdir().unwrap();
        let used = root.path().join("used");
        let same_name = root.path().join("same-name");
        std::fs::create_dir_all(&used).unwrap();
        std::fs::create_dir_all(&same_name).unwrap();
        let metadata = metadata_json(&[package_json("dep", "1.0.0", used.to_str().unwrap(), None)]);
        let patches = set(&[(
            "crates-io",
            "dep",
            detail(DependencyDetail {
                path: Some(same_name.to_str().unwrap().to_owned()),
                ..Default::default()
            }),
        )]);
        let projected = smol::block_on(super::resolved_source_patches(
            &patches,
            &metadata,
            root.path(),
        ))
        .unwrap();
        assert!(
            projected.is_empty(),
            "the same name at a different path does not count"
        );
    }

    #[test]
    fn apple_source_patches_matches_alias_and_version() {
        let root = tempfile::tempdir().unwrap();
        let used = root.path().join("used");
        std::fs::create_dir_all(&used).unwrap();
        let metadata = metadata_json(&[package_json(
            "real-dep",
            "2.0.0",
            used.to_str().unwrap(),
            None,
        )]);
        let patches = set(&[(
            "crates-io",
            "alias-dep",
            detail(DependencyDetail {
                path: Some(used.to_str().unwrap().to_owned()),
                package: Some("real-dep".to_owned()),
                version: Some(cargo_metadata::semver::VersionReq::parse(">=2").unwrap()),
                ..Default::default()
            }),
        )]);
        let projected = smol::block_on(super::resolved_source_patches(
            &patches,
            &metadata,
            root.path(),
        ))
        .unwrap();
        assert!(projected["crates-io"].contains_key("alias-dep"));

        // A declared version requirement the resolved version does not
        // satisfy drops the row.
        let mismatched = set(&[(
            "crates-io",
            "alias-dep",
            detail(DependencyDetail {
                path: Some(used.to_str().unwrap().to_owned()),
                package: Some("real-dep".to_owned()),
                version: Some(cargo_metadata::semver::VersionReq::parse("<2").unwrap()),
                ..Default::default()
            }),
        )]);
        assert!(
            smol::block_on(super::resolved_source_patches(
                &mismatched,
                &metadata,
                root.path()
            ))
            .unwrap()
            .is_empty()
        );
    }

    /// `(row git, row branch, row tag, row rev, resolved source, keeps row)`
    type GitCase = (
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        &'static str,
        bool,
    );
    const GIT_CASES: &[GitCase] = &[
        // Bare row vs bare (default-reference) source — same identity.
        (
            "https://github.com/water-rs/waterui",
            None,
            None,
            None,
            "git+https://github.com/water-rs/waterui#abc123",
            true,
        ),
        // Bare row vs explicit `?branch=master` — different identity.
        (
            "https://github.com/water-rs/waterui",
            None,
            None,
            None,
            "git+https://github.com/water-rs/waterui?branch=master#abc123",
            false,
        ),
        // Explicit `branch = "master"` vs bare source — different identity.
        (
            "https://github.com/water-rs/waterui",
            Some("master"),
            None,
            None,
            "git+https://github.com/water-rs/waterui#abc123",
            false,
        ),
        // Explicit `branch = "master"` vs explicit `?branch=master`.
        (
            "https://github.com/water-rs/waterui",
            Some("master"),
            None,
            None,
            "git+https://github.com/water-rs/waterui?branch=master#abc123",
            true,
        ),
        // Same declared branch.
        (
            "https://github.com/water-rs/waterui",
            Some("dev"),
            None,
            None,
            "git+https://github.com/water-rs/waterui?branch=dev#abc123",
            true,
        ),
        // A different declared branch.
        (
            "https://github.com/water-rs/waterui",
            Some("main"),
            None,
            None,
            "git+https://github.com/water-rs/waterui?branch=dev#abc123",
            false,
        ),
        // Same declared tag.
        (
            "https://github.com/water-rs/waterui",
            None,
            Some("v1"),
            None,
            "git+https://github.com/water-rs/waterui?tag=v1#abc123",
            true,
        ),
        // Same declared rev.
        (
            "https://github.com/water-rs/waterui",
            None,
            None,
            Some("release"),
            "git+https://github.com/water-rs/waterui?rev=release#abc123",
            true,
        ),
        // `release-candidate` is not `release`.
        (
            "https://github.com/water-rs/waterui",
            None,
            None,
            Some("release-candidate"),
            "git+https://github.com/water-rs/waterui?rev=release#abc123",
            false,
        ),
        // Same name at a foreign repository.
        (
            "https://github.com/other/fork",
            Some("dev"),
            None,
            None,
            "git+https://github.com/water-rs/waterui?branch=dev#abc123",
            false,
        ),
        // `.git` suffix and trailing slash canonicalize away.
        (
            "https://github.com/water-rs/waterui.git",
            Some("dev"),
            None,
            None,
            "git+https://github.com/water-rs/waterui?branch=dev#abc123",
            true,
        ),
        (
            "https://github.com/water-rs/waterui/",
            Some("dev"),
            None,
            None,
            "git+https://github.com/water-rs/waterui?branch=dev#abc123",
            true,
        ),
    ];

    #[test]
    fn apple_source_patches_matches_git_references() {
        let root = tempfile::tempdir().unwrap();
        for (git, branch, tag, rev, source, expected) in GIT_CASES {
            let metadata = metadata_json(&[package_json("dep", "1.0.0", "/x/dep", Some(source))]);
            let patches = set(&[(
                "crates-io",
                "dep",
                detail(DependencyDetail {
                    git: Some((*git).to_owned()),
                    branch: branch.map(String::from),
                    tag: tag.map(String::from),
                    rev: rev.map(String::from),
                    ..Default::default()
                }),
            )]);
            let projected = smol::block_on(super::resolved_source_patches(
                &patches,
                &metadata,
                root.path(),
            ))
            .unwrap();
            assert_eq!(
                !projected.is_empty(),
                *expected,
                "row git={git} branch={branch:?} tag={tag:?} rev={rev:?} vs source {source}"
            );
        }
    }

    #[test]
    fn apple_source_patches_keeps_registry_rows_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let metadata = metadata_json(&[package_json("dep", "1.0.0", "/x", None)]);
        let registry = set(&[
            (
                "crates-io",
                "dep",
                Dependency::Simple(cargo_metadata::semver::VersionReq::parse("1").unwrap()),
            ),
            (
                "crates-io",
                "unpinned",
                detail(DependencyDetail {
                    version: Some(cargo_metadata::semver::VersionReq::parse("1").unwrap()),
                    ..Default::default()
                }),
            ),
        ]);
        let projected = smol::block_on(super::resolved_source_patches(
            &registry,
            &metadata,
            root.path(),
        ))
        .unwrap();
        assert_eq!(projected, registry, "registry-only rows are unchanged");
    }

    /// A git-resolved package's checkout never satisfies a `path` row.
    #[test]
    fn apple_source_patches_git_checkout_never_satisfies_a_path_row() {
        let root = tempfile::tempdir().unwrap();
        let shared = root.path().join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let metadata = metadata_json(&[package_json(
            "dep",
            "1.0.0",
            shared.to_str().unwrap(),
            Some("git+https://github.com/water-rs/waterui?branch=dev#abc123"),
        )]);
        let patches = set(&[(
            "crates-io",
            "dep",
            detail(DependencyDetail {
                path: Some(shared.to_str().unwrap().to_owned()),
                ..Default::default()
            }),
        )]);
        assert!(
            smol::block_on(super::resolved_source_patches(
                &patches,
                &metadata,
                root.path()
            ))
            .unwrap()
            .is_empty()
        );
    }

    /// The merge winner's row is the one projected, verbatim.
    #[test]
    fn apple_source_patches_projects_the_merge_winner_verbatim() {
        let root = tempfile::tempdir().unwrap();
        let sdk_dir = root.path().join("sdk-dep");
        let user_dir = root.path().join("user-dep");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        std::fs::create_dir_all(&user_dir).unwrap();
        let sdk_row = detail(DependencyDetail {
            path: Some(sdk_dir.to_str().unwrap().to_owned()),
            ..Default::default()
        });
        let user_row = detail(DependencyDetail {
            path: Some(user_dir.to_str().unwrap().to_owned()),
            ..Default::default()
        });
        let merged = super::merge(
            set(&[("crates-io", "dep", sdk_row)]),
            set(&[("crates-io", "dep", user_row.clone())]),
        );
        let metadata = metadata_json(&[package_json(
            "dep",
            "1.0.0",
            user_dir.to_str().unwrap(),
            None,
        )]);
        let projected = smol::block_on(super::resolved_source_patches(
            &merged,
            &metadata,
            root.path(),
        ))
        .unwrap();
        assert_eq!(projected["crates-io"]["dep"], user_row);
        assert_eq!(projected["crates-io"].len(), 1, "the SDK's row is gone");
    }

    #[test]
    fn apple_source_patches_errors_at_the_input_boundaries() {
        let root = tempfile::tempdir().unwrap();

        // A patch path that does not exist is an error naming the path.
        let missing = root.path().join("nonexistent");
        let metadata = metadata_json(&[package_json("dep", "1.0.0", "/x", None)]);
        let patches = set(&[(
            "crates-io",
            "dep",
            detail(DependencyDetail {
                path: Some(missing.to_str().unwrap().to_owned()),
                ..Default::default()
            }),
        )]);
        let error = smol::block_on(super::resolved_source_patches(
            &patches,
            &metadata,
            root.path(),
        ))
        .unwrap_err();
        assert!(error.to_string().contains("nonexistent"), "{error}");

        // A metadata document without a resolve is invalid input.
        let no_resolve = serde_json::from_value::<cargo_metadata::Metadata>(serde_json::json!({
            "packages": [], "workspace_members": [], "workspace_default_members": [],
            "resolve": null, "target_directory": "/t", "build_directory": "/t",
            "version": 1, "workspace_root": "/t", "metadata": null
        }))
        .unwrap();
        let error = smol::block_on(super::resolved_source_patches(
            &PatchSet::new(),
            &no_resolve,
            root.path(),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        // A package source Cargo emitted but SourceId cannot parse.
        let bad_source = metadata_json(&[package_json(
            "dep",
            "1.0.0",
            "/x/dep",
            Some("bogus+not-a-url"),
        )]);
        let error = smol::block_on(super::resolved_source_patches(
            &PatchSet::new(),
            &bad_source,
            root.path(),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    fn git(rev: &str) -> Dependency {
        Dependency::Detailed(Box::new(DependencyDetail {
            git: Some("https://github.com/water-rs/waterkit".to_owned()),
            rev: Some(rev.to_owned()),
            ..DependencyDetail::default()
        }))
    }

    fn path(path: &str) -> Dependency {
        Dependency::Detailed(Box::new(DependencyDetail {
            path: Some(path.to_owned()),
            ..DependencyDetail::default()
        }))
    }

    fn set(entries: &[(&str, &str, Dependency)]) -> PatchSet {
        let mut set = PatchSet::new();
        for (registry, name, dependency) in entries {
            set.entry((*registry).to_owned())
                .or_default()
                .insert((*name).to_owned(), dependency.clone());
        }
        set
    }

    #[test]
    fn the_projects_entries_join_every_source_of_the_frameworks() {
        let framework = set(&[
            ("crates-io", "waterui-core", path("/checkout/core")),
            (
                "https://github.com/water-rs/waterui",
                "waterui-core",
                path("/checkout/core"),
            ),
        ]);
        let project = set(&[
            ("crates-io", "waterkit-clipboard", git("abc")),
            ("https://github.com/water-rs/fork", "forked", path("/fork")),
        ]);
        let merged = super::merge(framework, project);
        assert_eq!(
            merged,
            set(&[
                ("crates-io", "waterui-core", path("/checkout/core")),
                ("crates-io", "waterkit-clipboard", git("abc")),
                (
                    "https://github.com/water-rs/waterui",
                    "waterui-core",
                    path("/checkout/core")
                ),
                ("https://github.com/water-rs/fork", "forked", path("/fork")),
            ])
        );
    }

    #[test]
    fn an_entry_both_owners_spell_the_same_way_is_kept_once() {
        let framework = set(&[("crates-io", "waterui-core", path("/checkout/core"))]);
        let project = set(&[("crates-io", "waterui-core", path("/app/../checkout/./core"))]);
        let merged = super::merge(framework.clone(), project);
        assert_eq!(merged, framework);
    }

    #[test]
    fn the_projects_entry_overrides_the_frameworks_for_the_same_crate() {
        let framework = set(&[
            ("crates-io", "waterkit-clipboard", git("framework-rev")),
            ("crates-io", "waterkit-core", git("framework-rev")),
        ]);
        let project = set(&[("crates-io", "waterkit-clipboard", git("project-rev"))]);
        assert_eq!(
            super::merge(framework, project),
            set(&[
                ("crates-io", "waterkit-clipboard", git("project-rev")),
                ("crates-io", "waterkit-core", git("framework-rev")),
            ])
        );
    }

    #[test]
    fn without_drops_the_named_crates_of_the_same_source_only() {
        let full = set(&[
            ("crates-io", "waterkit-clipboard", git("a")),
            ("crates-io", "waterkit-core", git("a")),
            (
                "https://github.com/water-rs/waterui",
                "waterkit-clipboard",
                git("a"),
            ),
        ]);
        let names = set(&[("crates-io", "waterkit-clipboard", git("other"))]);
        assert_eq!(
            super::without(&full, &names),
            set(&[
                ("crates-io", "waterkit-core", git("a")),
                (
                    "https://github.com/water-rs/waterui",
                    "waterkit-clipboard",
                    git("a")
                ),
            ])
        );
    }

    #[test]
    fn the_same_crate_under_another_source_is_not_an_override() {
        let framework = set(&[("crates-io", "waterui-core", path("/checkout/core"))]);
        let project = set(&[(
            "https://github.com/water-rs/waterui",
            "waterui-core",
            path("/elsewhere/core"),
        )]);
        let merged = super::merge(framework.clone(), project);
        assert_eq!(merged.len(), 2, "different sources are different tables");
        assert_eq!(merged["crates-io"], framework["crates-io"]);
    }

    #[test]
    fn beyond_keeps_what_the_written_set_does_not_hold_verbatim() {
        let written = set(&[
            ("crates-io", "waterui-core", path("../waterui/core")),
            ("crates-io", "waterkit-clipboard", git("written")),
        ]);
        let current = set(&[
            ("crates-io", "waterui-core", path("../waterui/core")),
            ("crates-io", "waterkit-clipboard", git("edited")),
            ("crates-io", "nami-derive", git("own")),
        ]);
        assert_eq!(
            super::beyond(&current, &written),
            set(&[
                ("crates-io", "waterkit-clipboard", git("edited")),
                ("crates-io", "nami-derive", git("own")),
            ])
        );
    }
}

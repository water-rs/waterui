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

use std::path::Path;

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

//! Reports which workspace packages a diff between two revisions affects.
//!
//! The PR/push gate runs `affected --base <rev>` from the repository root
//! (the checkout standing at the head revision) and reads one JSON object
//! from stdout:
//!
//! ```json
//! {
//!   "merge_base": "…",
//!   "workspace": false,
//!   "affected": ["waterui-core"],
//!   "owners": {"core/src/lib.rs": "waterui-core"},
//!   "test_asset_consumers": ["hydrolysis", "waterui-testing"]
//! }
//! ```
//!
//! `affected` is what the determinator marks — the packages owning a
//! changed file plus every package whose build could change because a
//! dependency did — as the `-p` list the gate checks and lints. `workspace`
//! is true when a changed path either matched a `mark-changed = "all"`
//! rule in `rules.toml` or matched no package at all, where enumerating
//! packages would pretend precision the diff does not have.
//!
//! `owners` maps every changed path to the package that owns it by
//! directory (`null` when a rule consumed it). The comment-only lane in
//! `affected.py` reads the owners of the `.rs` files it classified: the
//! layout gate's tree comparison needs the owning crates, and the owning
//! crate is a fact about the path, not about the diff — so it comes from
//! this tool's package graph rather than a second, hand-rolled mapper.
//!
//! `test_asset_consumers` is the set of workspace members whose
//! `cargo check -p <pkg> --all-targets` compiles a generated, uncommitted
//! test asset. Most members qualify through the package graph: any member
//! whose dev-dependency closure reaches `waterui-testing` (the only
//! enabler of `hydrolysis/testing`, the feature whose `TEST_FONTS`
//! `include_bytes!` the generated fonts), plus `waterui-testing` itself.
//! Dev edges count only at the first hop, matching cargo's rule that
//! dev-dependencies do not chain. `waterui-cli` is the one member named
//! directly: its `#[cfg(test)]` font-subsetting module `include_bytes!`s
//! `cli/tests/fixtures/fonts/Roboto-Regular.ttf`, which the
//! generate-test-assets composite regenerates — a path membership, not a
//! dependency edge, so the graph cannot derive it.
//!
//! The root package (`waterui`, manifest at the repository root) gets one
//! correction on top of ancestor matching: its package directory is the
//! repository root, so every unmatched path resolves to it — `Clippy.toml`,
//! `deny.toml`, `tests/layout-twins/**` and any future top-level file would
//! select only `waterui` plus its reverse dependencies. A path that
//! ancestor-matches only the root package counts as package-owned when it
//! is one of its declared target files (`facade.rs`, `tests/*.rs`) or sits
//! directly inside a directory holding one (`tests/`); anything else is
//! reported as unmatched, which selects the whole workspace.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use determinator::{rules::DeterminatorRules, rules::PathMatch, Determinator};
use guppy::{
    graph::{DependencyDirection, PackageGraph},
    CargoMetadata, PackageId,
};

const RULES_TOML: &str = include_str!("../rules.toml");

fn git(args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args[0]));
    if !output.status.success() {
        panic!(
            "git {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output.stdout
}

fn cargo_metadata(manifest_path: &Utf8PathBuf, locked: bool) -> CargoMetadata {
    let mut command = Command::new("cargo");
    command
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(manifest_path);
    if locked {
        command.arg("--locked");
    }
    let output = command.output().expect("cargo metadata failed to start");
    if !output.status.success() {
        panic!(
            "cargo metadata on {manifest_path} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let json = String::from_utf8(output.stdout).expect("cargo metadata output is not UTF-8");
    CargoMetadata::parse_json(&json).expect("cargo metadata output did not parse")
}

/// The base revision checked out as a linked worktree: `cargo metadata`
/// needs the whole tree (path dependencies), not the manifest alone.
struct BaseWorktree(Utf8PathBuf);

impl BaseWorktree {
    fn add(revision: &str) -> Self {
        let path = std::env::temp_dir().join(format!("affected-base-{}", std::process::id()));
        let path = Utf8PathBuf::from_path_buf(path).expect("temp dir is not UTF-8");
        git(&[
            "worktree",
            "add",
            "--detach",
            "--quiet",
            path.as_str(),
            revision,
        ]);
        Self(path)
    }
}

impl Drop for BaseWorktree {
    fn drop(&mut self) {
        let removed = Command::new("git")
            .args(["worktree", "remove", "--force", self.0.as_str()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !removed {
            let _ = Command::new("git").args(["worktree", "prune"]).status();
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn parse_args() -> (String, String) {
    let mut base = None;
    let mut head: String = "HEAD".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_ref() {
            "--base" => base = Some(args.next().expect("--base needs a value")),
            "--head" => head = args.next().expect("--head needs a value"),
            other => panic!("unknown argument: {other}"),
        }
    }
    (base.expect("--base <rev> is required"), head)
}

/// What the root package itself can own: its declared target files
/// (`facade.rs`, `tests/avatar.rs`, …) plus the directories that directly
/// hold them (`tests/`), matching the depth cargo autodiscovery scans.
/// `tests/layout-twins/**` fails both — it nests one level deeper.
struct RootOwnership<'a> {
    package_id: &'a PackageId,
    files: BTreeSet<Utf8PathBuf>,
    dirs: BTreeSet<Utf8PathBuf>,
}

impl<'a> RootOwnership<'a> {
    fn new(graph: &'a PackageGraph) -> Self {
        let workspace_root = graph.workspace().root();
        let members = graph.query_workspace().resolve();
        let Some(root_package) = members
            .packages(DependencyDirection::Forward)
            .find(|package| package.manifest_path().parent() == Some(workspace_root))
        else {
            panic!("no workspace package manifests at the root");
        };
        let mut files = BTreeSet::new();
        let mut dirs = BTreeSet::new();
        for target in root_package.build_targets() {
            let relative = target
                .path()
                .strip_prefix(workspace_root)
                .expect("root package target outside the workspace")
                .to_path_buf();
            if let Some(parent) = relative.parent() {
                if !parent.as_str().is_empty() {
                    dirs.insert(parent.to_path_buf());
                }
            }
            files.insert(relative);
        }
        Self {
            package_id: root_package.id(),
            files,
            dirs,
        }
    }

    /// Whether the root package's own sources include `path` — a declared
    /// target file or a sibling sitting directly inside a target directory.
    fn owns(&self, path: &Utf8Path) -> bool {
        self.files.contains(path)
            || path
                .parent()
                .is_some_and(|parent| self.dirs.contains(parent))
    }
}

/// The workspace members whose `--all-targets` build compiles a generated,
/// uncommitted test asset. A member qualifies when a dev-dependency of its
/// own reaches `waterui-testing` — the only crate that enables
/// `hydrolysis/testing`, whose `TEST_FONTS` `include_bytes!` the generated
/// fonts — through normal/build links; dev edges are first-hop only
/// because cargo does not make dev-dependencies transitive.
/// `waterui-testing` itself is a consumer: checking it compiles the dep.
/// `waterui-cli` needs the same treatment for the font its own `#[cfg(test)]`
/// code `include_bytes!`s from `cli/tests/fixtures/fonts/`: that is a path
/// membership, not a dependency edge, so it is named here instead of being
/// derived.
fn test_asset_consumers(graph: &PackageGraph) -> BTreeSet<String> {
    let members: Vec<_> = graph
        .query_workspace()
        .resolve()
        .packages(DependencyDirection::Forward)
        .collect();
    let Some(testing) = members.iter().find(|p| p.name() == "waterui-testing") else {
        return BTreeSet::new();
    };
    let testing_id = testing.id().clone();
    let mut consumers = BTreeSet::new();
    for member in &members {
        let mut seeds: BTreeSet<&PackageId> = member
            .direct_links()
            .filter(|link| link.dev().is_present())
            .map(|link| link.to().id())
            .collect();
        seeds.insert(member.id());
        // Forward closure over normal and build links only.
        let mut reached: BTreeSet<&PackageId> = seeds.iter().copied().collect();
        let mut stack: Vec<&PackageId> = seeds.iter().copied().collect();
        while let Some(id) = stack.pop() {
            for link in graph.metadata(id).expect("known id").direct_links() {
                if (link.normal().is_present() || link.build().is_present())
                    && reached.insert(link.to().id())
                {
                    stack.push(link.to().id());
                }
            }
        }
        if reached.contains(&testing_id) {
            consumers.insert(member.name().to_string());
        }
    }
    if members.iter().any(|p| p.name() == "waterui-cli") {
        consumers.insert("waterui-cli".to_string());
    }
    consumers
}

/// The package a path would ancestor-match to, using the same
/// `member_by_path` walk the determinator performs after its rules: the
/// first ancestor directory that is a workspace member's source
/// directory. Needed for the rule-carve-out — a mark-nothing rule may
/// still consume a file that lives inside a package (prose globs cannot
/// express "outside every package" in globset syntax, where `*` crosses
/// separators), and in-package files belong to their package because
/// `include_str!` and `#![doc]` compile them into the crate.
fn ancestor_owner(graph: &PackageGraph, path: &Utf8Path) -> Option<PackageId> {
    for ancestor in path.ancestors() {
        if let Ok(package) = graph.workspace().member_by_path(ancestor) {
            return Some(package.id().clone());
        }
    }
    None
}

fn main() {
    let (base, head) = parse_args();

    // The merge-base is the honest comparison point: a pull request's
    // reported base sha may sit behind commits the branch already merged.
    let merge_base = String::from_utf8(git(&["merge-base", &base, &head]))
        .expect("merge-base output is not UTF-8")
        .trim()
        .to_string();

    let changed: Vec<String> = git(&["diff", "--name-only", "-z", &merge_base, &head])
        .split(|&byte| byte == 0)
        .filter(|token| !token.is_empty())
        .map(|token| String::from_utf8(token.to_vec()).expect("path is not UTF-8"))
        .collect();

    let base_worktree = BaseWorktree::add(&merge_base);
    let old_graph = cargo_metadata(&base_worktree.0.join("Cargo.toml"), false)
        .build_graph()
        .expect("the base revision's package graph did not resolve");
    let new_graph = cargo_metadata(&Utf8PathBuf::from("Cargo.toml"), true)
        .build_graph()
        .expect("the head revision's package graph did not resolve");

    let rules = DeterminatorRules::parse(RULES_TOML).expect("rules.toml did not parse");
    let mut determinator = Determinator::new(&old_graph, &new_graph);
    determinator
        .set_rules(&rules)
        .expect("rules.toml does not resolve against this workspace");
    determinator.add_changed_paths(changed.iter().map(String::as_str));

    let root = RootOwnership::new(&new_graph);
    let test_assets = test_asset_consumers(&new_graph);

    // A path that matched nothing, matched only the root package by
    // directory without being part of its declared sources, or hit a rule
    // that marks everything mean the same thing to the gate: the diff can
    // reach anywhere, so report `workspace` rather than a list that happens
    // to equal it.
    let mut workspace = false;
    let mut owners: BTreeMap<String, Option<String>> = BTreeMap::new();
    // Packages the rules swallowed but that own the file anyway: the
    // determinator's own set cannot see them, so they are unioned into
    // `affected` below. Reverse dependencies are not pulled in — an
    // include_str'd document compiles into its owning crate alone.
    let mut manual_affected: BTreeSet<String> = BTreeSet::new();
    for path in &changed {
        let mut matched: Vec<&PackageId> = Vec::new();
        let verdict = determinator.match_path(path.as_str(), |id| matched.push(id));
        match verdict {
            PathMatch::RuleMatchedAll | PathMatch::NoMatches => {
                workspace = true;
                owners.insert(path.clone(), None);
            }
            PathMatch::RuleMatched(_) => {
                // A rule swallowed the path. When it lives inside a real
                // package anyway — `components/…/instructions.md` matched
                // by `*.md`, say — it still belongs to that package:
                // prose carve-outs apply outside packages only.
                match ancestor_owner(&new_graph, Utf8Path::new(path.as_str())) {
                    Some(id)
                        if id != *root.package_id || root.owns(Utf8Path::new(path.as_str())) =>
                    {
                        let name = new_graph
                            .metadata(&id)
                            .expect("ancestor match is a known package")
                            .name()
                            .to_string();
                        manual_affected.insert(name.clone());
                        owners.insert(path.clone(), Some(name));
                    }
                    _ => {
                        owners.insert(path.clone(), None);
                    }
                }
            }
            PathMatch::AncestorMatched => {
                let owned: Vec<&PackageId> = matched
                    .iter()
                    .copied()
                    .filter(|id| {
                        **id != *root.package_id || root.owns(Utf8Path::new(path.as_str()))
                    })
                    .collect();
                if owned.is_empty() {
                    workspace = true;
                    owners.insert(path.clone(), None);
                } else {
                    let name = new_graph
                        .metadata(owned[0])
                        .expect("ancestor match is a known package")
                        .name()
                        .to_string();
                    owners.insert(path.clone(), Some(name));
                }
            }
        }
    }

    let set = determinator.compute();
    let mut affected: Vec<String> = set
        .affected_set
        .packages(DependencyDirection::Forward)
        .map(|metadata| metadata.name().to_string())
        .collect();
    affected.extend(manual_affected);
    affected.sort_unstable();
    affected.dedup();

    println!(
        "{}",
        serde_json::json!({
            "workspace": workspace,
            "merge_base": merge_base,
            "affected": affected,
            "owners": owners,
            "test_asset_consumers": test_assets,
        })
    );
}

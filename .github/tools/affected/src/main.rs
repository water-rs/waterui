//! Reports which workspace packages a diff between two revisions affects.
//!
//! The PR/push gate runs `affected --base <rev>` from the repository root
//! (the checkout standing at the head revision) and reads one JSON object
//! from stdout:
//!
//! ```json
//! {"merge_base": "…", "workspace": false, "affected": ["waterui-core"]}
//! ```
//!
//! `affected` is what the determinator marks — the packages owning a
//! changed file plus every package whose build could change because a
//! dependency did — as the `-p` list the gate checks and lints. `workspace`
//! is true when a changed path either matched a `mark-changed = "all"`
//! rule in `rules.toml` or matched nothing at all, where enumerating
//! packages would pretend precision the diff does not have.

use std::process::Command;

use camino::Utf8PathBuf;
use determinator::{rules::DeterminatorRules, rules::PathMatch, Determinator};
use guppy::{graph::DependencyDirection, CargoMetadata};

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
    let mut head = "HEAD".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base" => base = Some(args.next().expect("--base needs a value")),
            "--head" => head = args.next().expect("--head needs a value"),
            other => panic!("unknown argument: {other}"),
        }
    }
    (base.expect("--base <rev> is required"), head)
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

    // A path that matched nothing and a rule that marks everything mean the
    // same thing to the gate: the diff can reach anywhere, so report
    // `workspace` rather than a list that happens to equal it.
    let workspace = changed.iter().any(|path| {
        matches!(
            determinator.match_path(path, |_| {}),
            PathMatch::RuleMatchedAll | PathMatch::NoMatches
        )
    });

    let set = determinator.compute();
    let mut affected: Vec<String> = set
        .affected_set
        .packages(DependencyDirection::Forward)
        .map(|metadata| metadata.name().to_string())
        .collect();
    affected.sort_unstable();

    println!(
        "{}",
        serde_json::json!({
            "workspace": workspace,
            "merge_base": merge_base,
            "affected": affected,
        })
    );
}

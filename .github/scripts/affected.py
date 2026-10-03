# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "tree-sitter==0.26.0",
#     "tree-sitter-rust==0.24.2",
# ]
# ///
"""Compute what the PR/push gate must run for a diff.

Reads the diff between the given base revision and `HEAD`, then writes
`GITHUB_OUTPUT` keys:

- `packages` — the crates the gate checks and lints: space-separated
  package names, or the literal `workspace` when the diff can reach any
  crate (an outside-package path the rules do not map selects everything,
  per `.github/tools/affected/rules.toml`). Empty when nothing can be
  affected — a prose-only change still gets `cargo fmt`, nothing more.
- `package-args` — `packages` rendered as `-p` flags for cargo.
- `comment-only` — `true` when every changed file is either prose or a
  `.rs` file whose syntax tree is unchanged once comments, doc comments
  and doc attributes are removed (the `rust_semantic_diff` comparison the
  layout gate also runs). The gate then lints and builds rustdoc for only
  the crates owning those files — never their reverse dependencies — and
  MSRV stays off unless the diff moves a dependency boundary on its own.
- `owners` — space-separated package names owning the changed `.rs`
  files (meaningful when `comment-only` is true).
- `msrv` — `true` when the diff can move the toolchain floor: a
  `rust-version` key, any Cargo.toml dependency section, or Cargo.lock.
- `test-assets` — `true` when hydrolysis or a reverse dependency of it is
  in scope (their test/dev code compiles the generated fonts), or the
  whole workspace is.

Usage:

    affected.py --base <rev> [--tool .github/tools/affected/target/release/affected]

Any uncertainty — an unresolvable base, an unparsable manifest — widens
the gate to `workspace`; it never narrows it on a guess.
"""

import argparse
import fnmatch
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

from rust_semantic_diff import changed_entries, git, semantic_differs, source_at

# Cargo.toml tables whose content can move the dependency graph or the
# toolchain floor. Compared by parsed value, not text, so a comment or
# formatting edit elsewhere in the manifest does not re-arm MSRV.
DEPENDENCY_TABLES = (
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "workspace.dependencies",
    "patch",
)


def is_prose(path):
    """Whether `path` is documentation or repository prose that no compile
    can read — the same set the `code` path filter treats as 'not code':
    `!**/*.md`, `!docs/**`, `!LICENSE*`, `!.github/ISSUE_TEMPLATE/**`."""
    if fnmatch.fnmatch(path, "*.md") or fnmatch.fnmatch(path, "**/*.md"):
        return True
    if fnmatch.fnmatch(path, "docs/**"):
        return True
    if fnmatch.fnmatch(path, "LICENSE*"):
        return True
    return fnmatch.fnmatch(path, ".github/ISSUE_TEMPLATE/**")


def manifest_tables(source):
    """The MSRV-relevant content of a Cargo.toml: `rust-version` keys and
    every dependency-bearing table, parsed so layout and comments drop
    out. `None` when the manifest does not parse — the caller treats that
    as 'could matter' rather than guessing."""
    try:
        manifest = tomllib.loads(source)
    except tomllib.TOMLDecodeError:
        return None

    def flatten(value, prefix=()):
        if not isinstance(value, dict):
            return {prefix: value}
        result = {}
        for key, child in value.items():
            result.update(flatten(child, prefix + (key,)))
        return result

    relevant = {}
    for section, body in flatten(manifest).items():
        top = section[0]
        if top in DEPENDENCY_TABLES or section[-1] == "rust-version":
            relevant[section] = body
    return relevant


def msrv_relevant(base, head, entries):
    """Whether the diff can move the toolchain floor: Cargo.lock, or a
    Cargo.toml whose `rust-version` or dependency tables changed. A parse
    failure or an added/deleted manifest counts as relevant — the gate
    widens rather than guesses."""
    for entry in entries:
        path = entry[-1]
        if path == "Cargo.lock":
            return True
        if not path.endswith("Cargo.toml"):
            continue
        if entry[0].startswith(("A", "D", "R", "C")):
            return True
        old_tables = manifest_tables(source_at(base, path))
        new_tables = manifest_tables(source_at(head, path))
        if old_tables is None or new_tables is None or old_tables != new_tables:
            return True
    return False


def owner_packages(paths):
    """The workspace package owning each path — the nearest ancestor
    manifest, from cargo's own metadata rather than a hand-rolled walk."""
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"], text=True
        )
    )
    root = Path(metadata["workspace_root"])
    members = set(metadata["workspace_members"])
    manifest_dirs = {
        package["name"]: str(Path(package["manifest_path"]).parent.relative_to(root))
        for package in metadata["packages"]
        if package["id"] in members
    }
    owners = set()
    for path in paths:
        best = max(
            (
                (name, directory)
                for name, directory in manifest_dirs.items()
                if path.startswith(directory + "/")
            ),
            key=lambda entry: len(entry[1]),
            default=None,
        )
        if best:
            owners.add(best[0])
    return owners


def emit(outputs):
    print(json.dumps(outputs))
    output_file = os.environ.get("GITHUB_OUTPUT")
    if output_file:
        with open(output_file, "a") as handle:
            for key, value in outputs.items():
                handle.write(f"{key}={value}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="base revision or sha")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument(
        "--tool",
        default=".github/tools/affected/target/release/affected",
        help="path to the built determinator binary",
    )
    args = parser.parse_args()

    outputs = {
        "packages": "workspace",
        "package-args": "",
        "comment-only": "false",
        "owners": "",
        "msrv": "true",
        "test-assets": "true",
    }
    try:
        base = git("merge-base", args.base, args.head).strip()
        entries = changed_entries(base, args.head)
        msrv = msrv_relevant(base, args.head, entries)

        # The comment-only classification: a modified .rs file is
        # comment-only when its normalised tree is unchanged; prose does
        # not widen the gate; anything else does.
        commentable_paths = []
        has_rs_change = False
        for entry in entries:
            status = entry[0]
            paths = entry[1:] if status.startswith(("R", "C")) else [entry[1]]
            for path in paths:
                if status == "M" and path.endswith(".rs"):
                    has_rs_change = True
                elif not is_prose(path):
                    commentable_paths = None
                    break
            if commentable_paths is None:
                break
            if status == "M" and paths[-1].endswith(".rs"):
                commentable_paths.append(paths[-1])

        comment_only = (
            commentable_paths is not None
            and has_rs_change
            and all(
                not semantic_differs(source_at(base, path), source_at(args.head, path))
                for path in commentable_paths
            )
        )

        if not entries:
            outputs.update(
                {
                    "packages": "",
                    "msrv": "false",
                    "test-assets": "false",
                }
            )
        elif comment_only:
            owners = sorted(owner_packages(commentable_paths))
            outputs.update(
                {
                    "packages": " ".join(owners),
                    "package-args": " ".join(f"-p {name}" for name in owners),
                    "comment-only": "true",
                    "owners": " ".join(owners),
                    "msrv": "true" if msrv else "false",
                    "test-assets": "true" if "hydrolysis" in owners else "false",
                }
            )
        else:
            report = json.loads(
                subprocess.check_output(
                    [args.tool, "--base", base, "--head", args.head], text=True
                )
            )
            if report["workspace"]:
                outputs["msrv"] = "true" if msrv else "false"
            else:
                affected = report["affected"]
                outputs.update(
                    {
                        "packages": " ".join(affected),
                        "package-args": " ".join(f"-p {name}" for name in affected),
                        "msrv": "true" if msrv else "false",
                        "test-assets": "true"
                        if {
                            "hydrolysis",
                            "hydrolysis-android-test-app",
                            "waterui-testing",
                        }
                        & set(affected)
                        else "false",
                    }
                )
    except Exception as error:  # widen on any failure — never scope on a guess
        print(
            f"::warning::affected.py fell back to the whole workspace: {error}",
            file=sys.stderr,
        )

    emit(outputs)


if __name__ == "__main__":
    main()

# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "tree-sitter==0.26.0",
#     "tree-sitter-rust==0.24.2",
# ]
# ///
"""Compute what the PR/push gate must run for a diff.

Reads the diff between the given base revision and `HEAD`, runs the
determinator tool on it, then writes `GITHUB_OUTPUT` keys:

- `packages` — the crates the gate checks and lints: space-separated
  package names, or the literal `workspace` when the diff can reach any
  crate (an outside-package path the rules do not map, or a path that
  directory-matches only the root package without belonging to it, selects
  everything — see the tool's module doc). Empty when nothing can be
  affected — a prose-only change still gets `cargo fmt`, nothing more.
- `package-args` — `packages` rendered as `-p` flags for cargo.
- `comment-only` — `true` when every changed file is either prose or a
  `.rs` file whose syntax tree is unchanged once comments, doc comments
  and doc attributes are removed (the `rust_semantic_diff` comparison the
  layout gate also runs). The gate then lints and builds rustdoc for only
  the crates owning those files — never their reverse dependencies — and
  MSRV stays off unless the diff moves a dependency boundary on its own.
- `msrv` — `true` when the diff can move the toolchain floor: a
  `rust-version` key, any Cargo.toml dependency section (including
  `target.<cfg>.dependencies`), or Cargo.lock.
- `scene-assets` — `true` when a package whose `--all-targets` compile
  `include_bytes!`s a generated Cherenkov scene font is in scope, or the
  whole workspace is. That is a fixed owner set, not a dev-dependency
  closure: `cherenkov-oracle`'s glyph tests, `cherenkov-gpu`'s
  bitmap/browser test targets, and `cherenkov-cpu`'s bitmap unit tests
  `include_bytes!` `scenes/fonts/*`. Test-only code never compiles for a
  dependent, so `SCENE_ASSET_OWNERS` names them directly. `cherenkov-cpu`'s
  integration tests and `cherenkov-bench` read the corpus at run time and
  are not in the set: the gate never runs tests. Jobs that execute those
  tests generate the corpus themselves.
- `apple-legs` — JSON list of test.yml's `macos` matrix entries: the Apple
  lint targets with an Apple-gated crate in scope (`apple_gated.py`),
  `[]` when there is none, so no macOS runner starts to find nothing.

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

from apple_gated import legs as apple_legs
from rust_semantic_diff import changed_entries, git, semantic_differs, source_at

# Cargo.toml tables whose content can move the dependency graph or the
# toolchain floor. Compared by parsed value, not text, so a comment or
# formatting edit elsewhere in the manifest does not re-arm MSRV.
# `[target.<cfg>.dependencies]` sections flatten to ("target", <cfg>,
# <kind>, <name>) — caught by the `target` + `*dependencies` test below.
DEPENDENCY_TABLES = (
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "workspace.dependencies",
    "patch",
)

# The packages whose `--all-targets` compile `include_bytes!`s a file the
# Cherenkov scene generator writes (its test targets embed
# `scenes/fonts/*` subsets). The gate's `scene-assets` output is this set
# intersected with the scoped package names: test-only code never
# compiles for a dependent, so no dev-dependency closure is involved.
SCENE_ASSET_OWNERS = {"cherenkov-oracle", "cherenkov-gpu", "cherenkov-cpu"}


def is_prose(path):
    """Whether `path` is documentation or repository prose that no compile
    can read — the same set the `code` path filter treats as 'not code'.

    Scoped to paths *outside* every package: a markdown file inside a
    package directory is not prose — `include_str!` and
    `#![doc = include_str!(…)]` compile it into the crate, so the
    determinator's ancestor matching must still see it. The crate-side
    examples are `#![doc = include_str!("../README.md")]` in several
    components and `include_str!("instructions.md")` in the mcp protocol
    crate."""
    if "/" not in path and fnmatch.fnmatch(path, "*.md"):
        return True
    if "/" not in path and (
        fnmatch.fnmatch(path, "LICENSE*")
        or fnmatch.fnmatch(path, "README*")
        or fnmatch.fnmatch(path, "CONTRIBUTING*")
        or fnmatch.fnmatch(path, "CODE_OF_CONDUCT*")
        or fnmatch.fnmatch(path, "SECURITY*")
    ):
        return True
    if fnmatch.fnmatch(path, "docs/**"):
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
        is_dependency = top in DEPENDENCY_TABLES or (
            top == "target"
            and any(part.endswith("dependencies") for part in section[1:])
        )
        if is_dependency or section[-1] == "rust-version":
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
        "msrv": "true",
        "apple-legs": json.dumps(apple_legs("workspace"), separators=(",", ":")),
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

        # The tool always runs: its `owners` map is the only path→crate
        # mapper (the comment-only lane reads it).
        report = json.loads(
            subprocess.check_output(
                [args.tool, "--base", base, "--head", args.head], text=True
            )
        ) if entries else None

        def scene_assets_for(names):
            return "true" if SCENE_ASSET_OWNERS & set(names) else "false"

        def apple_legs_for(scope):
            return json.dumps(apple_legs(scope), separators=(",", ":"))

        if not entries:
            outputs.update(
                {
                    "packages": "",
                    "msrv": "false",
                    "scene-assets": "false",
                    "apple-legs": "[]",
                }
            )
        elif report["workspace"]:
            # The diff can reach anywhere — workspace in scope keeps every
            # signal on, and generated assets must exist for it.
            outputs.update(
                {
                    "packages": "workspace",
                    "msrv": "true" if msrv else "false",
                    "scene-assets": "true",
                    "apple-legs": apple_legs_for("workspace"),
                }
            )
        elif comment_only:
            owners = sorted(
                {
                    report["owners"].get(path)
                    for path in commentable_paths
                    if report["owners"].get(path)
                }
            )
            outputs.update(
                {
                    "packages": " ".join(owners),
                    "package-args": " ".join(f"-p {name}" for name in owners),
                    "comment-only": "true",
                    "msrv": "true" if msrv else "false",
                    "scene-assets": scene_assets_for(owners),
                    "apple-legs": apple_legs_for(" ".join(owners)),
                }
            )
        else:
            affected = report["affected"]
            outputs.update(
                {
                    "packages": " ".join(affected),
                    "package-args": " ".join(f"-p {name}" for name in affected),
                    "msrv": "true" if msrv else "false",
                    "scene-assets": scene_assets_for(affected),
                    "apple-legs": apple_legs_for(" ".join(affected)),
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

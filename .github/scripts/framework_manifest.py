"""Build the certified framework manifest (`framework.json`) for a release.

The manifest records the worktree's submodule pins, its lockfile hashes, the
scaffold table the root manifest's `[package.metadata.waterui]` declares, and
that metadata table verbatim. The Rust side derives exactly the same scaffold
table for a tree (`framework_scaffold` in
cli/src/project_model/framework.rs) — the two must not drift, so this script
never reads anything under cli/.

`stable` certifies the framework release tag release-plz published
(`v<version>`), or — under release preflight, where the release does not
exist yet — the candidate revision it will publish. The named revision is
checked out so every recorded fact is read from the released tree.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tomllib


# The submodules whose checkout carries a native backend repository; each
# basename keys the `{name}-backend-url`/`{name}-backend-revision` scaffold
# entries, matching BACKEND_SUBMODULES on the Rust side.
BACKEND_SUBMODULES = ("backends/apple", "backends/android")


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def recorded_submodules():
    """path -> commit for every submodule at its recorded revision."""
    submodules = {}
    for line in subprocess.check_output(
        ["git", "submodule", "status", "--recursive"], text=True
    ).splitlines():
        if not line.startswith(" "):
            raise RuntimeError(f"Submodule is not at its recorded revision: {line}")
        commit, path, *_ = line.split()
        submodules[path] = commit
    return submodules


def lockfile_hashes(submodules):
    lockfiles = {}
    for path in [Path("Cargo.lock"), *(Path(path) / "Cargo.lock" for path in submodules)]:
        if path.is_file():
            lockfiles[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
    return lockfiles


def framework_scaffold(framework):
    """The scaffold table the framework manifest itself declares: each
    `scaffold-packages` entry's `[workspace.dependencies]` requirement and
    each backend's `{name}-backend-url` from `[package.metadata.waterui]`.
    Identical to `framework_scaffold` in the CLI for the same tree."""
    metadata = framework["package"]["metadata"]["waterui"]
    workspace = framework["workspace"]["dependencies"]
    scaffold = {}
    for name in metadata["scaffold-packages"]:
        dependency = workspace[name]
        scaffold[f"{name}-version"] = (
            dependency if isinstance(dependency, str) else dependency["version"]
        )
    for path in BACKEND_SUBMODULES:
        name = path.rsplit("/", 1)[-1]
        scaffold[f"{name}-backend-url"] = metadata[f"{name}-backend-url"]
    return scaffold


def build_manifest(channel, tag, revision, repository):
    """Write `framework.json` for the checked-out revision."""
    framework = tomllib.loads(Path("Cargo.toml").read_text())
    metadata = framework["package"]["metadata"]["waterui"]
    submodules = recorded_submodules()
    manifest = {
        "schema_version": 2,
        "channel": channel,
        "repository": repository,
        "revision": revision,
        "tag": tag,
        "run_id": int(os.environ["GITHUB_RUN_ID"]),
        "run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"]),
        "submodules": submodules,
        "lockfiles": lockfile_hashes(submodules),
        "scaffold": framework_scaffold(framework),
        # The framework-owned metadata table verbatim — including
        # `minimum-cli-version`: a new fact added there flows into every
        # manifest without a schema change.
        "metadata": metadata,
    }
    Path("framework.json").write_text(json.dumps(manifest, indent=2) + "\n")


def checkout(revision):
    """Put the worktree at `revision` so every recorded fact names the tree
    being certified rather than whatever the job happened to check out."""
    git("checkout", revision)
    git("submodule", "update", "--init", "--recursive")


def prepare_stable(tag, revision):
    repository = os.environ["GITHUB_REPOSITORY"]
    if revision is None:
        # release-plz pushed the tag during this run, after the job checked
        # out — fetch it before resolving the commit it names.
        git("fetch", "origin", f"refs/tags/{tag}:refs/tags/{tag}")
        revision = git("rev-parse", f"{tag}^{{commit}}")
    checkout(revision)
    build_manifest("stable", tag, revision, repository)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="channel", required=True)
    stable = commands.add_parser(
        "stable", help="certify a published framework release"
    )
    stable.add_argument(
        "--tag", required=True, help="the v<version> tag the release carries"
    )
    stable.add_argument(
        "--revision",
        help="the commit the tag certifies (defaults to resolving the tag; "
        "release preflight passes the candidate commit before the tag exists)",
    )
    args = parser.parse_args()
    prepare_stable(args.tag, args.revision)


if __name__ == "__main__":
    main()
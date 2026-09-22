"""Build the certified framework manifest (`framework.json`) for a release.

The manifest records the worktree's lockfile hash, the scaffold table the root manifest's `[package.metadata.waterui]` declares, and
that metadata table verbatim. A scaffold package whose
`[workspace.dependencies]` requirement is a git pin (`git` + `rev`) is not one
the stable channel distributes, whatever the registry holds for that name, so
a `stable` manifest withholds its entries from `scaffold` and records the pin
under `experimental-packages` instead; `nightly` carries it in `scaffold` like any
other package. The Rust side derives exactly the same tables for a tree
(`framework_scaffold` in
https://github.com/water-rs/cli/blob/dev/src/project_model/framework.rs) — the
two must not drift, so this script never reads anything from the CLI.

`stable` certifies the framework release tag release-plz published
(`v<version>`), or — under release preflight, where the release does not
exist yet — the candidate revision it will publish. The named revision is
checked out so every recorded fact is read from the released tree.

`nightly` certifies the `dev` revision a green full-matrix run tested, as
the immutable `nightly-<date>-<sha12>` tag `nightly.yml` publishes. It
refuses a revision that is already certified or that does not advance the
newest certification, so the certified line only moves forward.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

NIGHTLY_TAG = re.compile(r"nightly-\d{8}-[0-9a-f]{12}")


# No backend rides a gitlink. The scaffold table copies each backend's
# declared release version or exact commit from `[package.metadata.waterui]`.


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def lockfile_hashes():
    """The dependency lock the CLI verifies a certified tree against."""
    lock = Path("Cargo.lock")
    return {str(lock): hashlib.sha256(lock.read_bytes()).hexdigest()}


def framework_scaffold(framework):
    """The scaffold table the framework manifest itself declares: each
    `scaffold-packages` entry's `[workspace.dependencies]` requirement —
    `{name}-version`, plus `{name}-git` and `{name}-rev` when the requirement
    pins a repository — and every backend coordinate — `{name}-backend-url`,
    plus the `{name}-backend-version` of a backend pinned by release or the
    `{name}-backend-revision` of one pinned by commit — from
    `[package.metadata.waterui]`.
    Identical to `framework_scaffold` in the CLI for the same tree."""
    metadata = framework["package"]["metadata"]["waterui"]
    workspace = framework["workspace"]["dependencies"]
    scaffold = {}
    for name in metadata["scaffold-packages"]:
        dependency = workspace[name]
        scaffold[f"{name}-version"] = (
            dependency if isinstance(dependency, str) else dependency["version"]
        )
        # A scaffold package pinned from git keeps that source: a bare
        # `{name}-version` cannot express the commit the framework builds
        # against, and the registry may not carry it at all.
        if isinstance(dependency, dict) and "git" in dependency:
            revision = dependency.get("rev")
            if not isinstance(revision, str) or not re.fullmatch(
                r"[0-9a-fA-F]{40}", revision
            ):
                raise RuntimeError(
                    f"workspace.dependencies.{name} must pin an immutable Git revision"
                )
            scaffold[f"{name}-git"] = dependency["git"]
            scaffold[f"{name}-rev"] = revision
    for key, value in metadata.items():
        if key.endswith(("-backend-url", "-backend-version", "-backend-revision")):
            scaffold[key] = value
    return scaffold


def channel_scaffold(framework, channel):
    """The `(scaffold, experimental)` tables `channel`'s manifest records.

    A scaffold package pinned to a git revision is not one the stable channel
    distributes, since it distributes only registry requirements, so `stable`
    withholds its `{name}-*` entries from the scaffold table and records the pin — name, git URL,
    revision and declared version — under `experimental-packages`; `dev` and
    `nightly` distribute it through `scaffold` as always. The split derives
    from the dependency's shape alone: `framework_scaffold` already marks a
    git pin with a `{name}-git` entry, so no second list is maintained. The
    CLI splits the derived table the same way and holds the certification's
    `experimental-packages` to agreement.
    """
    scaffold = framework_scaffold(framework)
    experimental = {}
    if channel == "stable":
        for key in sorted(scaffold):
            if key.endswith("-git"):
                name = key[: -len("-git")]
                experimental[name] = {
                    "version": scaffold.pop(f"{name}-version"),
                    "git": scaffold.pop(f"{name}-git"),
                    "rev": scaffold.pop(f"{name}-rev"),
                }
    return scaffold, experimental


def build_manifest(channel, tag, revision, repository):
    """Write `framework.json` for the checked-out revision."""
    framework = tomllib.loads(Path("Cargo.toml").read_text())
    metadata = framework["package"]["metadata"]["waterui"]
    scaffold, experimental = channel_scaffold(framework, channel)
    manifest = {
        "schema_version": 2,
        "channel": channel,
        "repository": repository,
        "revision": revision,
        "tag": tag,
        "run_id": int(os.environ["GITHUB_RUN_ID"]),
        "run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"]),
        "lockfiles": lockfile_hashes(),
        "scaffold": scaffold,
        # The packages the channel withholds: stable's git-pinned scaffold
        # packages, empty on every channel that distributes them.
        "experimental-packages": experimental,
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


def prepare_stable(tag, revision):
    repository = os.environ["GITHUB_REPOSITORY"]
    if revision is None:
        # release-plz pushed the tag during this run, after the job checked
        # out — fetch it before resolving the commit it names.
        git("fetch", "origin", f"refs/tags/{tag}:refs/tags/{tag}")
        revision = git("rev-parse", f"{tag}^{{commit}}")
    checkout(revision)
    build_manifest("stable", tag, revision, repository)


def published_nightlies(repository):
    """Every published (non-draft) nightly prerelease, newest last."""
    pages = json.loads(subprocess.check_output([
        "gh", "api", f"repos/{repository}/releases", "--paginate", "--slurp",
    ], text=True))
    releases = [
        release for page in pages for release in page
        if not release["draft"] and release["prerelease"]
        and NIGHTLY_TAG.fullmatch(release["tag_name"])
    ]
    return sorted(releases, key=lambda release: release["published_at"])


def write_outputs(outputs):
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        for key, value in outputs.items():
            output.write(f"{key}={value}\n")
    if outputs["eligible"] == "false":
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary:
            summary.write(outputs["reason"] + "\n")


def prepare_nightly():
    repository = os.environ["GITHUB_REPOSITORY"]
    revision = git("rev-parse", "HEAD")
    if revision != os.environ["GITHUB_SHA"]:
        raise RuntimeError("The checkout is not the revision tested by this run")
    date = git("show", "-s", "--format=%cs", "HEAD").replace("-", "")
    tag = f"nightly-{date}-{revision[:12]}"
    releases = published_nightlies(repository)
    if any(release["tag_name"] == tag for release in releases):
        write_outputs({"eligible": "false", "reason": "This revision is already certified"})
        return
    if releases:
        latest = git("rev-parse", f'{releases[-1]["tag_name"]}^{{commit}}')
        comparison = subprocess.run(["git", "merge-base", "--is-ancestor", latest, revision])
        if comparison.returncode == 1:
            write_outputs({"eligible": "false", "reason": "This run does not advance the certified revision"})
            return
        comparison.check_returncode()
    build_manifest("nightly", tag, revision, repository)
    write_outputs({"eligible": "true", "tag": tag, "revision": revision})


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
    commands.add_parser(
        "nightly", help="certify the dev revision a green full-matrix run tested"
    )
    args = parser.parse_args()
    if args.channel == "nightly":
        prepare_nightly()
    else:
        prepare_stable(args.tag, args.revision)


if __name__ == "__main__":
    main()

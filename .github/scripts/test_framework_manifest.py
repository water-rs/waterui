"""Tests for `framework_manifest.py`'s scaffold derivation.

They run against the repository's own root manifest, so the split the
release workflows record is the one asserted here: `framework_scaffold` in
the CLI (`water-rs/cli`, `src/project_model/framework.rs`) derives the same
tables for the same tree — the certification contract holds them to
agreement.
"""

from pathlib import Path
import tomllib

import framework_manifest

ROOT = Path(__file__).resolve().parents[2]
FRAMEWORK = tomllib.loads((ROOT / "Cargo.toml").read_text())
WORKSPACE = FRAMEWORK["workspace"]["dependencies"]
SCAFFOLD_PACKAGES = FRAMEWORK["package"]["metadata"]["waterui"]["scaffold-packages"]


def git_pinned(name):
    """Whether `scaffold-packages`'s `name` requirement pins a git revision —
    the shape that makes a package experimental."""
    dependency = WORKSPACE[name]
    return isinstance(dependency, dict) and "git" in dependency


def test_every_git_pinned_scaffold_package_pins_an_immutable_revision():
    for name in SCAFFOLD_PACKAGES:
        if git_pinned(name):
            revision = WORKSPACE[name]["rev"]
            assert len(revision) == 40 and all(
                character in "0123456789abcdefABCDEF" for character in revision
            ), f"workspace.dependencies.{name} must pin a full commit hash"


def test_stable_withholds_git_pinned_packages_from_the_scaffold_table():
    scaffold, experimental = framework_manifest.channel_scaffold(FRAMEWORK, "stable")

    # `waterui-winui` is unpublished — the package this split exists for.
    assert "waterui-winui" in SCAFFOLD_PACKAGES
    assert git_pinned("waterui-winui")

    for name in SCAFFOLD_PACKAGES:
        if not git_pinned(name):
            assert scaffold[f"{name}-version"] == (
                WORKSPACE[name]
                if isinstance(WORKSPACE[name], str)
                else WORKSPACE[name]["version"]
            )
            continue
        assert not any(key.startswith(f"{name}-") for key in scaffold), (
            f"stable must not scaffold git-pinned {name}"
        )
        assert experimental[name] == {
            "version": WORKSPACE[name]["version"],
            "git": WORKSPACE[name]["git"],
            "rev": WORKSPACE[name]["rev"],
        }

    # Backend coordinates are not scaffold packages: they stay in the table.
    assert "apple-backend-url" in scaffold
    assert "android-backend-revision" in scaffold


def test_nightly_carries_git_pinned_packages_in_the_scaffold_table():
    scaffold, experimental = framework_manifest.channel_scaffold(FRAMEWORK, "nightly")

    for name in SCAFFOLD_PACKAGES:
        assert f"{name}-version" in scaffold
        if git_pinned(name):
            assert scaffold[f"{name}-git"] == WORKSPACE[name]["git"]
            assert scaffold[f"{name}-rev"] == WORKSPACE[name]["rev"]
    assert experimental == {}

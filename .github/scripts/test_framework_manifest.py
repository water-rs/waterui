"""Tests for `framework_manifest.py`'s scaffold derivation.

They run against the repository's own root manifest, so the split the
release workflows record is the one asserted here: `framework_scaffold` in
the CLI (`cli/src/project_model/framework.rs` in this tree) derives the
same tables for the same tree — the certification contract holds them to
agreement.
"""

import copy
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


def requirement_version(dependency):
    return dependency if isinstance(dependency, str) else dependency["version"]


def with_git_pin(framework, name):
    """`framework` with scaffold package `name` pinned to a git revision — the
    shape a not-yet-published package has between releases."""
    pinned = copy.deepcopy(framework)
    pinned["workspace"]["dependencies"][name] = {
        "version": requirement_version(WORKSPACE[name]),
        "git": f"https://github.com/water-rs/{name}",
        "rev": "0123456789abcdef0123456789abcdef01234567",
    }
    return pinned


def test_stable_scaffolds_every_released_package_by_version():
    scaffold, experimental = framework_manifest.channel_scaffold(FRAMEWORK, "stable")

    for name in SCAFFOLD_PACKAGES:
        if git_pinned(name):
            continue
        assert scaffold[f"{name}-version"] == requirement_version(WORKSPACE[name])
        assert name not in experimental

    # Backend coordinates are not scaffold packages: they stay in the table.
    assert "apple-backend-path" in scaffold
    assert "android-backend-revision" in scaffold
    # Host coordinates are not scaffold packages either. The Hydrolysis
    # Android host lives in this repository (#1428): no external pin exists,
    # and the subdirectory names the Gradle composite root inside the
    # certified framework checkout.
    assert "hydrolysis-android-host-subdirectory" in scaffold
    assert "hydrolysis-android-host-url" not in scaffold
    assert "hydrolysis-android-host-revision" not in scaffold


def test_hydrolysis_android_host_path_is_scaffolded_on_every_channel():
    """`--platform android --backend hydrolysis` reads the host coordinates
    through the resolved framework on all three channels, so the in-tree
    path must survive the scaffold split regardless of which packages the
    channel withholds. The host is part of this repository (#1428): the
    revision the manifest certifies is the revision the host is checked out
    at, so no external URL or second pin may reappear."""
    for channel in ("dev", "nightly", "stable"):
        scaffold, _ = framework_manifest.channel_scaffold(FRAMEWORK, channel)
        assert scaffold["hydrolysis-android-host-subdirectory"] == (
            "backends/hydrolysis/android"
        ), f"{channel} must carry the in-tree hydrolysis-android-host-subdirectory"
        assert "hydrolysis-android-host-url" not in scaffold
        assert "hydrolysis-android-host-revision" not in scaffold


def test_stable_withholds_git_pinned_packages_from_the_scaffold_table():
    # Whichever package is between releases, the split derives from the
    # dependency's shape alone, so a synthetic pin exercises it on every tree.
    name = SCAFFOLD_PACKAGES[-1]
    pinned = with_git_pin(FRAMEWORK, name)
    scaffold, experimental = framework_manifest.channel_scaffold(pinned, "stable")

    assert not any(key.startswith(f"{name}-") for key in scaffold), (
        f"stable must not scaffold git-pinned {name}"
    )
    pinned_dependencies = pinned["workspace"]["dependencies"]
    assert experimental == {
        package: {
            "version": pinned_dependencies[package]["version"],
            "git": pinned_dependencies[package]["git"],
            "rev": pinned_dependencies[package]["rev"],
        }
        for package in SCAFFOLD_PACKAGES
        if package == name or git_pinned(package)
    }
    for other in SCAFFOLD_PACKAGES:
        if other != name and not git_pinned(other):
            assert scaffold[f"{other}-version"] == requirement_version(WORKSPACE[other])


def test_nightly_carries_git_pinned_packages_in_the_scaffold_table():
    name = SCAFFOLD_PACKAGES[-1]
    pinned = with_git_pin(FRAMEWORK, name)
    dependency = pinned["workspace"]["dependencies"][name]

    scaffold, experimental = framework_manifest.channel_scaffold(pinned, "nightly")

    for other in SCAFFOLD_PACKAGES:
        assert f"{other}-version" in scaffold
    assert scaffold[f"{name}-git"] == dependency["git"]
    assert scaffold[f"{name}-rev"] == dependency["rev"]
    assert experimental == {}


def test_nami_pinned_backends_are_git_pinned_scaffold_packages():
    """A scaffolded graph carries this workspace's nami pin, which the
    published `waterui-dew`/`waterui-gtk`/`waterui-winui` cannot satisfy —
    their releases still require the nami line that kept `Signal::get`
    (nami#26). Until each backend releases a migrated version, its
    `[workspace.dependencies]` requirement pins the nami-migration head, so
    `dev` and `nightly` scaffold that commit and `stable` withholds the
    package under `experimental-packages`. Drop this test with the pins."""
    for name in ("waterui-dew", "waterui-gtk", "waterui-winui"):
        assert git_pinned(name), (
            f"[workspace.dependencies].{name} must pin its nami-migration head"
        )
        dependency = WORKSPACE[name]
        rev = dependency.get("rev", "")
        assert len(rev) == 40 and all(c in "0123456789abcdef" for c in rev), (
            f"[workspace.dependencies].{name} must pin an immutable revision"
        )
        assert dependency["git"].startswith("https://github.com/water-rs/")

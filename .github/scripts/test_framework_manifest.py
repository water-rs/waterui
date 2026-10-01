"""Tests for `framework_manifest.py`'s scaffold derivation.

They run against the repository's own root manifest, so the split the
release workflows record is the one asserted here: `framework_scaffold` in
the CLI (`water-rs/cli`, `src/project_model/framework.rs`) derives the same
tables for the same tree — the certification contract holds them to
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
    assert "apple-backend-url" in scaffold
    assert "android-backend-revision" in scaffold
    # Host coordinates are not scaffold packages either.
    assert "hydrolysis-android-host-url" in scaffold
    assert "hydrolysis-android-host-revision" in scaffold
    assert "hydrolysis-android-host-subdirectory" in scaffold


def test_hydrolysis_android_host_pin_is_scaffolded_on_every_channel():
    """`--platform android --backend hydrolysis` reads the host coordinates
    through the resolved framework on all three channels, so the pin must
    survive the scaffold split regardless of which packages the channel
    withholds."""
    for channel in ("dev", "nightly", "stable"):
        scaffold, _ = framework_manifest.channel_scaffold(FRAMEWORK, channel)
        assert scaffold["hydrolysis-android-host-url"].startswith(
            "https://github.com/water-rs/"
        ), f"{channel} must carry hydrolysis-android-host-url"
        revision = scaffold["hydrolysis-android-host-revision"]
        assert len(revision) == 40 and all(
            character in "0123456789abcdef" for character in revision
        ), f"{channel} must carry an immutable hydrolysis-android-host-revision"
        assert scaffold["hydrolysis-android-host-subdirectory"] == "android"


def test_stable_withholds_git_pinned_packages_from_the_scaffold_table():
    # Whichever package is between releases, the split derives from the
    # dependency's shape alone, so a synthetic pin exercises it on every tree.
    name = SCAFFOLD_PACKAGES[-1]
    pinned = with_git_pin(FRAMEWORK, name)
    dependency = pinned["workspace"]["dependencies"][name]

    scaffold, experimental = framework_manifest.channel_scaffold(pinned, "stable")

    assert not any(key.startswith(f"{name}-") for key in scaffold), (
        f"stable must not scaffold git-pinned {name}"
    )
    assert experimental == {
        name: {
            "version": dependency["version"],
            "git": dependency["git"],
            "rev": dependency["rev"],
        }
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


def test_nami_pinned_backends_resolve_through_patch_pins():
    """A scaffolded graph carries this workspace's nami pin, which the
    published `waterui-dew`/`waterui-gtk`/`waterui-winui` cannot satisfy —
    their releases still require the nami line that kept `Signal::get`
    (nami#26). Until each backend releases a migrated version, its
    `[patch.crates-io]` pin is what keeps a generated project resolving;
    without it `water fetch` fails resolution before scaffolding finishes.
    Drop this test with the pins."""
    patches = FRAMEWORK["patch"]["crates-io"]
    for name in ("waterui-dew", "waterui-gtk", "waterui-winui"):
        dependency = patches.get(name)
        assert isinstance(dependency, dict), (
            f"[patch.crates-io] must pin {name} to its nami-migration head"
        )
        rev = dependency.get("rev", "")
        assert len(rev) == 40 and all(c in "0123456789abcdef" for c in rev), (
            f"[patch.crates-io].{name} must pin an immutable revision"
        )
        assert dependency["git"].startswith("https://github.com/water-rs/")

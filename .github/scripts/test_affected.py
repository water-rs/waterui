"""Unit tests for the parts of affected.py that need no git object:

the prose classification (which must agree with ci.yml's `code` filter)
and the MSRV-relevance extraction, which compares parsed manifest content
so layout and comment edits do not re-arm the MSRV leg.

The tool-level behaviours — root-package swallowing, the in-package
markdown carve-out and the `nextest-fixtures` check of the `test-fonts`
setup-script rule against the package graph — run against the real
repository, skipped when the built determinator binary is absent.
"""

import json
import os
from pathlib import Path
import subprocess
import tempfile

import pytest

from affected import is_prose, manifest_tables


def test_top_level_markdown_is_prose():
    assert is_prose("README.md")
    assert is_prose("CHANGELOG.md")


def test_in_package_markdown_is_not_prose():
    # `include_str!` / `#![doc = include_str!("../README.md")]` compile
    # markdown inside a package into the crate — it must keep flowing to
    # package ownership.
    assert not is_prose("backends/hydrolysis/README.md")
    assert not is_prose("components/devtools/mcp/protocol/src/instructions.md")
    assert not is_prose("src/README.md")


def test_docs_directory_is_prose():
    assert is_prose("docs/layout-spec.md")
    assert is_prose("docs/sub/dir/page.txt")


def test_licenses_and_issue_templates_are_prose():
    assert is_prose("LICENSE-MIT")
    assert is_prose(".github/ISSUE_TEMPLATE/bug.md")


def test_code_and_ci_files_are_not_prose():
    assert not is_prose("core/src/lib.rs")
    assert not is_prose(".github/workflows/ci.yml")
    assert not is_prose(".github/tools/affected/rules.toml")
    assert not is_prose("tests/layout-twins/README.md")


def test_dependency_sections_and_rust_version_are_extracted():
    tables = manifest_tables(
        """
        [package]
        name = "demo"
        rust-version = "1.88"

        [dependencies]
        serde = "1"
        """
    )
    assert tables[("package", "rust-version")] == "1.88"
    assert tables[("dependencies", "serde")] == "1"


def test_target_dependency_tables_are_extracted():
    # `[target.<cfg>.dependencies]` sections move the graph too — MSRV must
    # re-arm for them the same way it does for plain `[dependencies]`.
    tables = manifest_tables(
        """
        [target.'cfg(unix)'.dependencies]
        ncurses = "5"

        [target.'cfg(target_os = "android")'.dev-dependencies]
        probe = "0.1"
        """
    )
    assert tables[("target", "cfg(unix)", "dependencies", "ncurses")] == "5"
    assert (
        tables[("target", "cfg(target_os = \"android\")", "dev-dependencies", "probe")]
        == "0.1"
    )


def test_manifest_layout_and_comments_do_not_change_the_result():
    base = manifest_tables('[dependencies]\nserde = "1"\n')
    commented = manifest_tables('# a comment\n[dependencies]\n\nserde = "1" # pinned\n')
    assert base == commented


def test_an_unparseable_manifest_is_not_a_table_set():
    assert manifest_tables("not = [valid") is None


# ---------------------------------------------------------------------------
# Integration tests: real diffs through the built determinator tool.
# They synthesize commits on top of HEAD via git plumbing objects — the
# worktree is never touched — so each case is a single-path diff.
# ---------------------------------------------------------------------------

REPO = Path(__file__).resolve().parents[2]
TOOL = REPO / ".github/tools/affected/target/release/affected"


def _commit_with(edits):
    """Commit `edits` on top of HEAD and return the sha, without touching
    the worktree: a temp index, read-tree, update-index, write-tree,
    commit-tree."""
    env = dict(os.environ)
    with tempfile.NamedTemporaryFile(prefix="affected-idx-", delete=False) as handle:
        env["GIT_INDEX_FILE"] = handle.name
    subprocess.check_call(["git", "read-tree", "HEAD"], env=env, cwd=REPO)
    for path, content in edits.items():
        blob = (
            subprocess.check_output(
                ["git", "hash-object", "-w", "--stdin"],
                input=content.encode(),
                cwd=REPO,
            )
            .strip()
            .decode()
        )
        subprocess.check_call(
            ["git", "update-index", "--add", "--cacheinfo", f"100644,{blob},{path}"],
            env=env,
            cwd=REPO,
        )
    tree = (
        subprocess.check_output(["git", "write-tree"], env=env, cwd=REPO)
        .strip()
        .decode()
    )
    sha = (
        subprocess.check_output(
            ["git", "commit-tree", tree, "-p", "HEAD", "-m", "affected test"],
            cwd=REPO,
        )
        .strip()
        .decode()
    )
    os.unlink(env["GIT_INDEX_FILE"])
    return sha


def _report(edits):
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO)
    sha = _commit_with(edits)
    return json.loads(
        subprocess.check_output(
            [str(TOOL), "--base", head.strip().decode(), "--head", sha],
            text=True,
            cwd=REPO,
        )
    )


pytestmark_tool = pytest.mark.skipif(
    not TOOL.exists(), reason="the determinator tool is not built"
)


@pytestmark_tool
def test_clippy_toml_selects_the_workspace():
    report = _report({"Clippy.toml": "[lints]\n"})
    assert report["workspace"] is True


@pytestmark_tool
def test_lowercase_lint_config_selects_the_workspace():
    # The determinator's shipped defaults mark `clippy.toml`/`rustfmt.toml`
    # as changing nothing — rules.toml overrides them to the workspace.
    report = _report({"clippy.toml": "[lints]\n"})
    assert report["workspace"] is True


@pytestmark_tool
def test_an_unknown_top_level_file_selects_the_workspace():
    # It ancestor-matches only the root `waterui` package, whose directory
    # is the repository root — a non-source top-level file is reported as
    # unmatched instead of silently narrowing to waterui plus revdeps.
    report = _report({"some-random-config.yaml": "k: v\n"})
    assert report["workspace"] is True


@pytestmark_tool
def test_layout_twins_results_select_the_workspace():
    report = _report(
        {"tests/layout-twins/results/waterui/x-s1/probe.json": "{}\n"}
    )
    assert report["workspace"] is True


@pytestmark_tool
def test_in_package_markdown_selects_its_package():
    report = _report(
        {"components/devtools/mcp/protocol/src/instructions.md": "# hi\n"}
    )
    assert report["workspace"] is False
    assert report["affected"] == ["waterui-mcp-protocol"]


@pytestmark_tool
def test_top_level_markdown_selects_nothing():
    report = _report({"README.md": "# hi\n"})
    assert report["workspace"] is False
    assert report["affected"] == []


@pytestmark_tool
def test_root_package_sources_select_waterui():
    report = _report({"facade.rs": "// comment\n"})
    assert report["workspace"] is False
    assert "waterui" in report["affected"]


def _fixture_check(edits):
    """Run `affected nextest-fixtures` on a detached worktree of HEAD with
    `edits` applied; returns the completed process."""
    sha = _commit_with(edits)
    with tempfile.TemporaryDirectory(prefix="affected-fixtures-") as tmp:
        root = Path(tmp) / "tree"
        subprocess.check_call(
            ["git", "worktree", "add", "--detach", "--quiet", str(root), sha],
            cwd=REPO,
        )
        try:
            # The edits only add or drop an edge between workspace
            # members, which re-resolves the lockfile without the network.
            return subprocess.run(
                [str(TOOL), "nextest-fixtures", "--root", str(root)],
                capture_output=True,
                text=True,
                cwd=REPO,
                env={**os.environ, "CARGO_NET_OFFLINE": "true"},
            )
        finally:
            subprocess.check_call(
                ["git", "worktree", "remove", "--force", str(root)], cwd=REPO
            )


@pytestmark_tool
def test_test_fonts_rule_matches_the_graph():
    result = _fixture_check({})
    assert result.returncode == 0, result.stderr


@pytestmark_tool
def test_dropping_a_testing_edge_flags_the_rule():
    manifest = "components/foundation/controls/Cargo.toml"
    text = (REPO / manifest).read_text()
    edge = 'waterui-testing = { path = "../../../testing" }\n'
    assert edge in text
    result = _fixture_check({manifest: text.replace(edge, "")})
    assert result.returncode == 1
    assert (
        'extra (their tests do not link waterui-testing): ["waterui-controls"]'
        in result.stderr
    )


@pytestmark_tool
def test_gaining_a_testing_edge_flags_the_rule():
    manifest = "utils/meta/Cargo.toml"
    text = (REPO / manifest).read_text()
    assert "[dev-dependencies]" not in text
    edge = '\n[dev-dependencies]\nwaterui-testing = { path = "../../testing" }\n'
    result = _fixture_check({manifest: text + edge})
    assert result.returncode == 1
    assert (
        'missing (their tests link waterui-testing): ["waterui-meta"]'
        in result.stderr
    )


# The `macos` matrix: a leg exists only for a target with an Apple-gated
# crate in scope, so no macOS runner starts to find nothing to lint.
from apple_gated import IOS_SIM, crates_for, legs


def leg_targets(scope):
    return [leg["target"] for leg in legs(scope)]


def test_no_apple_gated_crate_starts_no_leg():
    assert legs("") == []
    assert legs("waterui-core waterui-layout") == []


def test_workspace_starts_every_leg():
    assert leg_targets("workspace") == ["", "", "", IOS_SIM]
    host = [leg for leg in legs("workspace") if leg["target"] == ""]
    # One pass per host leg: run back to back they overran the budget.
    assert [(leg["hydrolysis"], leg["waterui-cli"], leg["apple-gated"]) for leg in host] == [
        (True, False, False),
        (False, True, False),
        (False, False, True),
    ]


def test_host_only_crates_start_only_host_legs():
    assert legs("waterui-cli") == [
        {
            "name": "aarch64-apple-darwin (waterui-cli)",
            "target": "",
            "hydrolysis": False,
            "waterui-cli": True,
            "apple-gated": False,
        }
    ]
    assert leg_targets("waterui-testing cherenkov-bench") == [""]


def test_simulator_compatible_crates_start_both_legs():
    assert leg_targets("cherenkov-oracle") == ["", IOS_SIM]
    assert not any(leg["hydrolysis"] for leg in legs("cherenkov-oracle"))


def test_simulator_splits_library_only_crates():
    assert crates_for(IOS_SIM, "waterui cherenkov") == (["cherenkov"], ["waterui"])
    assert crates_for("", "waterui cherenkov") == (["waterui", "cherenkov"], [])


def test_an_unknown_target_is_an_error():
    with pytest.raises(ValueError):
        crates_for("x86_64-apple-ios", "workspace")

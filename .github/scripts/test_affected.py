"""Unit tests for the parts of affected.py that need no git object:

the prose classification (which must agree with ci.yml's `code` filter)
and the MSRV-relevance extraction, which compares parsed manifest content
so layout and comment edits do not re-arm the MSRV leg.
"""

from affected import is_prose, manifest_tables


def test_markdown_is_prose_at_any_depth():
    assert is_prose("README.md")
    assert is_prose("backends/hydrolysis/README.md")


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


def test_manifest_layout_and_comments_do_not_change_the_result():
    base = manifest_tables('[dependencies]\nserde = "1"\n')
    commented = manifest_tables('# a comment\n[dependencies]\n\nserde = "1" # pinned\n')
    assert base == commented


def test_an_unparseable_manifest_is_not_a_table_set():
    assert manifest_tables("not = [valid") is None

"""Tests for `layout_gate.py`'s semantic comparison and path classification.

The syntax-tree cases exercise `semantic_differs` on Rust sources directly;
the path cases exercise `gate` against a scratch git repository, since the
label decision for a file status (added, modified, under `src/tests/`) is a
property of the diff, not of either source alone.
"""

import subprocess

import pytest

import layout_gate

LAYOUT_SRC = "components/foundation/layout/src/"


def differs(base, head):
    return layout_gate.semantic_differs(base, head)


# --- Normalised attributes: lint and hint paths never need the label. ---


def test_adding_must_use_does_not_need_the_label():
    base = "pub fn content(&self) -> View {\n    self.child\n}\n"
    head = "#[must_use]\npub fn content(&self) -> View {\n    self.child\n}\n"
    assert not differs(base, head)


def test_adding_a_multi_line_expect_does_not_need_the_label():
    base = "fn weight(&self) -> f64 {\n    self.value\n}\n"
    head = """#[expect(
    clippy::cast_precision_loss,
    reason = "the span fits in f64 exactly",
)]
fn weight(&self) -> f64 {
    self.value
}
"""
    assert not differs(base, head)


def test_allow_warn_deny_inline_and_doc_do_not_need_the_label():
    base = "fn f() {}\n"
    for attribute in (
        "#[allow(dead_code)]",
        "#[warn(clippy::all)]",
        "#[deny(unsafe_code)]",
        "#[inline(always)]",
        '#[doc = "hidden"]',
    ):
        assert not differs(base, f"{attribute}\n{base}"), attribute


def test_an_inner_lint_attribute_does_not_need_the_label():
    assert not differs("pub mod m;\n", "#![allow(dead_code)]\npub mod m;\n")


# --- Comments and doc comments never need the label. ---


def test_a_doc_comment_edit_does_not_need_the_label():
    base = "/// The gap between stacked children.\nfn f() {}\n"
    head = "/// Space distributed between stacked children.\nfn f() {}\n"
    assert not differs(base, head)


def test_a_comment_edit_does_not_need_the_label():
    base = "// place children along the major axis\nfn f() {}\n"
    head = "/* children run along the major axis */\nfn f() {}\n"
    assert not differs(base, head)


# --- Test code never needs the label. ---


def test_an_edit_inside_cfg_test_code_does_not_need_the_label():
    base = """fn f() {}
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(1 + 1, 2);
    }
}
"""
    head = base.replace("assert_eq!(1 + 1, 2)", "assert_eq!(2 + 2, 4)")
    assert not differs(base, head)


def test_cfg_test_is_only_matched_literally():
    """`#[cfg(all(test, unix))]` stays: it is a semantic `cfg` the way any
    other is, not the plain test-code marker."""
    base = "fn f() {}\n"
    head = '#[cfg(all(test, unix))]\nfn g() {}\n' + base
    assert differs(base, head)


# --- Everything else in a frozen file needs the label. ---


def test_a_changed_literal_needs_the_label():
    base = "fn gap() -> f64 {\n    8.0\n}\n"
    head = "fn gap() -> f64 {\n    16.0\n}\n"
    assert differs(base, head)


def test_an_added_derive_needs_the_label():
    base = "pub struct Gap;\n"
    head = "#[derive(Debug)]\npub struct Gap;\n"
    assert differs(base, head)


def test_an_added_cfg_needs_the_label():
    base = "fn f() {}\n"
    head = "#[cfg(unix)]\nfn f() {}\n"
    assert differs(base, head)


def test_a_renamed_binding_needs_the_label():
    base = "fn f(axis: i32) -> i32 {\n    axis\n}\n"
    head = "fn f(direction: i32) -> i32 {\n    direction\n}\n"
    assert differs(base, head)


# --- Path classification and the diff-level gate. ---


def test_path_classification():
    assert layout_gate.classification("docs/layout-spec.md") == "spec"
    assert layout_gate.classification("core/src/ui/layout.rs") == "frozen"
    assert layout_gate.classification(f"{LAYOUT_SRC}stack/vstack.rs") == "frozen"
    assert layout_gate.classification(f"{LAYOUT_SRC}tests/mod.rs") is None
    assert layout_gate.classification(f"{LAYOUT_SRC}tests/contract.rs") is None
    # Non-Rust files under the frozen root are not in the frozen set.
    assert layout_gate.classification(f"{LAYOUT_SRC}notes.md") is None
    assert layout_gate.classification("ffi/src/layout.rs") is None


def git(repo, *args):
    subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True
    )


@pytest.fixture
def repo(tmp_path, monkeypatch):
    git(tmp_path, "init", "-q", "--initial-branch=dev")
    git(tmp_path, "config", "user.email", "gate@example.test")
    git(tmp_path, "config", "user.name", "Layout Gate")
    monkeypatch.chdir(tmp_path)
    return tmp_path


def commit(repo, *files):
    for path, content in files:
        file = repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content)
    git(repo, "add", "-A")
    git(repo, "commit", "-qm", "change")
    return subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def test_an_edit_in_a_tests_file_does_not_need_the_label(repo):
    base = commit(repo, (f"{LAYOUT_SRC}tests/mod.rs", "mod contract;\n"))
    head = commit(repo, (f"{LAYOUT_SRC}tests/mod.rs", "mod contract;\nmod more;\n"))
    assert layout_gate.gate(base, head) == []


def test_a_new_frozen_file_needs_the_label(repo):
    base = commit(repo, (f"{LAYOUT_SRC}lib.rs", "pub mod stack;\n"))
    head = commit(
        repo,
        (f"{LAYOUT_SRC}lib.rs", "pub mod stack;\npub mod axis;\n"),
        (f"{LAYOUT_SRC}axis.rs", "pub struct Axis;\n"),
    )
    assert layout_gate.gate(base, head) == [
        (f"{LAYOUT_SRC}axis.rs", "a frozen layout file was added"),
        (f"{LAYOUT_SRC}lib.rs", "the file's layout semantics changed"),
    ]


def test_a_spec_edit_needs_the_label(repo):
    base = commit(repo, ("docs/layout-spec.md", "# Layout\n"))
    head = commit(repo, ("docs/layout-spec.md", "# Layout\n\n## Gaps\n"))
    assert layout_gate.gate(base, head) == [
        ("docs/layout-spec.md", "the layout specification changed")
    ]


def test_an_unrelated_edit_does_not_need_the_label(repo):
    base = commit(repo, ("core/src/ui/text.rs", "fn f() {}\n"))
    head = commit(repo, ("core/src/ui/text.rs", "fn f() { g(); }\n"))
    assert layout_gate.gate(base, head) == []

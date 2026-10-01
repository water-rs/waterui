# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "tree-sitter==0.26.0",
#     "tree-sitter-rust==0.24.2",
# ]
# ///
"""Decide whether a pull request needs the `layout-decision` label.

The layout system is frozen (AGENTS.md, "Layout Is Frozen"), and
`.github/workflows/layout-decision.yml` runs this gate to block a pull
request until the maintainer records his decision by applying the label —
but only when the pull request can actually change layout semantics. The
label is required when, between the pull request's base and head:

- `docs/layout-spec.md`, the normative description of the layout system,
  changed at all;
- a frozen non-test Rust file was added or deleted — every `*.rs` under
  `components/foundation/layout/src/` except `src/tests/`, plus
  `core/src/ui/layout.rs`;
- such a file exists on both sides and its syntax tree differs after
  normalisation.

Normalisation removes what cannot change semantics: comments and doc
comments; attributes whose path is one of `must_use`, `expect`, `allow`,
`warn`, `deny`, `inline`, `doc` (inner or outer, single- or multi-line);
and items carrying `#[cfg(test)]`, such as `mod tests`. Every other
attribute (`cfg`, `derive`, `repr`, …) stays, because it can change
semantics. The comparison parses both revisions with tree-sitter and
compares the normalised node trees — node kinds plus leaf text — never
line diffs or regexes over source.

The script prints each frozen path that needs the label with the reason,
and exits 1 when any path does and 0 otherwise:

    layout_gate.py <base> <head>
"""

import argparse
import re
import subprocess
import sys

import tree_sitter
import tree_sitter_rust

SPEC_PATH = "docs/layout-spec.md"
LAYOUT_SOURCE = "components/foundation/layout/src/"
LAYOUT_TESTS = "components/foundation/layout/src/tests/"
CORE_LAYOUT = "core/src/ui/layout.rs"

# Attributes that record lints or hints: removing them from both trees is
# safe because the compiler ignores them for semantics. Every other
# attribute stays — `cfg` selects code, `derive` and `repr` generate it.
IGNORED_ATTRIBUTE_PATHS = frozenset(
    {"must_use", "expect", "allow", "warn", "deny", "inline", "doc"}
)

COMMENT_NODE_TYPES = frozenset({"line_comment", "block_comment"})
ATTRIBUTE_NODE_TYPES = frozenset({"attribute_item", "inner_attribute_item"})

LANGUAGE = tree_sitter.Language(tree_sitter_rust.language())


def git(*args, text=True):
    return subprocess.check_output(["git", *args], text=text)


def classification(path):
    """What `path` is to the gate: "spec", "frozen", or None."""
    if path == SPEC_PATH:
        return "spec"
    if path == CORE_LAYOUT:
        return "frozen"
    if (
        path.startswith(LAYOUT_SOURCE)
        and path.endswith(".rs")
        and not path.startswith(LAYOUT_TESTS)
    ):
        return "frozen"
    return None


def attribute_parts(attribute_item):
    """The `(path, arguments)` of an `attribute_item` or
    `inner_attribute_item`, each the node's text."""
    attribute = next(
        child
        for child in attribute_item.named_children
        if child.type == "attribute"
    )
    path = attribute.named_children[0].text.decode()
    arguments = (
        attribute.named_children[1].text.decode()
        if len(attribute.named_children) > 1
        else ""
    )
    return path, arguments


def is_ignored_attribute(attribute_item):
    """Whether the attribute carries a lint or hint path only."""
    path, _ = attribute_parts(attribute_item)
    return path in IGNORED_ATTRIBUTE_PATHS


def is_cfg_test_attribute(attribute_item):
    """Whether the attribute is exactly `#[cfg(test)]` (or `#![cfg(test)]`).
    `cfg(all(test, …))` is not matched: it is test code only on some
    configurations, and the freeze reads it like any other `cfg`."""
    path, arguments = attribute_parts(attribute_item)
    return path == "cfg" and re.sub(r"\s+", "", arguments) == "(test)"


def normalized_children(node):
    """The normalised children of `node`.

    tree-sitter attaches attributes to an item as siblings, not children,
    so the walk carries the attribute run (`pending`) it has seen since the
    last non-attribute node. An item preceded by `#[cfg(test)]` is dropped
    together with every attribute in that run — they all decorate test
    code. Comments never reach the output."""
    children = []
    pending = []
    for child in node.children:
        if child.type in COMMENT_NODE_TYPES:
            continue
        if child.type in ATTRIBUTE_NODE_TYPES:
            pending.append(child)
            continue
        if not any(is_cfg_test_attribute(a) for a in pending):
            children.extend(
                normalized(attribute)
                for attribute in pending
                if not is_ignored_attribute(attribute)
            )
            children.append(normalized(child))
        pending = []
    if pending and not any(is_cfg_test_attribute(a) for a in pending):
        children.extend(
            normalized(attribute)
            for attribute in pending
            if not is_ignored_attribute(attribute)
        )
    return children


def normalized(node):
    """The canonical form of `node`'s subtree: a `(kind, text)` pair for a
    leaf, `(kind, children)` otherwise. Two files are semantically equal
    when their roots' canonical forms are equal."""
    children = normalized_children(node)
    if children:
        return (node.type, tuple(children))
    return (node.type, node.text.decode())


def semantic_differs(base_source, head_source):
    """Whether the normalised syntax trees of the two sources differ."""
    base = tree_sitter.Parser(LANGUAGE).parse(base_source.encode())
    head = tree_sitter.Parser(LANGUAGE).parse(head_source.encode())
    return normalized(base.root_node) != normalized(head.root_node)


def changed_entries(base, head):
    """`git diff --name-status -M` between `base` and `head`, as a list of
    `(status, path)` or `(status, old_path, new_path)` for renames."""
    tokens = git(
        "diff", "--name-status", "-M", "-z", base, head, text=False
    ).split(b"\0")
    entries = []
    index = 0
    while index < len(tokens):
        status = tokens[index]
        index += 1
        if not status:
            continue
        if status[:1] in (b"R", b"C"):
            entries.append(
                (status.decode(), tokens[index].decode(), tokens[index + 1].decode())
            )
            index += 2
        else:
            entries.append((status.decode(), tokens[index].decode()))
            index += 1
    return entries


def source_at(revision, path):
    return git("show", f"{revision}:{path}", text=False).decode()


def gate(base, head):
    """Every `(path, reason)` that requires the `layout-decision` label."""
    base = git("merge-base", base, head).strip()
    findings = []
    for entry in changed_entries(base, head):
        status = entry[0]
        if status.startswith(("R", "C")):
            _, old_path, new_path = entry
            for path, side in ((old_path, "deleted"), (new_path, "added")):
                if classification(path) == "frozen":
                    findings.append((path, f"a frozen layout file was {side}"))
                elif classification(path) == "spec":
                    findings.append((path, "the layout specification changed"))
            continue
        _, path = entry
        kind = classification(path)
        if kind == "spec":
            findings.append((path, "the layout specification changed"))
        elif kind == "frozen":
            if status == "A":
                findings.append((path, "a frozen layout file was added"))
            elif status == "D":
                findings.append((path, "a frozen layout file was deleted"))
            elif semantic_differs(source_at(base, path), source_at(head, path)):
                findings.append(
                    (path, "the file's layout semantics changed")
                )
    return findings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", help="base revision (its merge-base with head is used)")
    parser.add_argument("head", help="head revision")
    args = parser.parse_args()
    findings = gate(args.base, args.head)
    for path, reason in findings:
        print(f"{path}: {reason}")
    sys.exit(1 if findings else 0)


if __name__ == "__main__":
    main()

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
line diffs or regexes over source. The shared machinery lives in
`rust_semantic_diff.py`, which the affected-only CI gate uses for its
comment-only comparison as well.

The script prints each frozen path that needs the label with the reason,
and exits 1 when any path does and 0 otherwise:

    layout_gate.py <base> <head>
"""

import argparse
import sys

from rust_semantic_diff import (
    changed_entries,
    git,
    semantic_differs,
    source_at,
)

SPEC_PATH = "docs/layout-spec.md"
LAYOUT_SOURCE = "components/foundation/layout/src/"
LAYOUT_TESTS = "components/foundation/layout/src/tests/"
CORE_LAYOUT = "core/src/ui/layout.rs"


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

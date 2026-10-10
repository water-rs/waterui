"""The workspace crates whose own sources carry Apple `cfg`s (#2476).

The Linux lint compiles `cfg(target_os = "macos")` and
`cfg(target_os = "ios")` code away, so test.yml's `macos` matrix lints
these crates on Apple targets through the `macos-lint` composite.
affected.py emits `legs` as the matrix, so a macOS runner starts only for
a target with work in the affected scope; the composite reads the cargo
`-p` arguments for its target from the same lists.

Usage:

    apple_gated.py --legs <scope>               # the matrix entries, as JSON
    apple_gated.py <target> <scope> [group]     # the cargo `-p` arguments

`<scope>` is affected.py's `packages`: `workspace`, or space-separated
package names. `<target>` is empty for the macOS host. `<group>` selects
one of the iOS simulator's `SIM_GROUPS`; it is meaningless for the host,
whose single leg lints the whole list.
"""

import json
import sys

IOS_SIM = "aarch64-apple-ios-sim"

# waterui-apple, cocoa-ui and cherenkov-gpu are not listed: apple.yml's
# rust job lints them on both targets. waterui-cli and hydrolysis's
# checked feature set have their own composite steps. The whole group
# lints `--all-targets` on the macOS host and on `aarch64-apple-ios-sim`
# alike; the sysinfo/libc defect that once forced library-only passes on
# the simulator is pinned away (Cargo.lock keeps libc 0.2.189, #2485).
CRATES = (
    "hydrolysis", "waterui", "waterui-internal",
    "cherenkov", "cherenkov-record", "cherenkov-cpu", "cherenkov-oracle",
    "cherenkov-scene", "cherenkov-shader", "cherenkov-bench",
    "filtrate", "filtrate-core", "filtrate-derive",
    "waterui-controls", "waterui-text", "waterui-graphics", "waterui-media",
    "waterui-locale", "waterui-preview", "waterui-preview-protocol",
    "waterui-testing", "waterui-ts", "waterui-ts-engine-jsc",
    "waterui-macros", "waterui-assets-macros", "waterui-url",
)

# The simulator's share of CRATES, split into parallel legs (#2521): the
# single leg that ran all of them cold overran the macos job's 10-minute
# budget. Every leg recompiles its dependency closure from scratch, so
# the groups are balanced by closure weight, not crate count — dev-dep
# tails dominate: hydrolysis's tests and examples pull criterion, m3,
# chart, mcp and icons-lucide; waterui and waterui-testing share the
# testing <-> hydrolysis cycle; the cherenkov family pulls wgpu/naga and
# the generated scene fonts; waterui-media's tests pull the video-gpu
# stack and the software codec; the devtools leg is the light one.
# The union of the groups is exactly CRATES: coverage is unchanged, and
# each group still lints `--all-targets`.
SIM_GROUPS = (
    ("backend", ("hydrolysis",)),
    ("facade", ("waterui", "waterui-internal", "waterui-testing")),
    (
        "engine",
        (
            "cherenkov", "cherenkov-record", "cherenkov-cpu",
            "cherenkov-oracle", "cherenkov-scene", "cherenkov-shader",
            "cherenkov-bench",
        ),
    ),
    (
        "components",
        (
            "filtrate", "filtrate-core", "filtrate-derive",
            "waterui-controls", "waterui-text", "waterui-graphics",
            "waterui-media", "waterui-locale",
        ),
    ),
    (
        "devtools",
        (
            "waterui-preview", "waterui-preview-protocol",
            "waterui-ts", "waterui-ts-engine-jsc",
            "waterui-macros", "waterui-assets-macros", "waterui-url",
        ),
    ),
)


def in_scope(crates, scope):
    if scope == "workspace":
        return list(crates)
    names = set(scope.split())
    return [crate for crate in crates if crate in names]


def crates_for(target, scope, group=""):
    """The crates `target` lints with `--all-targets`.

    `group` names one of `SIM_GROUPS` and is honoured only on the iOS
    simulator: its legs partition `CRATES`, while the macOS host's single
    leg lints the whole list.
    """
    if target == "":
        if group:
            raise ValueError(f"the macOS host leg has no group {group!r}")
        return in_scope(CRATES, scope)
    if target == IOS_SIM:
        if not group:
            return in_scope(CRATES, scope)
        for name, crates in SIM_GROUPS:
            if name == group:
                return in_scope(crates, scope)
        raise ValueError(f"unknown iOS simulator lint group {group!r}")
    raise ValueError(f"unknown Apple lint target {target!r}")


def leg(name, target, hydrolysis=False, cli=False, gated=False, group=""):
    return {
        "name": name,
        "target": target,
        "hydrolysis": hydrolysis,
        "waterui-cli": cli,
        "apple-gated": gated,
        "group": group,
    }


def legs(scope):
    """The `macos` matrix entries with work for `scope`; empty when none.

    Each pass — and, on the simulator, each crate group — is its own leg:
    every leg compiles its dependency closure cold, and passes run back
    to back overran the 10-minute budget (#2506 on the host, #2521 on the
    simulator).
    """
    hydrolysis = bool(in_scope(("hydrolysis",), scope))
    cli = bool(in_scope(("waterui-cli",), scope))
    entries = []
    if hydrolysis:
        entries.append(leg("aarch64-apple-darwin (hydrolysis)", "", hydrolysis=True))
    if cli:
        entries.append(leg("aarch64-apple-darwin (waterui-cli)", "", cli=True))
    if any(crates_for("", scope)):
        entries.append(leg("aarch64-apple-darwin", "", gated=True))
    if hydrolysis:
        entries.append(leg(f"{IOS_SIM} (hydrolysis)", IOS_SIM, hydrolysis=True))
    for name, _ in SIM_GROUPS:
        if any(crates_for(IOS_SIM, scope, name)):
            entries.append(leg(f"{IOS_SIM} ({name})", IOS_SIM, gated=True, group=name))
    return entries


def main(argv):
    if len(argv) == 2 and argv[0] == "--legs":
        print(json.dumps(legs(argv[1]), separators=(",", ":")))
    elif len(argv) in (2, 3):
        crates = crates_for(*argv)
        print(" ".join(f"-p {crate}" for crate in crates))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])

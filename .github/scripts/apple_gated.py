"""The workspace crates whose own sources carry Apple `cfg`s (#2476).

The Linux lint compiles `cfg(target_os = "macos")` and
`cfg(target_os = "ios")` code away, so test.yml's `macos` matrix lints
these crates on Apple targets through the `macos-lint` composite.
affected.py emits `legs` as the matrix, so a macOS runner starts only for
a target with work in the affected scope; the composite reads the cargo
`-p` arguments for its target from the same lists.

Usage:

    apple_gated.py --legs <scope>        # the matrix entries, as JSON
    apple_gated.py <target> <scope>      # `--all-targets` args, then
                                         # library-only args, one line each

`<scope>` is affected.py's `packages`: `workspace`, or space-separated
package names. `<target>` is empty for the macOS host.
"""

import json
import sys

IOS_SIM = "aarch64-apple-ios-sim"

# waterui-apple, cocoa-ui and cherenkov-gpu are not listed: apple.yml's
# rust job lints them on both targets. waterui-cli and hydrolysis's
# checked feature set have their own composite steps.
MACOS = (
    "hydrolysis", "waterui", "waterui-internal",
    "cherenkov", "cherenkov-record", "cherenkov-cpu", "cherenkov-oracle",
    "cherenkov-scene", "cherenkov-shader", "cherenkov-bench",
    "filtrate", "filtrate-core", "filtrate-derive",
    "waterui-controls", "waterui-text", "waterui-graphics", "waterui-media",
    "waterui-locale", "waterui-preview", "waterui-preview-protocol",
    "waterui-testing", "waterui-ts", "waterui-ts-engine-jsc",
    "waterui-macros", "waterui-assets-macros", "waterui-url",
)

IOS_SIM_ALL_TARGETS = (
    "waterui-internal",
    "cherenkov", "cherenkov-record", "cherenkov-cpu", "cherenkov-oracle",
    "cherenkov-scene", "cherenkov-shader",
    "filtrate", "filtrate-core", "filtrate-derive",
    "waterui-locale", "waterui-preview", "waterui-preview-protocol",
    "waterui-ts", "waterui-ts-engine-jsc",
    "waterui-assets-macros", "waterui-url",
)

# Library targets only on the simulator, because of a dependency defect:
# sysinfo 0.39.6 calls `libc::mach_host_self` and `libc::mach_task_self`
# on every Apple target, but libc 0.2.190 declares both for
# `target_os = "macos"` only (0.2.189 declared them for all Apple
# targets). These crates' test targets reach sysinfo through
# waterui-testing and do not compile for aarch64-apple-ios-sim.
# waterui-testing and cherenkov-bench depend on sysinfo outright and are
# linted on the host only.
IOS_SIM_LIB = (
    "hydrolysis", "waterui",
    "waterui-controls", "waterui-text", "waterui-graphics", "waterui-media",
    "waterui-macros",
)


def in_scope(crates, scope):
    if scope == "workspace":
        return list(crates)
    names = set(scope.split())
    return [crate for crate in crates if crate in names]


def crates_for(target, scope):
    """The `--all-targets` crates and the library-only crates for `target`."""
    if target == "":
        return in_scope(MACOS, scope), []
    if target == IOS_SIM:
        return in_scope(IOS_SIM_ALL_TARGETS, scope), in_scope(IOS_SIM_LIB, scope)
    raise ValueError(f"unknown Apple lint target {target!r}")


def leg(name, target, hydrolysis=False, cli=False, gated=False):
    return {
        "name": name,
        "target": target,
        "hydrolysis": hydrolysis,
        "waterui-cli": cli,
        "apple-gated": gated,
    }


def legs(scope):
    """The `macos` matrix entries with work for `scope`; empty when none.

    On the macOS host each pass is its own leg: every leg compiles cold,
    and the three passes run back to back overran the 10-minute budget
    (#2506). The simulator leg's passes share one graph and stay together.
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
    if any(crates_for(IOS_SIM, scope)):
        entries.append(leg(IOS_SIM, IOS_SIM, hydrolysis=hydrolysis, gated=True))
    return entries


def main(argv):
    if len(argv) == 2 and argv[0] == "--legs":
        print(json.dumps(legs(argv[1]), separators=(",", ":")))
    elif len(argv) == 2:
        for crates in crates_for(argv[0], argv[1]):
            print(" ".join(f"-p {crate}" for crate in crates))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])

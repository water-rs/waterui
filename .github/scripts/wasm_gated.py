"""The wasm32 lint legs: one list shared by the nightly and the PR gate.

The Linux lint compiles `cfg(target_arch = "wasm32")` code away, so
test.yml's `wasm` job lints the crates that carry wasm32-only code for
`wasm32-unknown-unknown`. The nightly runs every leg; the pull-request and
dev gates run only the legs whose package is in the affected scope, so a
change to wasm32-only code cannot pass the gate and fail the next nightly
(#3837).

Usage:

    wasm_gated.py --any <scope>   # prints `true` or `false`
    wasm_gated.py <scope>         # runs the legs in scope, as the job does

`<scope>` is affected.py's `packages`: `workspace`, or space-separated
package names (the affected crates and their reverse dependencies).
"""

import subprocess
import sys

# (package, cargo arguments). Order is the order the legs run in. Each leg's
# reason lives next to it; the job's `RUSTFLAGS=-D warnings` holds the
# workspace crates in the dependency graph to the same bar as the `-D
# warnings` after `--`, which only covers the package named with `-p`.
LEGS = (
    # waterkit-dialog is a registry crate, so cargo caps its lints; only the
    # workspace crate is named.
    ("waterui-media", ("-p", "waterui-media")),
    # `web` is Hydrolysis's browser runner; its web video bridge needs
    # `video` (#2799) and `webview-system` is the `<iframe>` bridge, which
    # only this leg can lint (#1795).
    (
        "hydrolysis",
        ("-p", "hydrolysis", "--features", "web,video,webview-system"),
    ),
    # The bare build selects a third of the backend's `cfg(feature)` surface
    # that `web` never compiles.
    ("hydrolysis", ("-p", "hydrolysis", "--no-default-features")),
    # filtrate's `webgl` feature is a deliberate `compile_error` off wasm32,
    # so the all-features pass is the only way the WebGL executor is linted
    # anywhere (#1449).
    ("filtrate", ("-p", "filtrate", "--all-features")),
    # The browser test target compiles the WebGPU surface code nothing on
    # the host can see.
    ("cherenkov-gpu", ("-p", "cherenkov-gpu", "--lib", "--test", "browser")),
    # The whole CPU backend's wasm shape.
    ("cherenkov-cpu", ("-p", "cherenkov-cpu", "--all-targets")),
    # The engine crate's wasm test targets.
    ("cherenkov", ("-p", "cherenkov", "--all-targets")),
)


def legs_in_scope(scope):
    if scope == "workspace":
        return list(LEGS)
    names = set(scope.split())
    return [leg for leg in LEGS if leg[0] in names]


def command(args):
    return (
        "cargo", "clippy", "--locked", "--profile", "ci-check",
        "--target", "wasm32-unknown-unknown", *args, "--", "-D", "warnings",
    )


def main(argv):
    if len(argv) == 2 and argv[0] == "--any":
        print("true" if legs_in_scope(argv[1]) else "false")
    elif len(argv) == 1:
        failed = []
        for _, args in legs_in_scope(argv[0]):
            cmd = command(args)
            print(f"::group::{' '.join(cmd)}", flush=True)
            status = subprocess.run(cmd).returncode
            print("::endgroup::", flush=True)
            if status != 0:
                failed.append(" ".join(cmd))
        for cmd in failed:
            print(f"::error::failed: {cmd}")
        if failed:
            raise SystemExit(1)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])

#!/usr/bin/env python3
"""bench/android/run.py — drive the frozen fixture suite.

The harness lives in hydrolysis so archiving android-backend cannot remove
the acceptance machinery. It consumes suite.toml (the frozen inventory) and
the pinned toolchain, launches each fixture's built APK, waits for a settled
non-blank frame, verifies against the frozen golden where one exists, and
emits one result record per (fixture, script, round) in the shared schema.

Device plumbing (settle detection, ANR dismissal, golden comparison,
gfxinfo/meminfo capture, perfetto) is the imported android-backend
implementation in scripts/e2e.py — imported verbatim and hash-pinned by
toolchain-lock.json so the old backend's measurement semantics are the
baseline, not a re-derivation.

APK production is parameterized: android-backend fixtures come from
`water package --backend android`; the hydrolysis host's packaging lands
with the plan's step 7 — until then `--apk-dir` maps fixture names to
locally built APKs.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(BENCH_DIR))
sys.path.insert(0, str(BENCH_DIR / "scripts"))

import metrics  # noqa: E402
from metrics import frames as frame_metrics  # noqa: E402
from metrics import memory as memory_metrics  # noqa: E402
from metrics import input_latency as latency_metrics  # noqa: E402

# The imported frozen implementation; see toolchain-lock.json for its hash.
import e2e  # noqa: E402


def fixture_env(fixture: dict, defaults: dict) -> dict:
    """The intent extras CI applies: shared defaults plus per-fixture env."""
    env = dict(defaults.get("env", {}))
    env.update(fixture.get("env", {}))
    return env


def launch_and_capture(
    serial: str,
    name: str,
    fixture: dict,
    apk: Path,
    defaults: dict,
    artifacts: Path,
) -> dict:
    """One launch: install, start, settle, verify or smoke-check.

    Returns the round's result `metrics` payload; artifacts land under
    `artifacts/<name>/`.
    """
    out = artifacts / name
    out.mkdir(parents=True, exist_ok=True)
    package = fixture["package"]
    mode = fixture["mode"]
    settle_s = fixture.get(
        "settle_timeout_seconds", defaults["settle_timeout_seconds"]
    )
    poll_s = fixture.get("poll_interval_ms", defaults["poll_interval_ms"]) / 1000

    e2e.install_apk(serial, str(apk))
    e2e.force_stop(serial, package)
    launch_ms = e2e.device_now_ms(serial)
    e2e.launch_app(serial, package, fixture_env(fixture, defaults))
    watch = e2e.ProcessWatch(serial, package)
    result: dict = {"apk_bytes": apk.stat().st_size}

    anr: list[str] = []
    foreign: list[str] = []
    settled, frame, first_content_ms = e2e.wait_for_settle(
        serial,
        settle_s,
        poll_s,
        watch=watch,
        package=package,
        anr=anr,
        foreign=foreign,
    )
    if anr:
        result["anr"] = anr
    if foreign:
        result["foreign_focus"] = foreign[-1]
    if first_content_ms is not None:
        result["first_content_ms"] = int(first_content_ms - launch_ms)

    if frame is not None:
        (out / "capture.png").write_bytes(frame)
        result["nonblank"] = e2e.is_nonblank(frame)

    golden_name = fixture.get("golden", "none")
    if mode == "verify" and golden_name != "none":
        golden_path = BENCH_DIR / golden_name
        if frame is None or not golden_path.exists():
            result["verify"] = "unavailable"
        else:
            from PIL import Image  # noqa: E402  — pinned harness dep

            diff = e2e.compare_images(
                Image.open(golden_path),
                Image.open(out / "capture.png"),
                pixel_tolerance=defaults["pixel_tolerance"],
            )
            result["golden_diff_fraction"] = diff
            result["verify"] = (
                "pass"
                if settled
                and diff <= defaults["max_diff_fraction"]
                else "fail"
            )
    else:
        # smoke, or verify without a frozen baseline (anchored_overlay):
        # settled non-blank content is the check.
        result["verify"] = "pass" if settled and result.get("nonblank") else "fail"

    # Round metrics: frames over the launch window + a memory snapshot. The
    # expensive dumps stay a separate replay per section 6 — here they run in
    # the launch round because launch IS the measured script; interaction
    # replays are the metrics/ callers' job.
    result.update(frame_metrics.collect(serial, package, out, "frames"))
    result.update(
        memory_metrics.collect_at(
            serial, package, "settled_idle", out, "memory"
        )
    )
    result.update(latency_metrics.collect(serial, out, "input"))
    e2e.dump_logcat(serial, out / "logcat.txt")
    e2e.force_stop(serial, package)
    return result


def cmd_run(args: argparse.Namespace) -> int:
    suite = metrics.load_suite(Path(args.suite))
    serial = args.serial or metrics.detect_serial()
    env_id = metrics.environment_identity(serial)
    lock = metrics.load_lock()
    revisions = {
        "harness": {
            "repository": "water-rs/hydrolysis",
            "revision": args.revision,
        },
        "suite_sources": {
            k: v["revision"] for k, v in lock["sources"].items()
        },
    }
    apk_dir = Path(args.apk_dir) if args.apk_dir else None
    selected = args.fixture or sorted(suite["fixtures"].keys())
    failures = []
    for name in selected:
        fixture = suite["fixtures"].get(name)
        if fixture is None:
            sys.exit(f"unknown fixture {name!r}; not in suite.toml")
        if fixture["mode"] == "skip" and not args.run_skipped:
            print(f"== {name}: skip — {fixture['reason']}")
            continue
        apk = apk_dir / f"{name}.apk" if apk_dir else None
        if apk is None or not apk.exists():
            print(f"== {name}: no APK under {apk_dir} — not run")
            continue
        print(f"== {name}: {fixture['mode']} on {serial} ({apk.name})")
        record = metrics.make_result(
            backend=args.backend,
            painter=args.painter,
            fixture=name,
            script="launch",
            round_index=args.round,
            revisions=revisions,
            environment=env_id,
            metrics=launch_and_capture(
                serial,
                name,
                fixture,
                apk,
                suite["defaults"],
                Path(args.out),
            ),
        )
        path = metrics.write_result(
            record, Path(args.out) / "results", f"{name}.round{args.round}"
        )
        status = record["metrics"].get("verify", "fail")
        print(f"   -> {status}  ({path})")
        if status != "pass":
            failures.append(name)
    if failures:
        print("FAILED fixtures:", ", ".join(failures))
        return 1
    return 0


def cmd_check_inventory(args: argparse.Namespace) -> int:
    """Landing check: the inventory matches android-backend's CI exactly.

    Every runnable example at the pinned waterui revision has an entry;
    frozen policies equal the imported manifest; skip entries are only the
    two documented ones; registered twins match the imported Twins.kt.
    """
    suite = metrics.load_suite(Path(args.suite))
    waterui = Path(args.waterui)
    manifest = json.loads(
        (BENCH_DIR / "reference" / "manifest.json").read_text()
    )
    twins_src = (
        BENCH_DIR
        / "reference"
        / "app"
        / "src"
        / "main"
        / "java"
        / "dev"
        / "waterui"
        / "android"
        / "reference"
        / "Twins.kt"
    ).read_text()

    errors: list[str] = []
    examples = sorted(
        p.name
        for p in (waterui / "examples").iterdir()
        if (p / "src" / "lib.rs").exists()
    )
    fixtures = suite["fixtures"]
    for name in examples:
        if name not in fixtures:
            errors.append(f"example {name!r} missing from suite.toml")
    for name in fixtures:
        if name not in examples:
            errors.append(f"suite.toml entry {name!r} has no fixture source")
        if name in manifest["examples"]:
            wanted = manifest["examples"][name]["mode"]
            actual = fixtures[name]["mode"]
            if actual != wanted:
                errors.append(
                    f"{name}: mode {actual!r} != manifest {wanted!r}"
                )
    extra_skips = {
        n for n, f in fixtures.items() if f["mode"] == "skip"
    } - {"chromium", "webview-cef"}
    if extra_skips:
        errors.append(f"new skip entries are not parity: {extra_skips}")
    registered = {
        n for n, f in fixtures.items() if f.get("compose_twin") == "registered"
    }
    twins_in_source = set()
    for line in twins_src.splitlines():
        line = line.strip()
        if line.startswith('"') and 'Twin()' in line:
            twins_in_source.add(line.split('"')[1])
    if registered != twins_in_source:
        errors.append(
            f"twin set {sorted(registered)} != Twins.kt {sorted(twins_in_source)}"
        )
    unfinished = suite.get("unfinished", {}).get("compose_twins", {})
    listed = set(
        unfinished.get("verify", [])
    ) | set(unfinished.get("smoke", [])) | set(unfinished.get("skip", []))
    missing = {
        n for n, f in fixtures.items() if f.get("compose_twin") == "missing"
    }
    if listed != missing:
        errors.append(
            "unfinished twin list diverges from fixtures marked missing: "
            f"listed={sorted(listed)} missing={sorted(missing)}"
        )
    for error in errors:
        print("INVENTORY ERROR:", error)
    if not errors:
        print(
            f"inventory OK: {len(fixtures)} fixtures match CI "
            f"({len(examples)} examples at the pinned waterui revision)"
        )
    return 1 if errors else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="run fixtures against built APKs")
    run.add_argument("--suite", default=str(BENCH_DIR / "suite.toml"))
    run.add_argument("--serial", default=None)
    run.add_argument("--apk-dir", default=None,
                     help="directory holding one <fixture>.apk per entry")
    run.add_argument("--fixture", action="append", default=None)
    run.add_argument("--run-skipped", action="store_true",
                     help="also attempt the two documented skip entries")
    run.add_argument("--backend", default="hydrolysis")
    run.add_argument("--painter", default="cherenkov")
    run.add_argument("--revision", default="unknown",
                     help="source revision of the backend under test")
    run.add_argument("--round", type=int, default=0)
    run.add_argument("--out", default=str(BENCH_DIR / "out"))
    run.set_defaults(func=cmd_run)

    check = sub.add_parser(
        "check-inventory",
        help="assert suite.toml matches android-backend's CI inventory",
    )
    check.add_argument("--suite", default=str(BENCH_DIR / "suite.toml"))
    check.add_argument("--waterui", required=True,
                       help="waterui checkout at the locked revision")
    check.set_defaults(func=cmd_check_inventory)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())

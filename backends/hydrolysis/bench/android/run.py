#!/usr/bin/env python3
"""bench/android/run.py — drive the frozen fixture suite.

The harness lives in hydrolysis so archiving android-backend cannot remove
the acceptance machinery. It consumes suite.toml (the frozen inventory) and
the pinned toolchain, launches each fixture's built APK, waits for a settled
non-blank frame, verifies against the golden where one exists, and emits one
result record per (fixture, script, round) in the shared schema.

The goldens are no longer stored (the repository carries no binary assets):
a missing `reference/goldens/<fixture>.png` is generated during the run by
rendering the fixture's registered Compose twin in the reference app and
capturing its settled frame (water-rs/hydrolysis#343), which needs
`--reference-apk`. A fixture whose twin is still unregistered — the
`[unfinished.compose_twins]` list — reports `verify: unavailable` and names
the gap.

Device plumbing (settle detection, ANR dismissal, golden comparison,
gfxinfo/meminfo capture, perfetto) is the imported android-backend
implementation in scripts/e2e.py — imported verbatim and hash-pinned by
toolchain-lock.json so the old backend's measurement semantics are the
baseline, not a re-derivation.

APK production is parameterized: android-backend fixtures come from
`water package --backend android`; the hydrolysis host's packaging lands
with the plan's step 7 — until then `--apk-dir` maps fixture names to
locally built APKs.

The Gradle trees (`reference/` here, `android/` for the host) are the
callers of `scripts/fetch-gradle-wrapper.py`: their `gradlew` scripts
need `gradle/wrapper/gradle-wrapper.jar`, which the repository does not
store. Run `uv run scripts/fetch-gradle-wrapper.py` once before any
`./gradlew` invocation — it materializes the jar for every wrapper in
the tree, hash-verified against the published checksums.
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
        if not golden_path.exists():
            # water-rs/hydrolysis#343: the golden is the reference app's
            # settled twin frame, generated at run time rather than stored.
            foreign_twin: list[str] = []
            twin = e2e.capture_twin(serial, name, settle_s, poll_s, foreign_twin)
            if twin is None:
                detail = (
                    f"foreign window '{foreign_twin[-1]}' held focus"
                    if foreign_twin
                    else "no registered twin — see [unfinished.compose_twins]"
                )
                result["golden_generation"] = detail
            else:
                golden_path.parent.mkdir(parents=True, exist_ok=True)
                golden_path.write_bytes(twin)
                result["golden_generation"] = "generated"
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
    if args.reference_apk:
        e2e.adb(serial, "install", "-r", args.reference_apk)
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


def flavor_for_fixture(name: str) -> str:
    """suite.toml fixture name -> gradle flavor (e.g. typography-rtl ->
    typography_rtl). Suite-hosted screens map to the suite variant."""
    if name in ("suite", "editing", "list-stress"):
        return "suite"
    return name.replace("-", "_")


def package_for_fixture(name: str) -> str:
    base = "dev.waterui.android.reference"
    if name in ("suite", "editing", "list-stress"):
        return base
    return f"{base}.{name.replace('-', '_')}"


def cmd_lock_campaign(args: argparse.Namespace) -> int:
    """Resolve the current stable Compose BOM and freeze it for the campaign.

    Section 6: resolve the current stable BOM at campaign start, record it
    and all resolved dependencies, then keep it immutable. `build` appends
    the resolved dependency graph to the same lock after the first build.
    """
    import urllib.request
    import xml.etree.ElementTree as ET

    url = (
        "https://dl.google.com/dl/android/maven2/"
        "androidx/compose/compose-bom/maven-metadata.xml"
    )
    with urllib.request.urlopen(url, timeout=30) as resp:
        meta = ET.fromstring(resp.read())
    versions = [v.text for v in meta.iter("version") if v.text]
    stable = [
        v
        for v in versions
        if not any(tag in v for tag in ("alpha", "beta", "rc", "-dev"))
    ]
    if not stable:
        sys.exit("no stable Compose BOM found in maven-metadata")
    bom = stable[-1]
    print(f"latest stable compose-bom: {bom}")

    lock_path = BENCH_DIR / "campaign-lock.json"
    lock = {}
    if lock_path.exists():
        lock = json.loads(lock_path.read_text())
        old = lock.get("compose_bom")
        if old and old != bom and not args.refresh:
            print(
                f"campaign already locked to {bom} differs from existing "
                f"{old}; pass --refresh to re-lock (recalibrate noise!)"
            )
    lock.update(
        {
            "schema": "bench/android/campaign-lock@1",
            "compose_bom": bom,
            "resolved_at_utc": time.strftime(
                "%Y-%m-%dT%H:%M:%SZ", time.gmtime()
            ),
        }
    )
    lock_path.write_text(json.dumps(lock, indent=2, sort_keys=True) + "\n")

    props_path = BENCH_DIR / "reference" / "gradle.properties"
    props = props_path.read_text()
    if "composeBomVersion" in props:
        props = "\n".join(
            f"composeBomVersion={bom}"
            if line.startswith("composeBomVersion=")
            else line
            for line in props.splitlines()
        ) + "\n"
    else:
        props += f"composeBomVersion={bom}\n"
    props_path.write_text(props)
    print(f"froze composeBomVersion={bom} -> {props_path}")
    return 0


def _gradle(*tasks: str, project: Path) -> None:
    env = dict(os.environ)
    env.setdefault("JAVA_HOME", "/usr/lib/jvm/java-21-openjdk-amd64")
    subprocess.run(
        [str(project / "gradlew"), *tasks],
        cwd=project,
        env=env,
        check=True,
    )


def _apk_out(project: Path, module: str) -> list[Path]:
    root = project / module / "build" / "outputs" / "apk"
    return sorted(root.glob("**/*.apk")) if root.is_dir() else []


def cmd_build(args: argparse.Namespace) -> int:
    """Build the release APK set for a campaign.

    Per fixture flavor: :app:assemble<Flavor>Release (non-debuggable,
    R8/resource-shrunk, arm64-v8a, debug-keystore-signed — identical signing
    and delivery across candidates). Copies each arm64 APK to
    --out/<fixture>.apk and records byte sizes in an artifacts manifest so
    the zero-tolerance package-bytes gate has measured inputs.
    """
    project = BENCH_DIR / "reference"
    suite = metrics.load_suite(Path(args.suite))
    fixtures = [
        n for n, f in sorted(suite["fixtures"].items()) if f["mode"] != "skip"
    ]
    flavors = {"suite"} | {flavor_for_fixture(n) for n in fixtures}

    # Gradle capitalizes only the first letter of the flavor in task names:
    # anchored_overlay -> assembleAnchored_overlayRelease.
    tasks = [
        f":app:assemble{f[0].upper() + f[1:]}Release" for f in sorted(flavors)
    ]
    tasks += [":macrobenchmark:assemble", ":baselineprofile:assemble"]
    print("gradle:", *tasks)
    _gradle(*tasks, project=project)

    # Propagate the suite-generated baseline profile into every per-fixture
    # variant's profile source (see BaselineProfileGenerator's docstring);
    # skipped quietly until generation has run once on device.
    gen = project / "app/src/suiteRelease/generated/baselineProfiles"
    propagated = 0
    if gen.is_dir():
        for flavor in flavors - {"suite"}:
            dst = project / f"app/src/{flavor}Release/generated/baselineProfiles"
            dst.mkdir(parents=True, exist_ok=True)
            for src_file in gen.glob("*"):
                if src_file.is_file():
                    (dst / src_file.name).write_bytes(src_file.read_bytes())
            propagated += 1

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    manifest = {"schema": "bench/android/artifacts@1", "apks": {}}
    for apk in _apk_out(project, "app"):
        # .../outputs/apk/<flavor>/release/app-<flavor>-arm64-v8a-release.apk
        flavor = apk.parts[-3].lower()
        for fixture in fixtures:
            if flavor_for_fixture(fixture) == flavor:
                dst = out_dir / f"{fixture}.apk"
                dst.write_bytes(apk.read_bytes())
                manifest["apks"][fixture] = {
                    "path": str(dst),
                    "apk_bytes": dst.stat().st_size,
                }
        if flavor == "suite":
            dst = out_dir / "suite.apk"
            dst.write_bytes(apk.read_bytes())
            manifest["apks"]["suite"] = {
                "path": str(dst),
                "apk_bytes": dst.stat().st_size,
            }
    for module, key in (("macrobenchmark", "macrobenchmark_apks"),
                        ("baselineprofile", "baselineprofile_apks")):
        manifest["apks"][key] = [
            {"path": str(p), "apk_bytes": p.stat().st_size}
            for p in _apk_out(project, module)
        ]
    manifest["baseline_profiles_propagated"] = propagated
    (out_dir / "artifacts.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    print(
        f"built {len(manifest['apks'])} artifacts -> {out_dir} "
        f"(profiles propagated to {propagated} variants)"
    )
    return 0


def cmd_verify_profile(args: argparse.Namespace) -> int:
    """Verify the baseline profile is installed and applied.

    Checks the package's dexopt state for the speed-profile compilation the
    benchmarks require (CompilationMode.Partial(BaselineProfileMode.Require)
    fails the benchmark when the profile is absent; this check makes the
    requirement visible before a campaign round).
    """
    serial = args.serial or metrics.detect_serial()
    pkg = package_for_fixture(args.fixture)
    dexopt = metrics.adb_out(
        serial, "shell", "dumpsys", "package", "dexopt"
    ).decode("utf-8", "replace")
    block = ""
    lines = dexopt.splitlines()
    for i, line in enumerate(lines):
        if line.strip() == pkg or line.strip().startswith(pkg + ":"):
            block = "\n".join(lines[i : i + 8])
            break
    status = "speed-profile" in block
    result = {
        "package": pkg,
        "dexopt_block": block,
        "speed_profile_applied": status,
    }
    print(json.dumps(result, indent=2))
    return 0 if status else 1


def cmd_instrument(args: argparse.Namespace) -> int:
    """Run one macrobenchmark test class against a fixture on the device.

    Example:
      run.py instrument --fixture list \\
          --class StartupBenchmark --extra-arg iterations=30
    """
    serial = args.serial or metrics.detect_serial()
    test_pkg = "dev.waterui.android.macrobenchmark.test"
    instr_args = {
        "fixture": args.fixture,
        "class": (
            "dev.waterui.android.macrobenchmark." + args.klass
        ),
        **dict(a.split("=", 1) for a in (args.extra_arg or [])),
    }
    cmd = ["am", "instrument", "-w"]
    for k, v in instr_args.items():
        cmd += ["-e", k, str(v)]
    cmd.append(f"{test_pkg}/androidx.test.runner.AndroidJUnitRunner")
    out = metrics.adb_out(serial, "shell", *cmd).decode("utf-8", "replace")
    print(out)
    # Pull the benchmark result payloads the test wrote to device media.
    media_dir = Path(args.out) / "benchmark-results"
    media_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            "adb", "-s", serial, "pull",
            f"/sdcard/Android/media/{test_pkg}",
            str(media_dir),
        ],
        capture_output=True,
    )
    print(f"results pulled -> {media_dir}")
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
        / "suite"
        / "java"
        / "dev"
        / "waterui"
        / "android"
        / "reference"
        / "SuiteDispatch.kt"
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
    # Suite-only coverage screens that are not frozen-inventory fixtures
    # (the editing suite and the 1,000-row stress case) are dispatched
    # through the same when-mapping but carry no fixture entry.
    twins_in_source -= {"editing", "list-stress"}
    if registered != twins_in_source:
        errors.append(
            f"twin set {sorted(registered)} != SuiteDispatch.kt "
            f"{sorted(twins_in_source)}"
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
    run.add_argument("--reference-apk", default=None,
                     help="built APK of bench/android/reference — required to "
                          "generate goldens for fixtures with registered twins")
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

    lock = sub.add_parser(
        "lock-campaign",
        help="resolve and freeze the stable Compose BOM for a campaign",
    )
    lock.add_argument("--refresh", action="store_true",
                      help="re-lock even when a campaign lock already exists")
    lock.set_defaults(func=cmd_lock_campaign)

    build = sub.add_parser(
        "build",
        help="build every release APK (per-fixture variants + suite + tests)",
    )
    build.add_argument("--suite", default=str(BENCH_DIR / "suite.toml"))
    build.add_argument("--out", default=str(BENCH_DIR / "out" / "apks"))
    build.set_defaults(func=cmd_build)

    verify = sub.add_parser(
        "verify-profile",
        help="check the installed package applied its baseline profile",
    )
    verify.add_argument("--fixture", required=True)
    verify.add_argument("--serial", default=None)
    verify.set_defaults(func=cmd_verify_profile)

    instr = sub.add_parser(
        "instrument",
        help="run a macrobenchmark test class for a fixture on the device",
    )
    instr.add_argument("--fixture", required=True)
    instr.add_argument("--class", dest="klass", required=True,
                       help="test class, e.g. StartupBenchmark, "
                            "FrameBenchmark, EnergyBenchmark")
    instr.add_argument("--serial", default=None)
    instr.add_argument("--extra-arg", action="append", default=None,
                       help="extra instrumentation args, key=value")
    instr.add_argument("--out", default=str(BENCH_DIR / "out"))
    instr.set_defaults(func=cmd_instrument)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())

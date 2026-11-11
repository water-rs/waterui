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
toolchain-lock.json as a frozen record of the old backend's measurement
semantics. Those scripts cannot run against the current CLI.

APK production: `build-hydrolysis` builds each fixture's Hydrolysis app
through `water package` for each painter (`gpu`, `hwui`, named as the CLI
names them) into `<out>/<painter>/<fixture>.apk`; `build` builds the Compose
reference with Gradle (release, R8, resource shrinking, a baseline profile
`run` verifies ART applied). Both are arm64-v8a and signed with the Gradle
debug keystore. `run --apk-dir <dir>` launches each fixture's APK, replays
its journey's input ops and writes the result record. `--painter` names the
Hydrolysis painter of the app under test and is absent for `--backend
compose`. A `build-hydrolysis --frame-profile` app additionally logs the
HWUI painter's per-frame counters, which `run` collects from logcat.

The Gradle trees (`reference/` here, `android/` for the host) are the
callers of `scripts/fetch-gradle-wrapper.py`: their `gradlew` scripts
need `gradle/wrapper/gradle-wrapper.jar`, which the repository does not
store. Run `uv run scripts/fetch-gradle-wrapper.py` once before any
`./gradlew` invocation — it materializes the jar for every wrapper in
the tree, hash-verified against the published checksums.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

# The Android painters the Hydrolysis host draws with, spelled as
# `water package --painter` spells them. Result records carry these names.
PAINTERS = ("gpu", "hwui")

# Records of the Compose reference name no Hydrolysis painter.
COMPOSE_PAINTER = "none"

BENCH_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(BENCH_DIR))
sys.path.insert(0, str(BENCH_DIR / "scripts"))

import metrics  # noqa: E402
from metrics import frames as frame_metrics  # noqa: E402
from metrics import memory as memory_metrics  # noqa: E402
from metrics import input_latency as latency_metrics  # noqa: E402
from metrics import frame_profile  # noqa: E402

# The imported frozen implementation; see toolchain-lock.json for its hash.
import e2e  # noqa: E402


def fixture_env(fixture: dict, defaults: dict) -> dict:
    """The intent extras CI applies: shared defaults plus per-fixture env."""
    env = dict(defaults.get("env", {}))
    env.update(fixture.get("env", {}))
    return env


def sdk_tool(*parts: str) -> str:
    """The newest `build-tools` binary named by `parts` under the SDK."""
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if not sdk:
        sys.exit("ANDROID_HOME/ANDROID_SDK_ROOT unset — cannot locate build-tools")
    candidates = sorted(
        Path(sdk).glob("build-tools/*/" + "/".join(parts)),
        key=lambda path: [int(p) for p in path.parent.name.split(".")],
    )
    if not candidates:
        sys.exit(f"no {'/'.join(parts)} under {sdk}/build-tools")
    return str(candidates[-1])


def apk_package(apk: Path) -> str:
    """The application id `apk` installs as — read from the APK, because the
    Hydrolysis app's bundle identifier comes from its Water.toml, not from the
    frozen CI inventory's `package`."""
    return subprocess.run(
        [sdk_tool("aapt2"), "dump", "packagename", str(apk)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def start_launcher(serial: str, package: str, env: dict) -> None:
    """Starts `package`'s launcher activity, killing any prior run — the
    same intent for the generated Hydrolysis host and the Compose reference,
    so neither depends on the other's activity name. `env` reaches the app as
    `waterui.env.*` string extras."""
    resolved = e2e.adb_out(
        serial, "shell", "cmd", "package", "resolve-activity", "--brief",
        "-a", "android.intent.action.MAIN",
        "-c", "android.intent.category.LAUNCHER",
        package,
    ).decode("utf-8", "replace").strip().splitlines()
    component = resolved[-1].strip() if resolved else ""
    if not component.startswith(package + "/"):
        sys.exit(f"{package} has no launcher activity: {resolved}")
    args = ["shell", "am", "start", "-S", "-n", component]
    for key, value in env.items():
        args += ["--es", f"waterui.env.{key}", value]
    output = e2e.adb_out(serial, *args).decode("utf-8", "replace")
    if "Error" in output:
        sys.exit(f"am start {package} failed: {output.strip()}")


# Scroll and swipe geometry of the shared journey runner (shared/kotlin
# Journey.kt `swipeDirection`): a full-screen fling between 20% and 80% of
# the display, along the centre line.
_FLING = {
    "up": (0.5, 0.8, 0.5, 0.2),
    "down": (0.5, 0.2, 0.5, 0.8),
    "left": (0.8, 0.5, 0.2, 0.5),
    "right": (0.2, 0.5, 0.8, 0.5),
}
# UiObject.scroll moves the content, so the finger travels the other way.
_SCROLL_FINGER = {"down": "up", "up": "down", "left": "right", "right": "left"}
FLING_MS = 120


def prepare_display(serial: str) -> None:
    """Wakes the display and clears the lock screen and notification shade, so
    the app under test owns the focused window the settle check demands."""
    metrics.adb(serial, "shell", "input", "keyevent", "KEYCODE_WAKEUP")
    metrics.adb(serial, "shell", "wm", "dismiss-keyguard")
    metrics.adb(serial, "shell", "cmd", "statusbar", "collapse")


def display_size(serial: str) -> tuple[int, int]:
    out = metrics.adb_out(serial, "shell", "wm", "size").decode()
    match = re.search(r"Physical size:\s*(\d+)x(\d+)", out)
    if match is None:
        sys.exit(f"cannot read the display size from `wm size`: {out!r}")
    return int(match.group(1)), int(match.group(2))


def replay_journey(serial: str, steps: list[dict], size: tuple[int, int]) -> list[str]:
    """Runs the shared journey's input-only ops through `adb shell input`.

    Returns the ops it could not run: selector ops (`tap`, `drag`,
    `input_text`, ...) need the UiAutomator runner in the macrobenchmark
    module, so a journey containing one replays only its coordinate ops and
    the record says which were left out.
    """
    width, height = size
    unsupported: list[str] = []
    for step in steps:
        op = step["op"]
        if op == "settle":
            continue  # the launch already waited for a settled frame
        if op == "wait_ms":
            time.sleep(step["ms"] / 1000)  # the spec's own scripted pacing
        elif op in ("swipe", "scroll"):
            direction = step["direction"]
            if op == "scroll":
                direction = _SCROLL_FINGER[direction]
            fx0, fy0, fx1, fy1 = _FLING[direction]
            for _ in range(step.get("times", 1)):
                metrics.adb(
                    serial, "shell", "input", "swipe",
                    str(int(width * fx0)), str(int(height * fy0)),
                    str(int(width * fx1)), str(int(height * fy1)),
                    str(FLING_MS),
                )
        elif op == "back":
            metrics.adb(serial, "shell", "input", "keyevent", "4")
        elif op == "key":
            metrics.adb(serial, "shell", "input", "keyevent", str(step["code"]))
        elif op == "home":
            metrics.adb(serial, "shell", "input", "keyevent", "3")
        elif op == "repeat":
            for _ in range(step.get("times", 1)):
                unsupported += replay_journey(serial, step["steps"], size)
        else:
            unsupported.append(op)
    return unsupported


def launch_and_capture(
    serial: str,
    name: str,
    fixture: dict,
    apk: Path,
    defaults: dict,
    artifacts: Path,
    painter: str | None,
    journey: list[dict] | None,
) -> dict:
    """One launch: install, start, settle, verify or smoke-check, replay the
    fixture's journey, then collect the round's frame and painter counters.

    Returns the round's result `metrics` payload; artifacts land under
    `artifacts/<name>/`.
    """
    out = artifacts / name
    out.mkdir(parents=True, exist_ok=True)
    package = apk_package(apk)
    mode = fixture["mode"]
    settle_s = fixture.get(
        "settle_timeout_seconds", defaults["settle_timeout_seconds"]
    )
    poll_s = fixture.get("poll_interval_ms", defaults["poll_interval_ms"]) / 1000

    prepare_display(serial)
    e2e.install_apk(serial, str(apk))
    e2e.force_stop(serial, package)
    if painter is None:
        # The Compose reference ships a baseline profile; verify ART applied
        # it rather than assuming the APK entry did its job.
        compile_with_profile(serial, package)
        applied, block = speed_profile_state(serial, package)
        if not applied:
            sys.exit(f"{package}: baseline profile not applied:\n{block}")
        result_profile = {"baseline_profile_applied": True}
    else:
        result_profile = {}
    # Reset the frame statistics and the log so the round counts only its
    # own frames and painter lines.
    metrics.adb(serial, "shell", "dumpsys", "gfxinfo", package, "reset")
    frame_profile.clear_logcat(serial)
    launch_at = time.monotonic()
    start_launcher(serial, package, fixture_env(fixture, defaults))
    watch = e2e.ProcessWatch(serial, package)
    result: dict = {
        "apk_bytes": apk.stat().st_size,
        "package": package,
        **result_profile,
    }

    anr: list[str] = []
    foreign: list[str] = []
    settled, frame, first_content_at = e2e.wait_for_settle(
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
    if first_content_at is not None:
        # Both instants are host monotonic seconds; the resolution is the
        # settle poll interval, so this orients the round, it does not gate.
        result["first_content_ms"] = int((first_content_at - launch_at) * 1000)

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

    # The short interaction: the shared journey's input-only ops, so the
    # frames below include scrolling work and not only the launch.
    if journey is None:
        result["replay"] = "no journey in interactions.json"
    else:
        unsupported = replay_journey(serial, journey, display_size(serial))
        result["replay"] = "partial" if unsupported else "complete"
        if unsupported:
            result["replay_unsupported_ops"] = sorted(set(unsupported))
    if painter is not None:
        result.update(frame_profile.collect(serial, painter, out, "frame_profile"))

    # Round metrics: frames over the launch + interaction window and a memory
    # snapshot. The expensive dumps stay a separate replay per section 6 —
    # here they run in the round because the round IS the measured script.
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
    if args.backend == "hydrolysis":
        if args.painter not in PAINTERS:
            sys.exit(f"--painter {' or '.join(PAINTERS)} is required for the hydrolysis backend")
        painter = args.painter
    else:
        if args.painter is not None:
            sys.exit(f"--painter names a Hydrolysis painter; backend {args.backend!r} has none")
        painter = None
    interactions = json.loads((BENCH_DIR / "interactions" / "interactions.json").read_text())
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
        if args.no_golden and fixture["mode"] == "verify":
            # Settled non-blank content is the check; the round measures
            # frames and counters, not pixels.
            fixture = {**fixture, "mode": "smoke"}
        print(f"== {name}: {fixture['mode']} on {serial} ({apk.name})")
        record = metrics.make_result(
            backend=args.backend,
            painter=painter or COMPOSE_PAINTER,
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
                painter,
                interactions["fixtures"].get(name, {}).get("journey"),
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


def gradle_env() -> dict:
    """The environment Gradle runs under: the caller's, with the CI image's
    JDK 21 where that path exists and `java` on PATH (JDK 21) elsewhere."""
    env = dict(os.environ)
    jdk = Path("/usr/lib/jvm/java-21-openjdk-amd64")
    if jdk.is_dir():
        env.setdefault("JAVA_HOME", str(jdk))
    return env


def _gradle(*tasks: str, project: Path) -> None:
    env = gradle_env()
    subprocess.run(
        [str(project / "gradlew"), *tasks],
        cwd=project,
        env=env,
        check=True,
    )


def _apk_out(project: Path, module: str) -> list[Path]:
    root = project / module / "build" / "outputs" / "apk"
    return sorted(root.glob("**/*.apk")) if root.is_dir() else []


BASELINE_PROFILE_ENTRY = "assets/dexopt/baseline.prof"


def apk_has_baseline_profile(apk: Path) -> bool:
    """Whether `apk` ships the compiled baseline profile ART installs."""
    import zipfile

    with zipfile.ZipFile(apk) as archive:
        return BASELINE_PROFILE_ENTRY in archive.namelist()


_DEPENDENCY = re.compile(
    r"([\w.\-]+:[\w.\-]+)(?::([\w.\-]+))?(?:\s*->\s*([\w.\-]+))?(?:\s|$)"
)


def record_resolved_dependencies(project: Path, flavors: list[str]) -> int:
    """Records every dependency Gradle resolved for the release runtime
    classpath into campaign-lock.json, so the campaign's BOM resolution is
    auditable and cannot drift mid-campaign. Returns the dependency count.
    """
    flavor = next((f for f in flavors if f != "suite"), "suite")
    configuration = f"{flavor[0].lower() + flavor[1:]}ReleaseRuntimeClasspath"
    env = gradle_env()
    tree = subprocess.run(
        [str(project / "gradlew"), "-q", ":app:dependencies",
         "--configuration", configuration],
        cwd=project, env=env, check=True, capture_output=True, text=True,
    ).stdout
    resolved: dict[str, str] = {}
    for line in tree.splitlines():
        match = _DEPENDENCY.search(line)
        version = match and (match.group(3) or match.group(2))
        if version:
            # `a:b:1.0 -> 2.0` and BOM-managed `a:b -> 2.0` resolve to 2.0.
            resolved[match.group(1)] = version
    lock_path = BENCH_DIR / "campaign-lock.json"
    lock = json.loads(lock_path.read_text())
    lock["resolved_dependencies"] = dict(sorted(resolved.items()))
    lock["resolved_dependencies_configuration"] = configuration
    lock_path.write_text(json.dumps(lock, indent=2, sort_keys=True) + "\n")
    return len(resolved)


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
    fixtures = args.fixture or [
        n for n, f in sorted(suite["fixtures"].items()) if f["mode"] != "skip"
    ]
    for name in fixtures:
        if name not in suite["fixtures"]:
            sys.exit(f"unknown fixture {name!r}; not in suite.toml")
        if suite["fixtures"][name]["mode"] == "skip":
            sys.exit(f"{name} is a documented skip: {suite['fixtures'][name]['reason']}")
    flavors = {flavor_for_fixture(n) for n in fixtures}
    if not args.fixture:
        flavors.add("suite")

    # Propagate the suite-generated baseline profile into every per-fixture
    # variant's profile source (see BaselineProfileGenerator's docstring)
    # BEFORE assembling, so the APKs carry it. `generate-profile` produces the
    # source on the device; without it nothing below can ship a profile.
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

    # Gradle capitalizes only the first letter of the flavor in task names:
    # anchored_overlay -> assembleAnchored_overlayRelease.
    tasks = [
        f":app:assemble{f[0].upper() + f[1:]}Release" for f in sorted(flavors)
    ]
    if not args.app_only:
        tasks += [":macrobenchmark:assemble", ":baselineprofile:assemble"]
    print("gradle:", *tasks)
    _gradle(*tasks, project=project)

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
                    "baseline_profile_in_apk": apk_has_baseline_profile(dst),
                }
        if flavor == "suite" and "suite" in flavors:
            dst = out_dir / "suite.apk"
            dst.write_bytes(apk.read_bytes())
            manifest["apks"]["suite"] = {
                "path": str(dst),
                "apk_bytes": dst.stat().st_size,
                "baseline_profile_in_apk": apk_has_baseline_profile(dst),
            }
    if not args.app_only:
        for module, key in (("macrobenchmark", "macrobenchmark_apks"),
                            ("baselineprofile", "baselineprofile_apks")):
            manifest["apks"][key] = [
                {"path": str(p), "apk_bytes": p.stat().st_size}
                for p in _apk_out(project, module)
            ]
    manifest["baseline_profiles_propagated"] = propagated
    manifest["resolved_dependencies"] = record_resolved_dependencies(
        project, sorted(flavors)
    )
    (out_dir / "artifacts.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    print(
        f"built {len(manifest['apks'])} artifacts -> {out_dir} "
        f"(profiles propagated to {propagated} variants)"
    )
    unprofiled = [
        name
        for name, entry in manifest["apks"].items()
        if isinstance(entry, dict) and not entry["baseline_profile_in_apk"]
    ]
    if unprofiled:
        print(
            "BASELINE PROFILE MISSING from: " + ", ".join(unprofiled)
            + " — run `run.py generate-profile` on the device, then rebuild"
        )
        return 1
    return 0


# `water package` has no feature switch, so a `frame-profile` build enables
# the feature through the application's own manifest: Cargo unifies the
# feature onto the launcher's `hydrolysis`. The stanza is appended for the
# duration of one build only.
_FRAME_PROFILE_STANZA = """
[target.'cfg(target_os = "android")'.dependencies]
hydrolysis = { workspace = true, features = ["frame-profile"] }
"""


@contextlib.contextmanager
def frame_profile_manifest(project: Path, lock: Path):
    """Temporarily adds the `frame-profile` dependency to `project`'s manifest
    and restores the manifest and the workspace `lock` the edit rewrites."""
    manifest = project / "Cargo.toml"
    original = manifest.read_text()
    original_lock = lock.read_text()
    if "target_os = \"android\"" in original and "hydrolysis" in original:
        sys.exit(f"{manifest} already declares an Android hydrolysis dependency")
    manifest.write_text(original + _FRAME_PROFILE_STANZA)
    try:
        yield
    finally:
        manifest.write_text(original)
        lock.write_text(original_lock)


def debug_keystore() -> Path:
    """The keystore the Compose release build signs with: Gradle's own debug
    keystore (`signingConfigs.getByName("debug")`), so both apps share one
    signing mode."""
    path = Path.home() / ".android" / "debug.keystore"
    if not path.is_file():
        sys.exit(f"{path} is missing — run any Gradle Android build once to create it")
    return path


def package_hydrolysis(
    water: str,
    waterui: Path,
    fixture: dict,
    painter: str,
    frame_profile_build: bool,
    debug: bool,
    log: Path,
) -> Path:
    """Builds one fixture's Hydrolysis APK through `water package` and returns
    the artifact it reports, unsigned."""
    project = waterui / fixture["source"]
    cmd = [
        water, "package",
        "--platform", "android", "--backend", "hydrolysis",
        "--painter", painter,
        "--arch", "arm64",
        "--path", str(project),
        "--yes",
        # A release package is left unsigned for `sign_with_debug_keystore`;
        # a debug package is signed by the CLI.
        *(["--debug"] if debug else ["--unsigned"]),
    ]
    with contextlib.ExitStack() as stack:
        if frame_profile_build:
            stack.enter_context(frame_profile_manifest(project, waterui / "Cargo.lock"))
        # The build's output goes to a log as it runs, so a long cold build
        # can be followed and a failure read in full.
        log.parent.mkdir(parents=True, exist_ok=True)
        print("water:", " ".join(cmd[1:]), f"(log: {log})", flush=True)
        with open(log, "w") as sink:
            done = subprocess.run(cmd, stdout=sink, stderr=subprocess.STDOUT)
    output = log.read_text()
    if done.returncode != 0:
        sys.stdout.write(output)
        sys.exit(f"water package failed for {project} ({painter}): exit {done.returncode}")
    ansi = re.compile(r"\x1b\[[0-9;]*m")
    match = re.search(r"Packaged at (\S+\.apk)", ansi.sub("", output))
    if match is None:
        sys.exit(f"water package reported no .apk artifact for {project}")
    return Path(match.group(1))


def sign_with_debug_keystore(apk: Path, dst: Path) -> None:
    """Signs `apk` into `dst` with the shared debug keystore."""
    dst.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            sdk_tool("apksigner"), "sign",
            "--ks", str(debug_keystore()),
            "--ks-key-alias", "androiddebugkey",
            "--ks-pass", "pass:android",
            "--key-pass", "pass:android",
            "--out", str(dst),
            str(apk),
        ],
        check=True,
    )


def cmd_build_hydrolysis(args: argparse.Namespace) -> int:
    """Build the Hydrolysis app of each fixture for each painter.

    Each APK is arm64-v8a, built by `water package` for the release profile
    and signed with the keystore the Compose reference signs with. They land
    in --out/<painter>/<fixture>.apk — the layout `run --apk-dir` reads — with
    their sizes in --out/artifacts.json.
    """
    suite = metrics.load_suite(Path(args.suite))
    waterui = Path(args.waterui)
    selected = args.fixture or sorted(
        n for n, f in suite["fixtures"].items() if f["mode"] != "skip"
    )
    manifest: dict = {"schema": "bench/android/artifacts@1", "apks": {}}
    out_dir = Path(args.out)
    for painter in args.painter or PAINTERS:
        for name in selected:
            fixture = suite["fixtures"].get(name)
            if fixture is None:
                sys.exit(f"unknown fixture {name!r}; not in suite.toml")
            if fixture["mode"] == "skip":
                sys.exit(f"{name} is a documented skip: {fixture['reason']}")
            built = package_hydrolysis(
                args.water, waterui, fixture, painter, args.frame_profile,
                args.debug, out_dir / "logs" / f"{painter}-{name}.water.log",
            )
            dst = out_dir / painter / f"{name}.apk"
            if args.debug:
                print("DEBUG build: its size and speed are not measurements")
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(built, dst)
            else:
                sign_with_debug_keystore(built, dst)
            manifest["apks"][f"{painter}/{name}"] = {
                "path": str(dst),
                "apk_bytes": dst.stat().st_size,
                "painter": painter,
                "frame_profile": args.frame_profile,
                "profile": "debug" if args.debug else "release",
            }
            print(f"built {painter}/{name}: {dst.stat().st_size} bytes -> {dst}")
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "artifacts.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    return 0


def cmd_generate_profile(args: argparse.Namespace) -> int:
    """Generate the suite variant's baseline profile on the attached device.

    Runs the shared journeys (all of them, or --fixture's subset) against the
    suite APK through the androidx.baselineprofile producer module; the
    profile lands in app/src/suiteRelease/generated/baselineProfiles, where
    `build` propagates it into every per-fixture variant. Needs the device
    held by the caller.
    """
    project = BENCH_DIR / "reference"
    task = ":app:generateSuiteReleaseBaselineProfile"
    # The producer needs the suite package as `targetAppId`. With plugin 1.5.0
    # a run that passed only a `-Pandroid.testInstrumentationRunnerArguments.*`
    # property failed with "targetAppId not passed", so it is passed here.
    extra = [
        "-Pandroid.testInstrumentationRunnerArguments.targetAppId="
        + package_for_fixture("suite")
    ]
    if args.fixture:
        extra.append(
            "-Pandroid.testInstrumentationRunnerArguments.fixtures="
            + ",".join(args.fixture)
        )
    if args.max_iterations is not None:
        extra += [
            "-Pandroid.testInstrumentationRunnerArguments.maxIterations="
            f"{args.max_iterations}",
            "-Pandroid.testInstrumentationRunnerArguments.stableIterations="
            f"{min(3, args.max_iterations)}",
        ]
    print("gradle:", task, *extra)
    _gradle(task, *extra, project=project)
    generated = project / "app/src/suiteRelease/generated/baselineProfiles"
    files = sorted(p.name for p in generated.glob("*")) if generated.is_dir() else []
    if not files:
        sys.exit(f"generation produced no profile under {generated}")
    print(f"generated {', '.join(files)} -> {generated}")
    return 0


def compile_with_profile(serial: str, package: str) -> None:
    """Installs the APK's baseline profile and forces the profile-guided
    compilation `BaselineProfileMode.Require` expects, once at install and
    outside any measured window.

    `cmd package compile -m speed-profile` only has a profile to compile
    against after ProfileInstaller moved the APK's into ART's profile
    directory, which the broadcast asks of the app's own receiver.
    """
    broadcast = metrics.adb_out(
        serial, "shell", "am", "broadcast",
        "-a", "androidx.profileinstaller.action.INSTALL_PROFILE",
        "-n", f"{package}/androidx.profileinstaller.ProfileInstallReceiver",
    ).decode("utf-8", "replace")
    # ProfileInstaller.RESULT_INSTALL_SUCCESS / RESULT_ALREADY_INSTALLED.
    if not re.search(r"result=(1|2)\b", broadcast):
        sys.exit(f"{package}: ProfileInstaller did not install the profile: {broadcast.strip()}")
    out = metrics.adb_out(
        serial, "shell", "cmd", "package", "compile",
        "-f", "-m", "speed-profile", package,
    ).decode("utf-8", "replace")
    if "Success" not in out:
        sys.exit(f"cmd package compile {package} failed: {out.strip()}")


def speed_profile_state(serial: str, package: str) -> tuple[bool, str]:
    """Whether `package`'s dexopt state is `speed-profile`, and that state's
    text — the evidence the installed baseline profile was applied."""
    dump = metrics.adb_out(
        serial, "shell", "dumpsys", "package", package
    ).decode("utf-8", "replace")
    section = dump.split("Dexopt state:", 1)
    block = ""
    if len(section) == 2:
        lines = section[1].splitlines()
        block = "\n".join(line.rstrip() for line in lines[:8])
    return "status=speed-profile" in block, block


def cmd_verify_profile(args: argparse.Namespace) -> int:
    """Verify the baseline profile is installed and applied.

    Checks the package's dexopt state for the speed-profile compilation the
    benchmarks require (CompilationMode.Partial(BaselineProfileMode.Require)
    fails the benchmark when the profile is absent; this check makes the
    requirement visible before a campaign round).
    """
    serial = args.serial or metrics.detect_serial()
    pkg = package_for_fixture(args.fixture)
    status, block = speed_profile_state(serial, pkg)
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
    for name, fixture in fixtures.items():
        skipped = fixture.get("compose_twin") == "skipped"
        if (fixture["mode"] == "skip") != skipped:
            errors.append(
                f"{name}: compose_twin {fixture.get('compose_twin')!r} "
                f"disagrees with mode {fixture['mode']!r} — exactly the "
                "documented skip entries carry compose_twin = \"skipped\""
            )
        if skipped and not fixture.get("compose_twin_skip_reason"):
            errors.append(f"{name}: a skipped twin needs compose_twin_skip_reason")
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
    run.add_argument("--no-golden", action="store_true",
                     help="check settled non-blank content only, for verify "
                          "fixtures too — no golden is read or generated")
    run.add_argument("--run-skipped", action="store_true",
                     help="also attempt the two documented skip entries")
    run.add_argument("--backend", default="hydrolysis")
    run.add_argument("--painter", choices=PAINTERS, default=None,
                     help="Android painter of the Hydrolysis app under test, "
                          "named as `water package --painter` names it; "
                          "required for --backend hydrolysis, absent for the "
                          "Compose reference (--backend compose)")
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
    build.add_argument("--fixture", action="append", default=None,
                       help="build only this fixture's variant (repeatable)")
    build.add_argument("--app-only", action="store_true",
                       help="skip the macrobenchmark and baselineprofile APKs")
    build.set_defaults(func=cmd_build)

    build_hydrolysis = sub.add_parser(
        "build-hydrolysis",
        help="build each fixture's Hydrolysis app per painter through "
             "`water package`",
    )
    build_hydrolysis.add_argument("--suite", default=str(BENCH_DIR / "suite.toml"))
    build_hydrolysis.add_argument("--waterui", required=True,
                                  help="waterui checkout holding the fixtures' sources")
    build_hydrolysis.add_argument("--water", default="water",
                                  help="the `water` CLI to build with")
    build_hydrolysis.add_argument("--painter", action="append", choices=PAINTERS,
                                  default=None,
                                  help="painter to build (repeatable; default both)")
    build_hydrolysis.add_argument("--fixture", action="append", default=None)
    build_hydrolysis.add_argument("--frame-profile", action="store_true",
                                  help="build with the `frame-profile` feature so "
                                       "the HWUI painter logs its per-frame counters "
                                       "— an attribution build, not a measured one")
    build_hydrolysis.add_argument("--debug", action="store_true",
                                  help="build the unoptimized profile, for a "
                                       "plumbing smoke run on a loaded host — "
                                       "never a measured build")
    build_hydrolysis.add_argument("--out", default=str(BENCH_DIR / "out" / "hydrolysis"))
    build_hydrolysis.set_defaults(func=cmd_build_hydrolysis)

    generate = sub.add_parser(
        "generate-profile",
        help="generate the Compose reference's baseline profile on the device",
    )
    generate.add_argument("--fixture", action="append", default=None,
                          help="journey subset to profile (repeatable; default all)")
    generate.add_argument("--max-iterations", type=int, default=None,
                          help="bound on the collection loop (library default 15)")
    generate.set_defaults(func=cmd_generate_profile)

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
    # A deadline's SIGTERM unwinds as SystemExit so `finally` blocks restore
    # what a build temporarily edited and `subprocess.run` kills its child.
    signal.signal(signal.SIGTERM, lambda _signo, _frame: sys.exit(143))
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())

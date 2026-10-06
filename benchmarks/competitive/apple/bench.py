#!/usr/bin/env python3
"""Competitive benchmark runner — iOS on the physical iPhone
(water-rs/waterui#1262, harness #1864).

Run with `uv run bench.py <command>` (stdlib only). Apple is measured on
one platform: the iPhone the manifest declares (`device`), attached to
the device host. Two hosts take part, each with the one Xcode it has:

  build host (macOS with flutter, CocoaPods, node, Rust, the water CLI):
    bootstrap                  fresh-clone dependency install
    build                      build every contestant for the iphoneos
                               SDK, unsigned; stage them with a staging
                               manifest and pack build/ios-stage-<head>.tar.gz

  device host (the Mac the iPhone is attached to; never compiles a
  contestant, builds only the XCTest runner with its own Xcode):
    device-session start --stage <tar.gz> --stage-sha256 <hex> [selection]
                               submit one measurement run as a LaunchAgent
                               job in the user's GUI (Aqua) session — xctrace,
                               DTServiceHub and the login keychain live
                               there, not in an ssh session — and return
    device-session status --run-dir <dir>
    device-session stop   --run-dir <dir>
    device-run --run-dir <dir> the job itself (launchd starts it)

  anywhere:
    report --input results.json [--out report.md]
    attribution --trace T --app A --max-fps F

Every contestant is measured the same way: the shared BenchRunner XCUITest
bundle drives the app, one all-process recording per launch (Animation
Hitches + Points of Interest + os_log) gives the frames, the runner's
marks, CPU samples and the WaterUI first-paint marker, and XCTest
metrics (launch, memory, CPU, hitch, scroll signposts) are collected
externally. No per-contestant shortcuts.
"""
from __future__ import annotations

import sys

if sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {sys.version.split()[0]}); it declares its "
        "version in pyproject.toml + .python-version and runs under the "
        "uv-managed interpreter (`uv run`)")

import argparse
import dataclasses
import datetime
import fcntl
import hashlib
import json
import os
import plistlib
import re
import shutil
import signal
import subprocess
import tarfile
import tempfile
import threading
import time
import traceback
import urllib.parse
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "lib"))
import toolchain  # benchmarks/competitive/lib/toolchain.py
import frame_stats as lib_frames  # benchmarks/competitive/lib/frame_stats.py

ROOT = Path(__file__).resolve().parent
MANIFEST = json.loads((ROOT / "manifest.json").read_text())
LOCK_DIR = Path("/tmp/device-locks")
STAGE_DIR = ROOT / "build" / "stage"
RUNS_DIR = ROOT / "build" / "device-runs"
RUNNER_PROJECT = ROOT / "xctest"


# ------------------------------------------------------------ processes

_ACTIVE_PROCS: list = []


def _kill_proc(p):
    """Kill a tracked process AND its grandchildren: every spawn is
    session-led (start_new_session), so the whole group dies — a plain
    p.kill() only takes out the /bin/sh wrapper and orphans xcodebuild/
    xctrace under it."""
    try:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL)
    except ProcessLookupError:
        pass


def _kill_active_procs():
    """Kill every tracked child process (xcodebuild, xctrace, builds).
    Called on signals so an interrupted run never leaves an orphaned
    build/test process driving the shared machine."""
    for p in list(_ACTIVE_PROCS):
        _kill_proc(p)


class CommandTimeout(RuntimeError):
    """A command outlived its declared bound; its process group was
    killed."""


def sh(cmd, cwd=ROOT, env=None, check=True, capture=False, timeout=None):
    """Run `cmd` session-led and tracked. A non-zero exit (with `check`)
    raises RuntimeError; outliving `timeout` kills the group and raises
    CommandTimeout — there is no return value standing for a timeout."""
    print(f"$ {cmd}", flush=True)
    e = dict(os.environ)
    e.setdefault("LANG", "en_US.UTF-8")
    e.setdefault("LC_ALL", "en_US.UTF-8")
    if env:
        e.update(env)
    p = subprocess.Popen(cmd, shell=True, cwd=cwd, env=e, text=True,
                         stdout=subprocess.PIPE if capture else None,
                         stderr=subprocess.PIPE if capture else None,
                         start_new_session=True)
    _ACTIVE_PROCS.append(p)
    try:
        out, err = p.communicate(timeout=timeout)
        r = subprocess.CompletedProcess(cmd, p.returncode, out, err)
    except subprocess.TimeoutExpired:
        _kill_proc(p)
        p.wait()
        raise CommandTimeout(
            f"command timed out after {timeout}s: {cmd}") from None
    finally:
        if p in _ACTIVE_PROCS:
            _ACTIVE_PROCS.remove(p)
    if check and r.returncode != 0:
        tail = (r.stdout or "")[-3000:] + (r.stderr or "")[-3000:]
        raise RuntimeError(f"command failed ({r.returncode}): {cmd}\n{tail}")
    return r


def _out(cmd: list[str], timeout: int = 60) -> str:
    """stdout of a short probe; a failing probe raises with its stderr."""
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)} failed rc={r.returncode}: "
                           f"{(r.stderr or r.stdout).strip()[-400:]}")
    return r.stdout.strip()


# ------------------------------------------------------- declared bounds
#
# Every device-side step carries a declared bound. The ones a stopping
# job can still take after launchd's SIGTERM add up to the job's
# ExitTimeOut (JOB_EXIT_TIMEOUT_S), so `device-session stop` never
# SIGKILLs it mid-cleanup.

DEVICECTL_QUERY_S = 60       # devicectl device info details / info apps
DEVICECTL_COPY_S = 120       # devicectl device copy to / from
DEVICECTL_INSTALL_S = 300    # devicectl device install app
DEVICECTL_UNINSTALL_S = 120  # devicectl device uninstall app
RECORDER_ARM_S = 120         # xctrace record → its "recording" line
RECORDER_STOP_S = 300        # xctrace SIGINT → the .trace written
RECORDER_DRAIN_S = 10        # the recorder's pty reader thread ending
LSOF_S = 60                  # every lsof of one scratch sweep, together
HUB_EXIT_S = 60              # DTServiceHub SIGTERM → its exit
# rmtree of the cell's .trace/.xcresult bundles and the status.json
# write while the job unwinds — local file operations without a
# subprocess, so no timeout bounds them; this is their allowance
LOCAL_FS_S = 60
# apps a run ever has installed at once: the runner and the contestant
# under measurement
MAX_INSTALLED_APPS = 2
# clear_bundle_id: list, uninstall, list again to verify
UNINSTALL_S = 2 * DEVICECTL_QUERY_S + DEVICECTL_UNINSTALL_S
# What a SIGTERM can still cost, in the order it is paid:
#   on_signal: kill this run's tracked processes (SIGKILL, immediate),
#     then uninstall and verify every installed app
#       MAX_INSTALLED_APPS × UNINSTALL_S
#   run_one's finally: stop the one recorder
#       RECORDER_STOP_S + RECORDER_DRAIN_S
#     then sweep the cell's scratch
#       LSOF_S + HUB_EXIT_S
#   the remaining finally blocks: run_device's per-contestant uninstall
#     finds nothing installed (on_signal's cleanup emptied the list),
#     then trace/xcresult removal and status.json
#       LOCAL_FS_S
# = 2 × 240 + 310 + 120 + 60 = 970 s.
JOB_EXIT_TIMEOUT_S = (MAX_INSTALLED_APPS * UNINSTALL_S
                      + RECORDER_STOP_S + RECORDER_DRAIN_S
                      + LSOF_S + HUB_EXIT_S
                      + LOCAL_FS_S)


# ---------------------------------------------------------- host identity

def host_evidence() -> dict:
    """Evidence that identifies a host's real hardware and OS. Recorded
    for the build host in the staging manifest and for the device host
    on every results file (re-verified on resume). A sysctl or ioreg
    probe that fails, or an ioreg answer without IOPlatformUUID, raises:
    a host whose identity cannot be read is never recorded as one."""
    def _sysctl(k):
        v = _out(["sysctl", "-n", k], timeout=30)
        if not v:
            raise RuntimeError(f"sysctl -n {k} printed nothing")
        return v

    def _ioreg_uuid():
        out = _out(["ioreg", "-rd1", "-c", "IOPlatformExpertDevice"],
                   timeout=30)
        m = re.search(r'"IOPlatformUUID"\s*=\s*"([^"]+)"', out)
        if m is None:
            raise RuntimeError("ioreg IOPlatformExpertDevice reports no "
                               "IOPlatformUUID")
        return m.group(1)

    return {"hw_model": _sysctl("hw.model"),
            "hw_uuid": _ioreg_uuid(),
            "cpu_brand": _sysctl("machdep.cpu.brand_string"),
            "hv_vmm_present": _sysctl("kern.hv_vmm_present") == "1",
            "os": f"{_out(['sw_vers', '-productName'])} "
                  f"{_out(['sw_vers', '-productVersion'])} "
                  f"({_out(['sw_vers', '-buildVersion'])})"}


def xcode_identity() -> dict:
    """The Xcode this host actually uses — recorded on every host, never
    pinned. The host must report exactly one Xcode: the active developer
    dir lies inside one Xcode .app and `xcodebuild -version` names one
    Xcode version and one build. The Xcode bundles installed under
    /Applications are recorded alongside as evidence."""
    dev = Path(_out(["xcode-select", "-p"])).resolve()
    bundle = next((p for p in (dev, *dev.parents) if p.suffix == ".app"),
                  None)
    if bundle is None:
        raise SystemExit(
            f"the active developer dir {dev} is not inside an Xcode .app "
            "(Command Line Tools alone cannot build or record) — select "
            "the host's Xcode with xcode-select")
    lines = _out(["xcodebuild", "-version"]).splitlines()
    versions = [ln for ln in lines if ln.startswith("Xcode ")]
    builds = [ln for ln in lines if ln.startswith("Build version ")]
    if len(versions) != 1 or len(builds) != 1:
        raise SystemExit(
            "xcodebuild -version does not report exactly one Xcode: "
            f"{lines!r}")
    return {"app": str(bundle),
            "developer_dir": str(dev),
            "version": versions[0].removeprefix("Xcode ").strip(),
            "build": builds[0].removeprefix("Build version ").strip(),
            "iphoneos_sdk": _out(["xcrun", "--sdk", "iphoneos",
                                  "--show-sdk-version"]),
            "iphoneos_sdk_build": _out(["xcrun", "--sdk", "iphoneos",
                                        "--show-sdk-build-version"]),
            "installed": sorted(str(p) for p in
                                Path("/Applications").glob("Xcode*.app"))}


def water_bin() -> str:
    """The `water` CLI built from THIS checkout (cli/ is a workspace
    member) — `cargo install --locked --path cli` into the suite-shared
    cache under a file lock, once per host. Refuses a dirty tracked
    checkout so the recorded HEAD sha is the real identity."""
    return str(toolchain.provision_water_cli())


def cli_evidence() -> dict:
    """Identity of the `water` binary the build invoked: path, sha256,
    --version output, plus the checkout HEAD it was built from."""
    p = Path(water_bin())
    r = subprocess.run([str(p), "--version"], capture_output=True,
                       text=True, timeout=30)
    if r.returncode != 0:
        raise RuntimeError(f"{p} --version failed rc={r.returncode}: "
                           f"{(r.stderr or r.stdout).strip()[-300:]}")
    return {"binary": str(p),
            "sha256": toolchain.sha256_file(p),
            "version": r.stdout.strip(),
            "checkout_head": toolchain.checkout_head(),
            "source": "in-tree cli/ (workspace member), cargo install --locked"}


def contestant(cid):
    for c in MANIFEST["contestants"]:
        if c["id"] == cid:
            return c
    raise KeyError(cid)


def du_bytes(path: Path) -> int:
    total = 0
    for p in path.rglob("*"):
        if p.is_file() and not p.is_symlink():
            total += p.lstat().st_size
    return total


def dir_sha256(path: Path) -> str:
    """Content hash of an app bundle: sorted (relpath, size, sha256)
    triples — the staging manifest records it so the device host measures
    exactly the artifact the build produced."""
    h = hashlib.sha256()
    for p in sorted(path.rglob("*")):
        if p.is_dir():
            continue
        rel = p.relative_to(path).as_posix()
        if p.is_symlink():
            h.update(f"{rel} -> {os.readlink(p)}\n".encode())
            continue
        fh = hashlib.sha256(p.read_bytes()).hexdigest()
        h.update(f"{rel} {p.lstat().st_size} {fh}\n".encode())
    return h.hexdigest()


def app_info(app: Path) -> dict:
    """The bundle's Info.plist (iOS layout: at the bundle root)."""
    return plistlib.loads((app / "Info.plist").read_bytes())


def bundle_executable(app: Path) -> str:
    """CFBundleExecutable — it need not equal the bundle stem (Flutter's
    is Runner, WaterUI's is its product name)."""
    exe = app_info(app).get("CFBundleExecutable")
    if not exe:
        raise RuntimeError(f"{app}/Info.plist has no CFBundleExecutable")
    return exe


def payload_zip_bytes(app: Path, work: Path) -> int:
    """Bytes of `Payload/<app>` deflated into a zip — the unsigned
    .ipa a store submission would carry, before thinning."""
    out = work / f"{app.stem}.unsigned.ipa"
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for p in sorted(app.rglob("*")):
            arc = Path("Payload") / app.name / p.relative_to(app)
            if p.is_symlink():
                info = zipfile.ZipInfo(str(arc))
                info.external_attr = 0o120777 << 16
                z.writestr(info, os.readlink(p))
            elif p.is_file():
                z.write(p, str(arc))
    size = out.stat().st_size
    out.unlink()
    return size


# ---------------------------------------------------------------- build

def cmd_bootstrap(args):
    """Fresh-clone dependency install on the build host, pinned to the
    committed lockfiles — the RN template, npm ci (package-lock.json),
    bundle install + pod install (Gemfile.lock / Podfile.lock), xcodegen
    generation of the native project. Any failure aborts; there is no
    silent retry.

    The whole bootstrap runs on a clean tracked tree and must leave it
    clean: a step that rewrites a committed lockfile or project means the
    committed copy is stale, and the staged set would otherwise carry a
    HEAD label the tree no longer matches."""
    t = MANIFEST["toolchain"]["build_host"]
    guarded = [ROOT, *{(ROOT / c["dir"]).resolve()
                       for c in MANIFEST["contestants"]}]
    toolchain.require_version("xcodegen", ["xcodegen", "--version"],
                              t["xcodegen"])
    toolchain.require_version("node", ["node", "--version"], t["node"])
    with toolchain.tracked_tree_unchanged("apple bootstrap", guarded):
        # RN root template files are generated, not committed —
        # materialize them from the pinned init before `npm ci`/`bundle`
        # reads the dir.
        toolchain.ensure_rn_template(
            ROOT / contestant("rn")["dir"], t["react_native_cli"],
            t["react_native"], t["react_native_template_sha256"],
            env=os.environ.copy())
        cp_ver = re.match(r"[\d.]+", t["cocoapods"])
        if not cp_ver:
            raise SystemExit("toolchain.build_host.cocoapods in "
                             "manifest.json does not start with a version")
        for step in MANIFEST["bootstrap"]["steps"]:
            sh(step.replace("{COCOAPODS}", cp_ver.group(0)))
    print("bootstrap complete")


def flutter_bin():
    """Resolved flutter binary for this host, version-checked against the
    manifest's declared toolchain."""
    t = MANIFEST["toolchain"]["build_host"]
    b = Path(os.path.expandvars(t["flutter_root"])) / "bin" / "flutter"
    toolchain.require_version("flutter", [str(b), "--version"], t["flutter"])
    return str(b)


def ensure_flutter_ios(d: Path):
    """The flutter app's ios/ directory is generated, not committed:
    produced by the pinned Flutter SDK's `flutter create` in a scratch
    dir, the authored ios-override files then replace the template
    sources. The generator's bundle id (dev.bench.benchFlutter) is left
    as built — the device host signs every contestant as the one
    harness.contestant_bundle_id. Reuses a generated dir only if
    .bench-generator records the same Flutter version — otherwise it is
    regenerated."""
    fb = flutter_bin()
    ver = subprocess.run([fb, "--version", "--machine"],
                         capture_output=True, text=True)
    try:
        tag = json.loads(ver.stdout)["frameworkVersion"]
    except (json.JSONDecodeError, KeyError) as e:
        raise RuntimeError(
            f"flutter --version --machine unreadable: "
            f"{(ver.stdout or ver.stderr)[:200]}") from e
    dst = d / "ios"
    stamp = dst / ".bench-generator"
    if dst.is_dir() and stamp.exists() and stamp.read_text().strip() == tag:
        return
    shutil.rmtree(dst, ignore_errors=True)
    with tempfile.TemporaryDirectory() as td:
        sh(f"'{fb}' create --platforms=ios "
           f"--project-name bench_flutter --org dev.bench "
           f"--template app '{td}/app'", cwd=td)
        shutil.copytree(Path(td) / "app" / "ios", dst)
    ovr = d / "ios-override"
    for f in ovr.rglob("*"):
        if f.is_file():
            rel = f.relative_to(ovr)
            (dst / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(f, dst / rel)
    stamp.write_text(tag + "\n")


def check_rpath_deps(app: Path) -> None:
    """Every @rpath dependency of the main executable must resolve inside
    the bundle's Frameworks/ — an unresolved one is a build failure,
    never a warning that ships a dead binary."""
    out = _out(["otool", "-L", str(app / bundle_executable(app))])
    for line in out.splitlines()[1:]:
        name = line.strip().split(" ")[0]
        if name.startswith("@rpath/"):
            lib = name.removeprefix("@rpath/")
            if not (app / "Frameworks" / lib).exists():
                raise RuntimeError(
                    f"unresolved @rpath dependency {name}: the artifact "
                    "does not carry it under Frameworks/")


def build_host_toolchain() -> dict:
    """Versions the build host's tools report — evidence beside the
    manifest's declared requirements (which bootstrap/build enforce)."""
    out = {}
    for name, cmd in {"flutter": [flutter_bin(), "--version"],
                      "node": ["node", "--version"],
                      "cocoapods": ["pod", "--version"],
                      "xcodegen": ["xcodegen", "--version"],
                      "rustc": ["rustc", "--version"]}.items():
        out[name] = _out(cmd, timeout=120).splitlines()[0]
    return out


def cmd_build(args):
    """Build every contestant for the iphoneos SDK, unsigned, on this
    (build) host and stage them with a staging manifest: per artifact its
    built bundle id, sha256, unsigned sizes; the build host's identity,
    Xcode and toolchain; the checkout HEAD and water CLI. The staged set
    is packed into build/ios-stage-<head12>.tar.gz for transfer to the
    device host. A failing contestant fails the build — a partial staged
    set is never packed."""
    cmd_bootstrap(args)
    head = toolchain.checkout_head()
    xcode = xcode_identity()
    shutil.rmtree(STAGE_DIR, ignore_errors=True)
    STAGE_DIR.mkdir(parents=True)
    failures = {}
    cli = None
    for c in MANIFEST["contestants"]:
        with toolchain.tracked_tree_unchanged(
                f"apple build {c['id']}",
                [ROOT, (ROOT / c["dir"]).resolve()]):
            if c["id"] == "flutter":
                ensure_flutter_ios(ROOT / c["dir"])
            for cmd in c["build"]:
                if "{WATER}" in cmd:
                    if cli is None:
                        cli = cli_evidence()
                    cmd = cmd.replace("{WATER}", cli["binary"])
                cmd = cmd.replace("{FLUTTER}", flutter_bin())
                try:
                    sh(cmd)
                except RuntimeError as e:
                    failures[c["id"]] = str(e)[-2000:]
                    break
        if c["id"] in failures:
            continue
        src = ROOT / c["artifact"]
        if not (src / "Info.plist").is_file():
            failures[c["id"]] = f"build ok but artifact missing: {src}"
            continue
        dst = STAGE_DIR / src.name
        shutil.copytree(src, dst, symlinks=True)
        if not (dst / bundle_executable(dst)).is_file():
            failures[c["id"]] = (f"staged {src.name} carries no executable "
                                 "— the build left an empty stub")
            continue
        try:
            check_rpath_deps(dst)
        except RuntimeError as e:
            failures[c["id"]] = str(e)
            continue
        print(f"staged {c['id']}: {dst} ({du_bytes(dst) / 1e6:.1f} MB)")
    if failures:
        print(json.dumps({"build_failures": failures}, indent=2))
        raise SystemExit(2)
    if cli is None:
        raise SystemExit("no contestant build invoked the water CLI — "
                         "the WaterUI contestant's build command lost "
                         "{WATER}")
    artifacts = {}
    with tempfile.TemporaryDirectory() as td:
        for c in MANIFEST["contestants"]:
            app = STAGE_DIR / Path(c["artifact"]).name
            info = app_info(app)
            artifacts[c["id"]] = {
                "app": app.name,
                "built_bundle_id": info["CFBundleIdentifier"],
                "executable": info["CFBundleExecutable"],
                "sha256": dir_sha256(app),
                "app_bytes": du_bytes(app),
                "unsigned_ipa_bytes": payload_zip_bytes(app, Path(td)),
            }
    staging = {"checkout_head": head,
               "built_utc": datetime.datetime.now(
                   datetime.timezone.utc).isoformat(timespec="seconds"),
               "build_host": host_evidence(),
               "xcode": xcode,
               "water_cli": cli,
               "toolchain": build_host_toolchain(),
               "artifacts": artifacts}
    (STAGE_DIR / "staging-manifest.json").write_text(
        json.dumps(staging, indent=1) + "\n")
    tar = ROOT / "build" / f"ios-stage-{head[:12]}.tar.gz"
    with tarfile.open(tar, "w:gz") as t:
        t.add(STAGE_DIR, arcname="stage")
    print(f"staged set → {tar}\nsha256 {toolchain.sha256_file(tar)}")


# ------------------------------------------------------- xctestrun surgery

def write_xctestrun(template: Path, out: Path, target_key: str,
                    products_subdir: str, app_name: str, bundle_id: str,
                    contestant: str, workload: str, drive: str,
                    duration: int, nonce: int, only_test: str | None = None,
                    step: int | None = None):
    """The runner's .xctestrun for one invocation. `bundle_id` is the id
    the app is installed under (the shared contestant id, which
    XCUIApplication launches); `contestant` is the stage entry being
    measured — the runner waits for that contestant's own ready post, so
    an installed app that is not the stage entry never satisfies it."""
    d = plistlib.loads(template.read_bytes())
    t = d[target_key]
    app_rel = f"__TESTROOT__/{products_subdir}/{app_name}"
    t["UITargetAppPath"] = app_rel
    deps = [p for p in t.get("DependentProductPaths", [])
            if not p.endswith(".app") or "Runner" in p]
    deps.append(app_rel)
    t["DependentProductPaths"] = deps
    if only_test:
        # one test per xcodebuild invocation — a shared invocation runs
        # the class's whole test list and a poll-attaching recorder
        # would bind to whichever test's app instance appeared first
        t["OnlyTestIdentifiers"] = [f"BenchTests/{only_test}"]
    env = t.setdefault("EnvironmentVariables", {})
    h = MANIFEST["harness"]
    if h["warmup_ms"] <= 0:
        raise RuntimeError(
            "harness.warmup_ms must be > 0 — the warmup is declared in "
            "the manifest and is never zero (METHOD)")
    if h["anchor_tolerance_ms"] <= 0:
        raise RuntimeError(
            "harness.anchor_tolerance_ms must be > 0 — the runner holds "
            "the contestant this long past the capture so the trace's "
            "window ends inside the held span")
    f = h["fling"]
    env.update({
        "BENCH_BUNDLE_ID": bundle_id,
        "BENCH_CONTESTANT": contestant,
        "BENCH_WORKLOAD": workload,
        "BENCH_DRIVE": drive,
        "BENCH_DURATION": str(duration),
        "BENCH_WARMUP_MS": str(h["warmup_ms"]),
        "BENCH_ANCHOR_TOLERANCE_MS": str(h["anchor_tolerance_ms"]),
        # the recorder-go latch this invocation must match — a latched
        # signal left by another invocation never releases this one
        "BENCH_RUN_NONCE": str(nonce),
        # the fling program the XCTest's swipe branch executes — from the
        # manifest, never Swift literals
        "BENCH_FLING": json.dumps({
            "flings_down": int(f["down"]),
            "flings_up": int(f["up"]),
            "start_fraction": float(f["start_fraction"]),
            "end_fraction": float(f["end_fraction"]),
            "duration_ms": float(f["duration_ms"]),
            "pause_s": float(f["pause_ms"]) / 1000.0,
            "hold_s": float(f["hold_s"]),
        }),
    })
    if step is not None:
        env["BENCH_STEP"] = str(step)
    out.write_bytes(plistlib.dumps(d))


def _metrics_record(res: Path) -> dict:
    """xcresult -> run record. A parse failure is a run error whose raw
    stderr is kept verbatim on the row (and lands in the report's
    errors table) rather than crashing flatten/report later."""
    metrics = parse_xcresult(res)
    if "_error" in metrics:
        return {"error": "xcresult metrics unreadable",
                "xcresult_error": metrics["_error"]}
    return {"metrics": metrics}


def parse_xcresult(path: Path) -> dict:
    """testIdentifier -> metric tail -> {unit, measurements}."""
    r = subprocess.run(
        ["xcrun", "xcresulttool", "get", "test-results", "metrics",
         "--path", str(path)],
        capture_output=True, text=True)
    if r.returncode != 0:
        return {"_error": r.stderr[-500:]}
    out = {}
    for t in json.loads(r.stdout):
        tid = t["testIdentifier"].split("/")[-1].rstrip("()")
        for run in t.get("testRuns", []):
            for m in run.get("metrics", []):
                ident = m["identifier"]
                tail = ident.split(".")[-1]
                out.setdefault(tid, {})[tail] = {
                    "unit": m["unitOfMeasurement"],
                    "measurements": m["measurements"],
                    "identifier": ident,
                }
    return out


def drive_for(workload: str) -> str:
    """The drive of a workload, identical for every contestant: the
    manifest's `workloads.<w>.drive` (swipe | tap | none)."""
    return MANIFEST["workloads"][workload]["drive"]


def sanitize_runs(state: dict) -> None:
    """Drop rows from superseded harness generations: rows without a
    `drive`, of an unknown contestant/workload, or whose drive disagrees
    with the manifest's current one. Error rows are evidence and stay."""
    ids = {c["id"] for c in MANIFEST["contestants"]}
    kept = []
    for x in state.get("runs", []):
        if "error" in x:
            kept.append(x)
            continue
        w = x.get("workload")
        if (x.get("contestant") in ids and w in MANIFEST["workloads"]
                and x.get("drive") == drive_for(w)):
            kept.append(x)
    state["runs"] = kept


def capacity_workload(w: str) -> bool:
    """W5/W6: the manifest declares a ladder (`steps`) for them."""
    return "steps" in MANIFEST["workloads"][w]


def _xctrace(args_, timeout=None):
    return subprocess.run(["xcrun", "xctrace", *args_],
                          capture_output=True, text=True, timeout=timeout)


# ---------------------------------------------------- trace attribution
#
# The Animation Hitches template records through the Hitches tap
# (Instruments.app/Contents/PlugIns/HitchesTap.framework, whose plug-in
# data declares the tables hitches, hitches-frame-lifetimes,
# hitches-renders, hitches-updates, hitches-gpu, hitches-framewait).
# Frame lifetimes are display-level: they carry the swap they presented
# and the display, never a process. Client updates (Core Animation
# commits) carry the committing process and the swap they landed in. A
# frame is the contestant's own when one of the swaps it presented
# composited an update from a process the contestant owns. Every value
# below is a trace-relative nanosecond timestamp (the tap's start-time /
# event-time columns), so the window, the frames and the runner's marks
# live on one clock and nothing is mapped through a guessed epoch.
#
# The tables are read by column mnemonic (column mnemonic: engineering
# type below); a column attribution needs that the device recording of
# the device host's Xcode does not carry fails it (_require_cols):
#   hitches-frame-lifetimes  start:start-time duration:duration
#       display:display-name swap-id:uint32 surface-id:uint32
#       frame-color layout-qualifier label
#   hitches-updates  start:start-time duration:duration process:process
#       display:display-name swap-id:uint32 surface-id:uint32
#       frame-color containment-level:uint32 label
#   os-signpost  time:event-time thread process event-type scope
#       identifier name:signpost-name format-string backtrace
#       subsystem:subsystem category:category message emit-location
#   os-log  (process, message — the WaterUI first-paint marker)
# The frames recording (Animation Hitches + Points of Interest + os_log)
# holds two os-signpost tables (categories InduceCondition and
# PointsOfInterest); the runner's marks live in the PointsOfInterest
# one, selected by its TOC attribute.

FRAMES_SCHEMA = "hitches-frame-lifetimes"
UPDATES_SCHEMA = "hitches-updates"
SIGNPOST_SCHEMA = "os-signpost"
SIGNPOST_TABLE = (("category", "PointsOfInterest"),)
FRAME_COLS = ("start", "duration", "swap-id", "display")
UPDATE_COLS = ("process", "swap-id", "display")
SIGNPOST_COLS = ("time", "name", "subsystem")
# The runner's own marks, emitted as Points of Interest events: the
# drive starts at drive-begin; measure-end closes the held span.
MARK_SUBSYSTEM = "dev.bench"
MARK_DRIVE_BEGIN = "drive-begin"
MARK_MEASURE_END = "measure-end"
# The one recording a launch makes: Animation Hitches plus Points of
# Interest (the runner's marks share the frames' trace clock) plus os_log
# (the WaterUI first-paint marker). Every contestant is recorded with the
# same instruments, so each pays the same recording cost.
FRAMES_TEMPLATE = "Animation Hitches"
FRAMES_INSTRUMENTS = ("Points of Interest", "os_log")
LOG_SCHEMA = "os-log"


class TraceAttributionError(RuntimeError):
    """The trace cannot say which frames are the contestant's, or the
    window the METHOD defines is not covered by what the runner held."""


def _export_table(trace: Path, schema: str,
                  where: tuple[tuple[str, str], ...] = ()):
    """`xctrace export` rows of the ONE table with `schema` (and the
    TOC attributes in `where`, e.g. the os-signpost table of the Points
    of Interest category) → (rows, err).

    Row children carry the column's ENGINEERING TYPE as their tag and
    appear in schema column order (an empty cell is a `<sentinel/>`), so
    each row is zipped against the `<schema><col><mnemonic>` list of
    its node. Elements either define a value (`id` attr, raw text or
    `fmt`) or repeat one (`ref` attr pointing at an earlier `id` of the
    same tag) — refs are resolved so every row is self-contained."""
    pred = "".join(f'[@{k}="{v}"]' for k, v in where)
    r = _xctrace(["export", "--input", str(trace), "--xpath",
                  f'/trace-toc/run[@number="1"]/data/table'
                  f'[@schema="{schema}"]{pred}'],
                 timeout=600)
    if r.returncode != 0:
        return None, (r.stderr or "")[-400:]
    return parse_export_rows(r.stdout, schema)


def parse_export_rows(xml_text: str, schema: str):
    """Pure parse of an `xctrace export` table document → (rows, err).

    The rows come from exactly one `<node>` that carries a `<schema>`
    named `schema`. xctrace also emits auxiliary nodes without a schema
    (Xcode 26.6: the os-signpost export of an Animation Hitches trace
    holds two nodes, the second schema-less) — they hold no rows of the
    table and are not selected. Zero or several schema nodes mean the
    export did not name one table, which fails."""
    import xml.etree.ElementTree as ET
    root = ET.fromstring(xml_text)
    nodes = root.findall(".//node[schema]")
    if len(nodes) != 1:
        return None, (f"export carries {len(nodes)} <node>s with a "
                      f"<schema>; expected exactly one {schema} table")
    node = nodes[0]
    sch = node.find("schema")
    if sch.get("name") != schema:
        return None, (f"export node's schema is {sch.get('name')!r}, "
                      f"not {schema!r}")
    idmap = {}
    for e in root.iter():
        if e.get("id") is not None:
            v = (e.text or "").strip() or (e.get("fmt") or "")
            idmap.setdefault(e.tag, {})[e.get("id")] = v

    def val(e):
        if e.tag == "sentinel":
            return None
        if e.get("ref") is not None:
            return idmap.get(e.tag, {}).get(e.get("ref"))
        t = (e.text or "").strip()
        return t if t else (e.get("fmt") or "")
    cols = [c.findtext("mnemonic") for c in sch.findall("col")]
    rows = []
    for row in node.findall("row"):
        cells = list(row)
        if len(cells) != len(cols):
            return None, (f"row has {len(cells)} cells for "
                          f"{len(cols)} schema columns")
        rows.append({c: val(e) for c, e in zip(cols, cells)})
    return rows, None


def parse_toc_processes(xml_text: str) -> list[dict]:
    """`xctrace export --toc` → [{name, pid, path}] of run 1's process
    list. A process entry without a path cannot be tied to a bundle and
    fails the parse rather than being skipped."""
    import xml.etree.ElementTree as ET
    root = ET.fromstring(xml_text)
    run = root.find("run[@number='1']")
    if run is None:
        raise TraceAttributionError("trace TOC has no run 1")
    procs = run.findall("processes/process")
    if not procs:
        raise TraceAttributionError("trace TOC lists no processes")
    out = []
    for p in procs:
        pid = p.get("pid")
        if pid is None or not pid.lstrip("-").isdigit():
            raise TraceAttributionError(
                f"TOC process without a pid: {p.attrib}")
        out.append({"name": p.get("name"), "pid": int(pid),
                    "path": p.get("path")})
    return out


def _trace_toc(trace: Path) -> list[dict]:
    r = _xctrace(["export", "--input", str(trace), "--toc"], timeout=600)
    if r.returncode != 0:
        raise TraceAttributionError(
            f"xctrace export --toc failed: {(r.stderr or '')[-400:]}")
    return parse_toc_processes(r.stdout)


def owned_processes(processes: list[dict], bundle_name: str,
                    main_rel: str) -> tuple[int, set[int]]:
    """(main pid, owned pids) of the contestant in one trace.

    Owned = every process whose executable lives inside the contestant's
    bundle (`…/<bundle_name>/…`): the app itself plus any helper
    executable it ships.
    Exactly one process may run the main executable — a second instance
    or none means the trace does not show one launch of this
    contestant."""
    seg = f"/{bundle_name}/"
    main_suffix = f"/{bundle_name}/{main_rel}"
    pathless = [p for p in processes if not p.get("path")
                and p["pid"] > 0]
    owned = {p["pid"] for p in processes
             if p.get("path") and seg in p["path"]}
    main = [p["pid"] for p in processes
            if p.get("path") and p["path"].endswith(main_suffix)]
    if len(main) != 1:
        raise TraceAttributionError(
            f"expected exactly one process running …{main_suffix} in the "
            f"trace, found {len(main)} ({main}); "
            f"{len(pathless)} TOC processes carry no path")
    return main[0], owned


def _pid_of_process(fmt: str | None) -> int | None:
    """Pid from an export `process` cell ("Name (pid)")."""
    m = re.search(r"\((\d+)\)\s*$", (fmt or "").strip())
    return int(m.group(1)) if m else None


def _require_cols(rows: list[dict], cols, schema: str):
    if rows and not set(cols) <= set(rows[0]):
        raise TraceAttributionError(
            f"{schema} lacks column(s) "
            f"{sorted(set(cols) - set(rows[0]))}; has {sorted(rows[0])}")


def _int(v, what: str) -> int:
    try:
        return int(str(v).strip())
    except (TypeError, ValueError):
        raise TraceAttributionError(f"{what}: not an integer: {v!r}") \
            from None


def _present_ns(f: dict) -> int | None:
    """Presentation instant of a frame lifetime (start + duration); an
    open lifetime (no duration) never presented."""
    if f["duration"] is None:
        return None
    return (_int(f["start"], f"{FRAMES_SCHEMA} start")
            + _int(f["duration"], f"{FRAMES_SCHEMA} duration"))


def _updates_by_swap(updates: list[dict]) -> tuple[dict, list[int]]:
    """({(display, swap id): [committing pid, …]}, [committing pid of
    every update without a swap id]) of the client updates. Swap ids are
    keyed with their display: the tap does not declare them unique
    across displays, and both tables carry the display. An update
    without a swap joins no frame; it is returned, never dropped
    silently, so the attribution evidence counts it."""
    out: dict = {}
    unswapped: list[int] = []
    for u in updates:
        pid = _pid_of_process(u["process"])
        if pid is None:
            raise TraceAttributionError(
                f"{UPDATES_SCHEMA} process cell without a pid: "
                f"{u['process']!r}")
        if u["swap-id"] is None:
            unswapped.append(pid)
            continue
        key = (u["display"], _int(u["swap-id"], f"{UPDATES_SCHEMA} swap-id"))
        out.setdefault(key, []).append(pid)
    return out, unswapped


def owned_presents(frames: list[dict], updates: list[dict],
                   owned: set[int]) -> dict:
    """Join frame lifetimes to the contestant's client updates by
    (display, swap).

    Returns {presents_ns (sorted), display, owned_swaps, frames_total,
    updates_owned, updates_without_swap, owned_updates_without_swap}:
    the last two count the client updates (all, and the contestant's)
    that carry no swap id and so cannot join any frame. Raises when the
    trace recorded no frame at all, when nothing joins, or when the
    owned frames span more than one display (the frame statistics take
    one refresh period)."""
    if not frames:
        raise TraceAttributionError(
            f"{FRAMES_SCHEMA} has 0 rows: the Hitches tap recorded no "
            "frame on any display")
    _require_cols(frames, FRAME_COLS, FRAMES_SCHEMA)
    _require_cols(updates, UPDATE_COLS, UPDATES_SCHEMA)
    by_swap, unswapped = _updates_by_swap(updates)
    swaps = {k for k, pids in by_swap.items() if owned & set(pids)}
    n_upd = sum(1 for pids in by_swap.values() for p in pids if p in owned)
    if not swaps:
        raise TraceAttributionError(
            f"no {UPDATES_SCHEMA} row from owned pids {sorted(owned)} "
            f"carries a swap ({len(updates)} updates in the trace)")
    presents, displays = [], set()
    for f in frames:
        t = _present_ns(f)
        if t is None or f["swap-id"] is None:
            continue
        key = (f["display"], _int(f["swap-id"], f"{FRAMES_SCHEMA} swap-id"))
        if key not in swaps:
            continue
        presents.append(t)
        displays.add(f["display"])
    if not presents:
        raise TraceAttributionError(
            f"no {FRAMES_SCHEMA} row presented a swap carrying an owned "
            f"update ({len(swaps)} owned swaps, {len(frames)} frames)")
    if len(displays) != 1:
        raise TraceAttributionError(
            f"owned frames span displays {sorted(map(str, displays))} — "
            "the measured contestant must present on one display")
    return {"presents_ns": sorted(presents), "display": displays.pop(),
            "owned_swaps": len(swaps), "frames_total": len(frames),
            "updates_owned": n_upd,
            "updates_without_swap": len(unswapped),
            "owned_updates_without_swap": sum(
                1 for p in unswapped if p in owned)}


def classify_display_frames(frames: list[dict], updates: list[dict],
                            owned: set[int], display: str,
                            lo_ns: int, hi_ns: int) -> dict:
    """Evidence for the attribution rule (not the rule itself; every rep
    stores it and `require_client_updates` gates on it): every
    frame presented on `display` within [lo_ns, hi_ns], classified by
    the client updates its swap carried — owned only, owned and
    foreign, foreign only, or none (a frame the render server produced
    without any client commit, e.g. a Core Animation animation it
    interpolates). Foreign updaters are counted per process cell, by
    the number of those frames they put an update in. Frames without a
    client update are also counted when their surface-id is one an
    owned update landed on — whether surface ids can attribute
    render-server frames is part of what this evidence settles."""
    by_swap, _ = _updates_by_swap(updates)
    owned_surfaces = {(u["display"], u.get("surface-id")) for u in updates
                      if _pid_of_process(u["process"]) in owned
                      and u.get("surface-id") is not None}
    counts = {"owned": 0, "owned+foreign": 0, "foreign": 0,
              "no-client-update": 0,
              "no-client-update-on-owned-surface": 0}
    foreign_pids: dict[int, int] = {}
    for f in frames:
        t = _present_ns(f)
        if (t is None or f["display"] != display or f["swap-id"] is None
                or not lo_ns <= t <= hi_ns):
            continue
        pids = set(by_swap.get(
            (display, _int(f["swap-id"], f"{FRAMES_SCHEMA} swap-id")), []))
        own, foreign = pids & owned, pids - owned
        if own and foreign:
            counts["owned+foreign"] += 1
        elif own:
            counts["owned"] += 1
        elif foreign:
            counts["foreign"] += 1
        else:
            counts["no-client-update"] += 1
            if (display, f.get("surface-id")) in owned_surfaces:
                counts["no-client-update-on-owned-surface"] += 1
        for pid in foreign:
            foreign_pids[pid] = foreign_pids.get(pid, 0) + 1
    names = {}
    for u in updates:
        pid = _pid_of_process(u["process"])
        if pid in foreign_pids:
            names[pid] = u["process"]
    return {"display": display, "span_ns": [lo_ns, hi_ns], **counts,
            "foreign_updaters": sorted(
                ({"process": names[p], "frames": n}
                 for p, n in foreign_pids.items()),
                key=lambda d: -d["frames"])}


def require_client_updates(classified: dict) -> None:
    """The gate that holds while the attribution rule for render-server
    frames is undecided. The owned-update join counts a frame only when
    its swap carried a client update from the contestant; a frame the
    render server presented without any client commit (a Core Animation
    animation it interpolates, as UIKit's UIView.animate and SwiftUI
    animations may be) joins nothing, so a window holding one would
    under-count that contestant's frames. Such a rep fails until the
    rule is decided; its classification is on the row as evidence."""
    n = classified["no-client-update"]
    if n:
        raise TraceAttributionError(
            f"{n} frame(s) on display {classified['display']} inside the "
            "window carried no client update (of them "
            f"{classified['no-client-update-on-owned-surface']} on a "
            "surface an owned update landed on): the attribution rule for "
            "render-server frames is undecided, so the owned-update join "
            "cannot count this window")


def runner_marks(signposts: list[dict]) -> dict:
    """{mark name: trace ns} of the runner's Points of Interest events —
    each mark exactly once per trace."""
    _require_cols(signposts, SIGNPOST_COLS, SIGNPOST_SCHEMA)
    found = {}
    for s in signposts:
        if s["subsystem"] != MARK_SUBSYSTEM:
            continue
        found.setdefault(s["name"], []).append(
            _int(s["time"], f"{SIGNPOST_SCHEMA} time"))
    out = {}
    for name in (MARK_DRIVE_BEGIN, MARK_MEASURE_END):
        ts = found.get(name, [])
        if len(ts) != 1:
            raise TraceAttributionError(
                f"expected one {MARK_SUBSYSTEM} '{name}' signpost in the "
                f"trace, found {len(ts)}")
        out[name] = ts[0]
    return out


def measurement_window(presents_ns: list[int], marks: dict, *,
                       warmup_ms: float, capture_ms: float,
                       tolerance_ms: float, driven: bool) -> dict:
    """METHOD's window on the trace clock: [first owned present +
    warmup, + capture] — readiness is the contestant's first owned
    present, read from the trace. The runner must have held the
    contestant through the window end. A driven cell (tap, swipe)
    starts its drive live at the contestant's ready post +
    warmup — the trace is only readable after the recording stops — so
    that start must sit within `tolerance_ms` of the window start
    (METHOD: the drive starts at window start). An undriven cell (W3,
    W5) has no drive to align: its drive-begin mark only opens the held
    span, and its offset is reported, not gated.
    Returns milliseconds on the trace clock."""
    first = presents_ns[0] / 1e6
    w0 = first + warmup_ms
    w1 = w0 + capture_ms
    drive = marks[MARK_DRIVE_BEGIN] / 1e6
    held = marks[MARK_MEASURE_END] / 1e6
    offset = drive - w0
    if driven and abs(offset) > tolerance_ms:
        raise TraceAttributionError(
            f"drive started {offset:+.1f} ms from the window start "
            f"(first owned present {first:.1f} ms + warmup {warmup_ms:g} "
            f"ms); tolerance is {tolerance_ms:g} ms")
    if held < w1:
        raise TraceAttributionError(
            f"runner released the contestant at {held:.1f} ms, before "
            f"the window end {w1:.1f} ms")
    return {"first_owned_present_ms": first, "window_start_ms": w0,
            "window_end_ms": w1, "drive_begin_ms": drive,
            "drive_offset_ms": round(offset, 3), "measure_end_ms": held}


def trace_tables(trace: Path) -> dict:
    """{schema: rows} of the three tables attribution reads."""
    tables = {}
    for schema, where in ((FRAMES_SCHEMA, ()), (UPDATES_SCHEMA, ()),
                          (SIGNPOST_SCHEMA, SIGNPOST_TABLE)):
        rows, err = _export_table(trace, schema, where)
        if rows is None:
            raise TraceAttributionError(f"{schema}: {err}")
        tables[schema] = rows
    return tables


def trace_attribution(trace: Path, bundle_name: str, main_rel: str,
                      tables: dict | None = None) -> dict:
    """Read one all-process trace: owned pids from its TOC, owned
    presents by the (display, swap) join, the runner marks. No window
    gating — the `attribution` command prints this as evidence."""
    main_pid, owned = owned_processes(_trace_toc(trace), bundle_name,
                                      main_rel)
    if tables is None:
        tables = trace_tables(trace)
    joined = owned_presents(tables[FRAMES_SCHEMA], tables[UPDATES_SCHEMA],
                            owned)
    return {"main_pid": main_pid, "owned_pids": sorted(owned),
            "signposts": tables[SIGNPOST_SCHEMA], **joined}


def trace_frame_window(trace: Path, bundle_name: str, main_rel: str,
                       capture_ms: float, refresh_ms: float,
                       driven: bool) -> dict:
    """Frame statistics of the contestant's own presents over METHOD's
    window, plus the window itself (trace ms), the attribution evidence
    and every frame on the contestant's display inside the window
    classified by the client updates its swap carried (`frames_by_update`,
    from the tables this function already exported). Raises
    TraceAttributionError when the attribution or the window is unsound;
    the classification is returned for the caller to store and gate
    (require_client_updates), so a gated rep still records it."""
    tables = trace_tables(trace)
    att = trace_attribution(trace, bundle_name, main_rel, tables)
    marks = runner_marks(att.pop("signposts"))
    h = MANIFEST["harness"]
    warmup_ms = float(h["warmup_ms"])
    presents_ns = att.pop("presents_ns")
    win = measurement_window(presents_ns, marks, warmup_ms=warmup_ms,
                             capture_ms=capture_ms,
                             tolerance_ms=float(h["anchor_tolerance_ms"]),
                             driven=driven)
    stats = lib_frames.frame_statistics(
        [t / 1e6 for t in presents_ns],
        window_start_ms=win["window_start_ms"], capture_ms=capture_ms,
        refresh_ms=refresh_ms)
    # the same window on the trace's integer ns clock
    lo_ns = presents_ns[0] + round(warmup_ms * 1e6)
    classified = classify_display_frames(
        tables[FRAMES_SCHEMA], tables[UPDATES_SCHEMA],
        set(att["owned_pids"]), att["display"],
        lo_ns, lo_ns + round(capture_ms * 1e6))
    return {"frame_stats": stats, "window": win, "attribution": att,
            "frames_by_update": classified}


def _proc_name(fmt: str | None):
    """Process name from an export `process` cell ("Name (pid)")."""
    m = re.match(r"^(.*) \((\d+)\)$", (fmt or "").strip())
    return m.group(1) if m else None


def _pid_of_thread(fmt: str) -> int | None:
    """Owning pid from an export `thread` cell ("… (name, pid: N)")."""
    m = re.findall(r"\(([^()]*), pid: (\d+)\)", fmt or "")
    return int(m[-1][1]) if m else None


_PAINT_RE = re.compile(r"waterui_first_paint_ms=\s*([0-9]+(?:\.[0-9]+)?)")


def first_paint_ms(trace: Path, proc: str) -> float:
    """The `waterui_first_paint_ms=N` the WaterUI contestant emitted
    (os_log `dev.waterui`, notice level), read from the os-log table of
    the cell's one all-process recording (its os_log instrument) —
    devicectl exposes no console read. Rows are selected by `proc` (the
    contestant's executable name). An export error or an absent marker
    raises: the apple backend emits the marker once per process, so its
    absence is a defect, not a gap."""
    rows, err = _export_table(trace, LOG_SCHEMA)
    if rows is None:
        raise TraceAttributionError(f"{LOG_SCHEMA} export: {err}")
    for row in rows:
        if _proc_name(row.get("process")) != proc:
            continue
        m = _PAINT_RE.search(str(row.get("message") or ""))
        if m:
            return float(m.group(1))
    raise TraceAttributionError(
        f"no waterui_first_paint_ms marker from {proc} in the recording "
        f"({len(rows)} {LOG_SCHEMA} rows)")


def _spawn_pty(cmd: list[str], env: dict | None = None):
    """Spawn `cmd` session-led and tracked, its stdout+stderr on a
    pseudo-terminal: a tty line-buffers the child's stdio, so a status
    line it prints is readable the moment it is printed rather than when
    the process exits. Returns (proc, line reader)."""
    import pty
    master, slave = pty.openpty()
    try:
        p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=slave,
                             stderr=slave, start_new_session=True, env=env)
    finally:
        os.close(slave)
    _ACTIVE_PROCS.append(p)
    return p, PtyLines(master)


class PtyLines:
    """Lines from a pty master; each read blocks in select(2) on the
    descriptor, bounded by the caller's deadline (time.monotonic())."""

    def __init__(self, fd: int):
        self.fd = fd
        self.buf = b""
        self.eof = False

    def readline(self, deadline: float | None) -> str | None:
        """The next line, None at EOF; TimeoutError past the deadline."""
        import errno
        import select
        while b"\n" not in self.buf and not self.eof:
            wait = None if deadline is None else deadline - time.monotonic()
            if wait is not None and wait <= 0:
                raise TimeoutError
            ready, _, _ = select.select([self.fd], [], [], wait)
            if not ready:
                raise TimeoutError
            try:
                chunk = os.read(self.fd, 4096)
            except OSError as e:
                # macOS reports the slave side closing (child exited)
                # as EIO on the master
                if e.errno != errno.EIO:
                    raise
                chunk = b""
            if not chunk:
                self.eof = True
            self.buf += chunk
        if not self.buf:
            return None
        line, _, self.buf = self.buf.partition(b"\n")
        return line.decode(errors="replace").rstrip("\r")

    def close(self):
        os.close(self.fd)

class InstrumentsScratch:
    """The raw kernel-trace scratch files one cell's recording leaves.

    A recording writes its raw kdebug stream to an `instruments*.ktrace`
    file and converts it into the .trace at stop. The file stays in the
    user's Darwin temp dir after the recording ended (gigabytes for a W3
    recording), held open by the idle DTServiceHub agent that recorded
    it, so neither the .trace output nor the recorder process owns it.
    The cell owns it: the recorder runs with TMPDIR set to a scratch dir
    of its own (`scratch_dir`, removed whole by `sweep`), and every
    `instruments*.ktrace` that appeared in the user temp dir or a scratch
    dir between `__init__` (before the cell's recorder spawns) and
    `sweep` (after it stopped) is the cell's.

    `sweep` asks lsof which processes hold each such file. Every file may
    be held by at most one process, that process must be DTServiceHub,
    and the whole cell's files may be held by at most one pid: that one
    pid is the only process the runner may terminate (SIGTERM, logged),
    so the space comes back before the next recording starts. A file held
    by several processes, by anything but DTServiceHub, or a second
    DTServiceHub pid fails the cell and terminates nothing; the files are
    unlinked either way (an unlinked file held open keeps its blocks until
    the holder exits — the failed cell says so)."""

    PATTERN = "instruments*.ktrace"
    HOLDER = "DTServiceHub"

    @staticmethod
    def user_temp_dir() -> Path:
        """The user's Darwin temp dir (confstr DARWIN_USER_TEMP_DIR), the
        agent's scratch location whatever TMPDIR the recorder sees."""
        return Path(_out(["getconf", "DARWIN_USER_TEMP_DIR"]))

    def __init__(self, root: Path, user_tmp: Path):
        self.user_tmp = user_tmp
        self.root = root
        self.dirs: list[Path] = []
        # what sweep removed: [{path, bytes, holders {pid: command}}]
        self.removed: list[dict] = []
        # the DTServiceHub pid sweep terminated, if any
        self.terminated: int | None = None
        self.before = set(self.user_tmp.glob(self.PATTERN))

    def scratch_dir(self, name: str) -> Path:
        d = self.root / f"{name}.xctrace-tmp"
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir(parents=True)
        self.dirs.append(d)
        return d

    @staticmethod
    def _holders(f: Path, deadline: float) -> dict[int, str]:
        """{pid: full command name} of the processes holding `f` open;
        lsof exits 1 with no output when nothing does. Bounded by the
        sweep's shared `deadline` (time.monotonic()); past it the sweep
        fails rather than waiting longer than LSOF_S declares."""
        left = deadline - time.monotonic()
        if left <= 0:
            raise RuntimeError(f"lsof {f}: the sweep's {LSOF_S}s lsof "
                               "budget is spent")
        r = subprocess.run(["lsof", "+c", "0", "-F", "pc", "--", str(f)],
                           capture_output=True, text=True, timeout=left)
        if r.returncode not in (0, 1):
            raise RuntimeError(f"lsof {f}: rc={r.returncode} "
                               f"{r.stderr.strip()!r}")
        out, pid = {}, None
        for line in r.stdout.splitlines():
            if line.startswith("p"):
                pid = int(line[1:])
            elif line.startswith("c") and pid is not None:
                out[pid] = line[1:]
        return out

    @staticmethod
    def _terminate(pid: int, bound_s: float) -> bool:
        """SIGTERM `pid` and wait for its exit as a kqueue NOTE_EXIT
        event (it is not our child, so waitpid cannot); True once it
        exited."""
        import select
        kq = select.kqueue()
        try:
            try:
                kq.control([select.kevent(
                    pid, filter=select.KQ_FILTER_PROC,
                    flags=select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                    fflags=select.KQ_NOTE_EXIT)], 0, 0)
            except ProcessLookupError:
                return True
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                return True
            return bool(kq.control(None, 1, bound_s))
        finally:
            kq.close()

    def sweep(self, bound_s: float = HUB_EXIT_S) -> str | None:
        """Remove the cell's scratch; an error string when a scratch
        file cannot be released. Takes at most LSOF_S + `bound_s`."""
        leaked = set(self.user_tmp.glob(self.PATTERN)) - self.before
        for d in self.dirs:
            leaked |= set(d.rglob(self.PATTERN))
        errs = []
        hub_pids = set()
        lsof_deadline = time.monotonic() + LSOF_S
        for f in sorted(leaked):
            held = self._holders(f, lsof_deadline)
            self.removed.append({"path": str(f), "bytes": f.stat().st_size,
                                 "holders": held})
            f.unlink()
            if len(held) > 1:
                errs.append(f"{f.name} is held open by {len(held)} "
                            f"processes {held}")
            for pid, cmd in held.items():
                if cmd == self.HOLDER:
                    hub_pids.add(pid)
                else:
                    errs.append(f"{f.name} is held open by {cmd} "
                                f"(pid {pid}), not {self.HOLDER}")
        if len(hub_pids) > 1:
            errs.append(f"the cell's scratch is held by {len(hub_pids)} "
                        f"{self.HOLDER} pids {sorted(hub_pids)}; the runner "
                        "terminates at most one")
        if not errs and hub_pids:
            pid = hub_pids.pop()
            print(f"instruments scratch: SIGTERM {self.HOLDER} pid {pid} "
                  "(lsof: holds the cell's removed ktrace)", flush=True)
            self.terminated = pid
            try:
                if not self._terminate(pid, bound_s):
                    errs.append(f"{self.HOLDER} (pid {pid}) still holds a "
                                f"removed ktrace {bound_s:.0f}s after "
                                "SIGTERM")
            except PermissionError:
                errs.append(f"{self.HOLDER} (pid {pid}) holds a removed "
                            "ktrace and is not this user's to terminate")
        for d in self.dirs:
            shutil.rmtree(d)
        self.dirs = []
        return "; ".join(errs) or None


class XctraceRecorder:
    """One `xctrace record --all-processes` session.

    Recording every process means nothing has to exist when the recorder
    arms: it is armed BEFORE the runner's recorder-go is released (so it
    covers the contestant from its launch), and the contestant's rows
    are selected at export by process. Arming is xctrace's own
    "recording started" line, read from its tty — no attach retry and no
    sleep standing in for readiness. xctrace runs with TMPDIR set to
    `scratch`, a dir of the cell's InstrumentsScratch."""

    ARMED = "Ctrl-C to stop the recording"

    def __init__(self, out: Path, template: str, device_udid: str | None,
                 time_limit_s: int, scratch: Path,
                 instruments: tuple[str, ...] = ()):
        cmd = ["xcrun", "xctrace", "record", "--template", template,
               "--all-processes", "--output", str(out),
               "--time-limit", f"{time_limit_s}s"]
        for name in instruments:
            cmd += ["--instrument", name]
        if device_udid:
            cmd += ["--device", device_udid]
        self.out = out
        self.template = template
        self.log: list[str] = []
        self.proc, self.lines = _spawn_pty(
            cmd, env={**os.environ, "TMPDIR": f"{scratch}/"})
        self._drain = None
        self._result = None

    def arm(self, bound_s: float = RECORDER_ARM_S):
        deadline = time.monotonic() + bound_s
        while True:
            try:
                line = self.lines.readline(deadline)
            except TimeoutError:
                self.stop()
                raise RuntimeError(
                    f"xctrace {self.template}: not recording after "
                    f"{bound_s:.0f}s: {self.log[-5:]}") from None
            if line is None:
                rc = self.proc.wait()
                self.stop()
                raise RuntimeError(
                    f"xctrace {self.template} exited rc={rc} before "
                    f"recording: {self.log[-5:]}")
            self.log.append(line)
            if self.ARMED in line:
                break
        # keep reading so a chatty recorder never blocks on a full pty
        self._drain = threading.Thread(target=self._drain_rest, daemon=True)
        self._drain.start()

    def _drain_rest(self):
        while (line := self.lines.readline(None)) is not None:
            self.log.append(line)

    def stop(self, bound_s: float = RECORDER_STOP_S) -> str | None:
        """SIGINT and wait for the trace to be written; an error string
        when it was not. Idempotent."""
        if self._result is not None:
            return self._result[0]
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGINT)
            try:
                self.proc.wait(timeout=bound_s)
            except subprocess.TimeoutExpired:
                _kill_proc(self.proc)
                self.proc.wait()
        if self.proc in _ACTIVE_PROCS:
            _ACTIVE_PROCS.remove(self.proc)
        if self._drain is not None:
            self._drain.join(timeout=RECORDER_DRAIN_S)
        self.lines.close()
        err = None
        if self.proc.returncode != 0 or not self.out.exists():
            err = (f"xctrace {self.template} rc={self.proc.returncode}: "
                   f"{self.log[-5:]}")
        self._result = (err,)
        return err

def release_recorder_go(udid: str, runner_bid: str, nonce: int,
                        work: Path) -> None:
    """Release the runner's recorder-go gate. The host cannot reach the
    device's notify namespace, so `bench-recorder-go-<nonce>` is copied
    into the runner's tmp; the file persists (latched) and the runner
    watches the directory. The nonce rides into the runner through
    BENCH_RUN_NONCE, so a latch left by another invocation can never
    release this one."""
    sentinel = work / f"bench-recorder-go-{nonce}"
    sentinel.write_text("armed\n")
    try:
        sh(f"xcrun devicectl device copy to --device {udid} "
           f"--domain-type appDataContainer "
           f"--domain-identifier {runner_bid} "
           f"--source '{sentinel}' "
           f"--destination 'tmp/{sentinel.name}'",
           capture=True, timeout=DEVICECTL_COPY_S)
    finally:
        sentinel.unlink(missing_ok=True)


def pull_runner_log(udid: str, runner_bid: str, out: Path) -> None:
    """Fetch the runner's tmp/bench-runner.log — it carries the
    measure-window markers, the per-row device record (thermalState,
    maximumFramesPerSecond read inside the runner process) and the
    ready/drive diagnostics."""
    sh(f"xcrun devicectl device copy from --device {udid} "
       f"--domain-type appDataContainer "
       f"--domain-identifier {runner_bid} "
       f"--source tmp/bench-runner.log --destination '{out}'",
       capture=True, timeout=DEVICECTL_COPY_S)


def run_nonce() -> int:
    """A fresh non-zero run nonce for one invocation pair: it keys the
    recorder-go latch and every runner-log line the pair writes."""
    import secrets
    return secrets.randbits(63) | 1


def read_runner_log(path: Path, nonce: int):
    """Parse the runner log for this rep: the runner's marks and the
    device record it wrote inside the measured iteration. Entries are
    '<nonce> <device epoch> <text>' lines; only lines carrying this
    rep's run nonce count, so another rep's lines can never alias into
    this one (the device clock is never compared with the host's).
    Returns ({drive-begin, measure-end: device epoch s}, device record);
    the marks are absent unless each appears exactly once and in order."""
    found: dict[str, list[float]] = {}
    device = {}
    if not path.exists():
        return {}, device
    key = str(nonce)
    for line in path.read_text().splitlines():
        parts = line.split(" ", 2)
        if len(parts) != 3 or parts[0] != key:
            continue
        t, body = float(parts[1]), parts[2]
        if body in (MARK_DRIVE_BEGIN, MARK_MEASURE_END):
            found.setdefault(body, []).append(t)
        if body.startswith("device-record"):
            for kv in body.split():
                if "=" in kv:
                    k, v = kv.split("=", 1)
                    device[k] = v
    marks = {name: ts[0] for name, ts in found.items() if len(ts) == 1}
    if (len(found) != 2 or len(marks) != 2
            or marks[MARK_MEASURE_END] <= marks[MARK_DRIVE_BEGIN]):
        marks = {}
    return marks, device


def _trace_proc_cpu(trace: Path, pid: int, window_ms):
    """CPU seconds of process `pid` inside the window, from the
    all-process time-sample table: Running samples whose thread belongs
    to `pid` (the export names it in the thread's "(name, pid: N)"),
    times the median sample period. window_ms = (w0, w1) on the trace
    clock — the same clock as the frame window."""
    rows, err = _export_table(trace, "time-sample")
    if not rows:
        return None, err or "time-sample: 0 rows"
    stamped = [(_int(r["time"], "time-sample time"), r) for r in rows]
    raw = sorted(t for t, _ in stamped)
    dt = sorted(b - a for a, b in zip(raw, raw[1:]) if b > a)
    if not dt:
        return None, "time-sample: too few samples for a period"
    smp = dt[len(dt) // 2]
    w0, w1 = (w * 1e6 for w in window_ms)
    n = sum(1 for t, r in stamped
            if w0 <= t <= w1
            and r.get("thread-state") == "Running"
            and _pid_of_thread(r.get("thread")) == pid)
    return round(n * smp / 1e9, 3), None


# ------------------------------------------------------------- signing
#
# The device host signs every app itself, with the one "Apple
# Development" identity its login keychain holds and the provisioning
# profile Xcode's automatic signing created on that host for each of the
# two exact bundle ids the manifest declares (the runner's and the one
# every contestant shares). The keychain is reachable from the
# user's GUI session, where the device-run job runs.

@dataclasses.dataclass(frozen=True)
class Identity:
    sha1: str
    name: str


@dataclasses.dataclass(frozen=True)
class Profile:
    path: Path
    uuid: str
    name: str
    team: str
    expires: datetime.datetime
    entitlements: dict


def signing_identity() -> Identity:
    """The keychain's one valid "Apple Development" code-signing
    identity; none or several fail naming what was found."""
    kind = MANIFEST["signing"]["identity_kind"]
    out = _out(["security", "find-identity", "-v", "-p", "codesigning"])
    ids = re.findall(rf'([0-9A-F]{{40}})\s+"({re.escape(kind)}: [^"]+)"',
                     out)
    if len(ids) != 1:
        raise SystemExit(
            f"the device host must hold exactly one valid '{kind}' "
            f"signing identity; found {len(ids)}:\n{out}")
    return Identity(*ids[0])


def decode_profile(path: Path) -> dict:
    return plistlib.loads(
        _out(["security", "cms", "-D", "-i", str(path)]).encode())


def profile_rejection(d: dict, bundle_id: str, identity: Identity,
                      udid: str, not_before: datetime.datetime) -> str | None:
    """Why a decoded profile cannot sign `bundle_id` for this identity on
    this device until `not_before` (naive UTC, as plistlib decodes the
    profile's dates) — None when it can. The App ID must be exactly
    <team>.<bundle_id>: a wildcard profile is never assumed."""
    team = d["TeamIdentifier"][0]
    app_id = d["Entitlements"]["application-identifier"]
    if app_id != f"{team}.{bundle_id}":
        return f"App ID {app_id}"
    certs = {hashlib.sha1(c).hexdigest().upper()
             for c in d["DeveloperCertificates"]}
    if identity.sha1 not in certs:
        return "does not include the signing identity's certificate"
    if udid not in d.get("ProvisionedDevices", []):
        return f"does not provision device {udid}"
    if d["ExpirationDate"] <= not_before:
        return f"expires {d['ExpirationDate']:%Y-%m-%d %H:%M} UTC"
    return None


def find_profile(bundle_id: str, identity: Identity, udid: str) -> Profile:
    """The provisioning profile on this host that signs `bundle_id`
    (exact App ID) with `identity` for the device and stays valid for
    the declared margin; among several, the one valid longest. None
    fails naming the bundle id and how the operator creates its profile —
    there is no fallback to another id."""
    s = MANIFEST["signing"]
    pdir = Path(os.path.expanduser(s["profile_dir"]))
    not_before = (datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
                  + datetime.timedelta(hours=s["profile_min_validity_h"]))
    ok, near = [], []
    for p in sorted(pdir.glob("*.mobileprovision")):
        d = decode_profile(p)
        why = profile_rejection(d, bundle_id, identity, udid, not_before)
        if why is None:
            ok.append(Profile(p, d["UUID"], d["Name"], d["TeamIdentifier"][0],
                              d["ExpirationDate"], d["Entitlements"]))
        elif d["Entitlements"]["application-identifier"].endswith(
                f".{bundle_id}"):
            near.append(f"{p.name}: {why}")
    if ok:
        return max(ok, key=lambda pr: pr.expires)
    raise LookupError(
        f"no usable provisioning profile for bundle id {bundle_id} in "
        f"{pdir} (signed by '{identity.name}', device {udid}, valid "
        f"≥ {s['profile_min_validity_h']} h)"
        + ("".join(f"\n    rejected {n}" for n in near))
        + f"\n  create it on this host with Xcode automatic signing: in "
        f"any iOS app project set the bundle identifier to {bundle_id}, "
        "enable 'Automatically manage signing' with the personal team of "
        f"'{identity.name}', select the attached iPhone and build once "
        "(free-team profiles expire after 7 days)")


def codesign(path: Path, identity: Identity,
             entitlements: Path | None = None) -> None:
    cmd = ["codesign", "--force", "--sign", identity.sha1,
           "--timestamp=none"]
    if entitlements is not None:
        cmd += ["--entitlements", str(entitlements)]
    _out([*cmd, str(path)], timeout=300)


NESTED_CODE = (".framework", ".dylib", ".xctest", ".appex", ".app")


def sign_bundle(app: Path, identity: Identity, profile: Profile) -> None:
    """Embed `profile` and sign `app` inside-out: every nested
    framework, dylib and test bundle (deepest first), then the app with
    the profile's entitlements. A nested app or extension would need a
    profile of its own, which the harness does not provision — it fails."""
    nested = [p for p in app.rglob("*")
              if p.suffix in NESTED_CODE
              and (p.is_dir() or p.suffix == ".dylib")]
    for p in nested:
        if p.suffix in (".appex", ".app"):
            raise RuntimeError(
                f"{p} is a nested app/extension: it needs its own "
                "provisioning profile, which the harness does not "
                "provision")
    shutil.copy2(profile.path, app / "embedded.mobileprovision")
    ents = app.parent / f"{app.stem}.entitlements"
    ents.write_bytes(plistlib.dumps(profile.entitlements))
    for p in sorted(nested, key=lambda q: -len(q.relative_to(app).parts)):
        codesign(p, identity)
    codesign(app, identity, ents)
    ents.unlink()


def signed_copy(src: Path, dst: Path, bundle_id: str, identity: Identity,
                profile: Profile) -> None:
    """Copy a verified staged artifact to `dst`, give it the bundle id it
    is installed as (CFBundleIdentifier) and sign it — the staged
    artifact itself stays as built."""
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(src, dst, symlinks=True)
    info = app_info(dst)
    info["CFBundleIdentifier"] = bundle_id
    (dst / "Info.plist").write_bytes(plistlib.dumps(info))
    sign_bundle(dst, identity, profile)


# ------------------------------------------------------------ thinning

_THIN_APP_SIZE = re.compile(r"^App size: (.+) compressed, (.+) uncompressed$",
                            re.M)


def thinned_size(app: Path, identity: Identity, profile: Profile,
                 product_type: str, work: Path) -> dict:
    """The thinned .ipa of a signed app for the measured device's
    variant (`product_type`), as App Store thinning produces it:
    `xcodebuild -exportArchive` of an archive holding the app, with
    exportOptions `thinning` = the device's product type. Returns the
    variant .ipa's bytes (compressed: the download), the sum of its
    entries' sizes (uncompressed: installed app bytes) and Xcode's own
    App Thinning Size Report line. Runs on the device host — export
    signs, so it needs the host's identity and profile."""
    shutil.rmtree(work, ignore_errors=True)
    archive = work / f"{app.stem}.xcarchive"
    apps = archive / "Products" / "Applications"
    apps.mkdir(parents=True)
    shutil.copytree(app, apps / app.name, symlinks=True)
    info = app_info(app)
    (archive / "Info.plist").write_bytes(plistlib.dumps({
        "ArchiveVersion": 2,
        "CreationDate": datetime.datetime.now(datetime.timezone.utc).replace(
            tzinfo=None),
        "Name": app.stem,
        "SchemeName": app.stem,
        "ApplicationProperties": {
            "ApplicationPath": f"Applications/{app.name}",
            "CFBundleIdentifier": info["CFBundleIdentifier"],
            "CFBundleShortVersionString":
                info["CFBundleShortVersionString"],
            "CFBundleVersion": info["CFBundleVersion"],
            "SigningIdentity": identity.name,
            "Team": profile.team,
            "Architectures": ["arm64"],
        }}))
    opts = work / "ExportOptions.plist"
    opts.write_bytes(plistlib.dumps({
        "method": "debugging",
        "signingStyle": "manual",
        "teamID": profile.team,
        "signingCertificate": identity.sha1,
        "provisioningProfiles": {info["CFBundleIdentifier"]: profile.uuid},
        "thinning": product_type,
    }))
    export = work / "export"
    sh(f"xcodebuild -exportArchive -archivePath '{archive}' "
       f"-exportPath '{export}' -exportOptionsPlist '{opts}'",
       capture=True, timeout=900)
    ipas = sorted(export.rglob("*.ipa"))
    if len(ipas) != 1:
        raise RuntimeError(f"thinned export for {product_type} produced "
                           f"{len(ipas)} .ipa files: {ipas}")
    report = export / "App Thinning Size Report.txt"
    m = _THIN_APP_SIZE.search(report.read_text())
    if m is None:
        raise RuntimeError(f"{report} has no 'App size:' line")
    with zipfile.ZipFile(ipas[0]) as z:
        uncompressed = sum(i.file_size for i in z.infolist())
    out = {"thinned_ipa_bytes": ipas[0].stat().st_size,
           "thinned_app_bytes": uncompressed,
           "thinning_variant": product_type,
           "thinning_report_app_size": {"compressed": m.group(1),
                                        "uncompressed": m.group(2)}}
    shutil.rmtree(work)
    return out


# ------------------------------------------------------------ cells

# a cell's one xctrace recording never outlives it (SIGINT at its end);
# the limit only bounds a cell that hangs (runner thermal gate 600 s,
# launch + ready 60 s, warmup, the capture, and margin)
XCTRACE_LIMIT_S = 1800
SUBDIR = "Release-iphoneos"
RENDER_SERVER = "backboardd"


@dataclasses.dataclass
class DeviceCtx:
    udid: str
    testroot: Path      # __TESTROOT__: signed apps live in testroot/SUBDIR
    template: Path      # the runner build's .xctestrun
    target_key: str     # the test target's key in the .xctestrun
    runner_bid: str     # the runner app's bundle id (log, recorder-go)
    contestant_bid: str  # harness.contestant_bundle_id: every contestant
    results_dir: Path   # per-cell xcresults, traces and logs
    keep_traces: bool
    disk_floor: int     # harness.disk.recording_floor_bytes


def _xcodebuild_test(xr: Path, res: Path, ctx: DeviceCtx, tag: str):
    """Spawn one test-without-building invocation, tracked so a signal
    kills its group. Returns (proc, log file, log path)."""
    cmd = (f"xcodebuild test-without-building -xctestrun '{xr}' "
           f"-destination 'platform=iOS,id={ctx.udid}' "
           f"-resultBundlePath '{res}' -collect-test-diagnostics never")
    xblog = ctx.results_dir / f"{tag}.xcodebuild.log"
    xbf = open(xblog, "w")
    xb = subprocess.Popen(cmd, shell=True, cwd=ROOT, stdout=xbf,
                          stderr=subprocess.STDOUT, start_new_session=True)
    _ACTIVE_PROCS.append(xb)
    return xb, xbf, xblog


def _xcodebuild_wait(xb, xbf, xblog: Path, bound_s: int = 2400):
    """Wait for an invocation; (rc | None on timeout, output)."""
    try:
        xb.wait(timeout=bound_s)
        rc = xb.returncode
    except subprocess.TimeoutExpired:
        _kill_proc(xb)
        xb.wait()
        rc = None
    finally:
        if xb in _ACTIVE_PROCS:
            _ACTIVE_PROCS.remove(xb)
        xbf.close()
    out = xblog.read_text()
    xblog.unlink()
    return rc, out


def _xcodebuild_error(what: str, rc, out: str) -> str | None:
    if rc is None:
        return f"xcodebuild({what}) timed out after 2400s"
    if rc != 0:
        return f"xcodebuild({what}) rc={rc}: {out[-800:]}"
    return None


def warm_up(ctx: DeviceCtx, cid: str, app: Path, tag: str) -> str | None:
    """One discarded launch test: the first XCUITest attach after an
    install can fail inside the framework ("Failed to initialize for UI
    testing: XCTFuture") and must not land on a measured rep. Returns
    the invocation's error, recorded but never counted."""
    xr = ctx.testroot / f"{tag}-warmup.xctestrun"
    res = ctx.results_dir / f"{tag}-warmup.xcresult"
    shutil.rmtree(res, ignore_errors=True)
    write_xctestrun(ctx.template, xr, ctx.target_key, SUBDIR, app.name,
                    ctx.contestant_bid, cid, "w1", drive_for("w1"),
                    MANIFEST["workloads"]["w1"]["duration_s"],
                    nonce=run_nonce(), only_test="testLaunch")
    try:
        return _xcodebuild_error(
            "warm-up", *_xcodebuild_wait(*_xcodebuild_test(
                xr, res, ctx, f"{tag}-warmup")))
    finally:
        shutil.rmtree(res, ignore_errors=True)
        xr.unlink()


def run_one(ctx: DeviceCtx, cid: str, app: Path, workload: str, rep: int,
            step: int | None = None) -> dict:
    """One measured launch: two single-test xcodebuild invocations and
    one recording.

    The launch test and the workload test never share a process
    lifetime. Before the workload invocation's runner may launch the app,
    one `xctrace record --all-processes --device` recording is armed:
    Animation Hitches plus Points of Interest plus os_log (frames, the
    runner's drive-begin / measure-end marks, time-sample CPU and the
    WaterUI first-paint marker; the same instruments for every
    contestant, so each pays the same recording cost). The contestant's
    frames are the frame lifetimes whose swap composited an update from
    a process inside its bundle; METHOD's window [first owned present +
    warmup, + capture] is computed on that trace's clock, and the
    runner's marks prove that a driven cell's drive started within
    harness.anchor_tolerance_ms of the window start and that the
    contestant was held through its end. Every frame on the contestant's
    display inside the window is classified by the client updates its
    swap carried and stored on the row (`frames_by_update`); a window
    holding a frame with no client update fails the rep
    (require_client_updates). The app's and backboardd's CPU come from
    the same trace and window.

    The cell checks free disk against the declared recording floor
    before it starts, and its InstrumentsScratch removes the raw ktrace
    scratch once the recorder stopped, so the next recording starts
    without it. Unless the run keeps traces, the cell's .trace bundle is
    deleted once its numbers are extracted (its size is recorded)."""
    tag = f"{cid}-{workload}" + (f"-s{step}" if step is not None else "") \
        + f"-r{rep}"
    rec: dict = {}
    free = shutil.disk_usage(ctx.results_dir).free
    rec["disk_free_bytes"] = free
    if free < ctx.disk_floor:
        rec["error"] = (f"disk: {free} bytes free under {ctx.results_dir}, "
                        "below harness.disk.recording_floor_bytes="
                        f"{ctx.disk_floor}")
        return rec
    nonce = run_nonce()
    drive = drive_for(workload)
    duration = MANIFEST["workloads"][workload]["duration_s"]
    xr_launch = ctx.testroot / f"{tag}-launch.xctestrun"
    xr_work = ctx.testroot / f"{tag}-work.xctestrun"
    for xr, only in ((xr_launch, "testLaunch"), (xr_work, "testWorkload")):
        write_xctestrun(ctx.template, xr, ctx.target_key, SUBDIR, app.name,
                        ctx.contestant_bid, cid, workload, drive, duration,
                        nonce=nonce, only_test=only, step=step)
    res_launch = ctx.results_dir / f"{tag}-launch.xcresult"
    res_work = ctx.results_dir / f"{tag}-work.xcresult"
    trace = ctx.results_dir / f"{tag}.trace"
    runner_log = ctx.results_dir / f"{tag}-runner.log"
    for p in (res_launch, res_work, trace):
        shutil.rmtree(p, ignore_errors=True)
    runner_log.unlink(missing_ok=True)

    exe = bundle_executable(app)
    capture_ms = float(duration) * 1000.0
    recorder: XctraceRecorder | None = None
    try:
        # --- invocation 1: launch test (its own process lifetime) ----
        rc, out = _xcodebuild_wait(*_xcodebuild_test(
            xr_launch, res_launch, ctx, tag + "-launch"))
        if err := _xcodebuild_error("launch", rc, out):
            rec["error"] = err
            return rec
        launch = _metrics_record(res_launch)
        if "error" in launch:
            return {**rec, **launch}
        rec["metrics"] = launch["metrics"]

        # --- invocation 2: workload test, the recorder armed first ---
        scratch = InstrumentsScratch(ctx.results_dir,
                                     InstrumentsScratch.user_temp_dir())
        xb, xbf, xblog = _xcodebuild_test(xr_work, res_work, ctx,
                                          tag + "-work")
        try:
            try:
                recorder = XctraceRecorder(
                    trace, FRAMES_TEMPLATE, ctx.udid, XCTRACE_LIMIT_S,
                    scratch.scratch_dir(tag),
                    instruments=FRAMES_INSTRUMENTS)
                recorder.arm()
                release_recorder_go(ctx.udid, ctx.runner_bid, nonce,
                                    ctx.results_dir)
            except RuntimeError as e:
                # nothing was released: the runner would only time out
                # on its recorder-go gate — end the invocation now
                rec["error"] = f"recorder arming: {e}"
                _kill_proc(xb)
            rc, out = _xcodebuild_wait(xb, xbf, xblog)
            if "error" not in rec:
                if err := _xcodebuild_error("workload", rc, out):
                    rec["error"] = err
                else:
                    work = _metrics_record(res_work)
                    if "error" in work:
                        rec.update(work)
                    else:
                        rec["metrics"].update(work["metrics"])
        finally:
            # the sweep runs even when stopping is interrupted (a SIGTERM
            # unwinding through here): the scratch never outlives the cell
            try:
                if recorder is not None and (e := recorder.stop()):
                    rec["trace_error"] = e
            finally:
                if (e := scratch.sweep()) is not None:
                    rec.setdefault("error", f"instruments scratch: {e}")
                rec["instruments_scratch_removed"] = scratch.removed
                rec["instruments_scratch_terminated_pid"] = \
                    scratch.terminated
        if "error" in rec:
            return rec
        if "trace_error" in rec:
            rec["error"] = f"the recording failed: {rec['trace_error']}"
            return rec

        # --- the measurement window --------------------------------
        try:
            pull_runner_log(ctx.udid, ctx.runner_bid, runner_log)
        except RuntimeError as e:
            rec["error"] = f"runner log: {e}"
            return rec
        marks, device_rec = read_runner_log(runner_log, nonce)
        rec["device_record"] = device_rec
        if not marks:
            rec["error"] = ("runner log has no drive-begin/measure-end "
                            f"pair for run nonce {nonce} — the runner never "
                            "reached its measured span")
            return rec
        if not device_rec.get("maxFps"):
            rec["error"] = "runner log has no device-record maxFps"
            return rec
        refresh_ms = 1000.0 / float(device_rec["maxFps"])
        try:
            tw = trace_frame_window(trace, app.name, exe, capture_ms,
                                    refresh_ms=refresh_ms,
                                    driven=drive != "none")
            rec["window"] = {"source": "trace", **tw["window"]}
            rec["attribution"] = tw["attribution"]
            rec["frames_by_update"] = tw["frames_by_update"]
            require_client_updates(tw["frames_by_update"])
            render = [p["pid"] for p in _trace_toc(trace)
                      if p["name"] == RENDER_SERVER]
            if len(render) != 1:
                raise TraceAttributionError(
                    f"trace lists {len(render)} {RENDER_SERVER} processes")
            if cid == "waterui":
                rec["first_paint_ms"] = first_paint_ms(trace, exe)
        except TraceAttributionError as e:
            rec["error"] = f"frame attribution: {e}"
            return rec
        rec["refresh_ms"] = refresh_ms
        rec["frame_stats"] = tw["frame_stats"]
        rec["frames"] = tw["frame_stats"]["presents"]
        win = (tw["window"]["window_start_ms"], tw["window"]["window_end_ms"])
        # the all-process recording carries every process's samples; the
        # contestant and the render server are selected by pid
        rec["renderserver"] = RENDER_SERVER
        for pid, key in ((tw["attribution"]["main_pid"], "app_cpu_window_s"),
                         (render[0], "renderserver_cpu_s")):
            cpu_s, e = _trace_proc_cpu(trace, pid, win)
            if cpu_s is None:
                rec["error"] = f"time-sample CPU of pid {pid}: {e}"
                return rec
            rec[key] = cpu_s
        if rec["frames"]:
            rec["cpu_ms_per_frame"] = round(
                rec["app_cpu_window_s"] * 1000.0 / rec["frames"], 3)
        return rec
    finally:
        rec["trace_bytes"] = du_bytes(trace) if trace.exists() else 0
        if not ctx.keep_traces:
            shutil.rmtree(trace, ignore_errors=True)
        # keep xcresult small: the bundles are parsed and deleted
        shutil.rmtree(res_launch, ignore_errors=True)
        shutil.rmtree(res_work, ignore_errors=True)
        xr_launch.unlink(missing_ok=True)
        xr_work.unlink(missing_ok=True)


def capacity_summary(steps: list[dict]) -> dict:
    """Per-step budget shares and the ladder's capacities. A present is
    inside a budget when the interval since the previous present is at
    most budget × frame_stats.MISS_FACTOR (the same tolerance a missed
    vsync is counted with); shares are over the step's presents, so a
    present after an excluded >100 ms gap is outside every budget. A step
    collapses when fewer than half its presents land inside two 60 Hz
    budgets (WORKLOADS.md W5). capacity_<rate> is the largest step with
    ≥ 99% of presents inside that rate's budget."""
    def share(st, limit_ms):
        fs = st["frame_stats"]
        if not fs["presents"]:
            return 0.0
        n = sum(1 for iv in fs["intervals_ms"] if iv <= limit_ms)
        return round(100.0 * n / fs["presents"], 2)
    cap120 = cap60 = 0
    for st in steps:
        st["in_120hz_pct"] = share(st, 1000 / 120 * lib_frames.MISS_FACTOR)
        st["in_60hz_pct"] = share(st, 1000 / 60 * lib_frames.MISS_FACTOR)
        st["collapsed"] = share(st, 2000 / 60) < 50.0
        if st["in_120hz_pct"] >= 99.0:
            cap120 = max(cap120, st["n"])
        if st["in_60hz_pct"] >= 99.0:
            cap60 = max(cap60, st["n"])
    return {"capacity_120hz": cap120, "capacity_60hz": cap60}


STEP_KEYS = ("metrics", "frame_stats", "frames", "app_cpu_window_s",
             "renderserver_cpu_s", "cpu_ms_per_frame", "window",
             "attribution", "frames_by_update",
             "trace_bytes", "disk_free_bytes", "first_paint_ms")


def measure_cell(ctx: DeviceCtx, cid: str, app: Path, workload: str,
                 rep: int) -> dict:
    """One (contestant, workload, rep) row. A capacity workload runs one
    launch per ladder step and stops at the first collapsed step (or a
    failed one, which fails the row); its row carries every step and the
    ladder's capacities."""
    if not capacity_workload(workload):
        return run_one(ctx, cid, app, workload, rep)
    steps = []
    for n in MANIFEST["workloads"][workload]["steps"]:
        print(f"  step {n}", flush=True)
        r = run_one(ctx, cid, app, workload, rep, step=n)
        if "error" in r:
            return {**r, "capacity": {"steps": steps, "failed_step": n}}
        steps.append({"n": n, **{k: r[k] for k in STEP_KEYS if k in r}})
        summary = capacity_summary(steps)
        if steps[-1]["collapsed"]:
            break
    return {"capacity": {"steps": steps, **summary},
            "metrics": steps[0]["metrics"],
            "frames": sum(s["frames"] for s in steps)}


# ---------------------------------------------------------------- device

def device_lock(name: str, holder: dict):
    """Take the device's exclusive lock without waiting, and record
    `holder` (who took it) in the lock file. A lock another run holds
    fails at once, naming the holder that run recorded — a run never
    queues behind another one. The file is opened without truncation, so
    a failed attempt leaves the holder's record intact; the kernel drops
    the flock when its holder exits, so a stale record never blocks."""
    LOCK_DIR.mkdir(exist_ok=True)
    path = LOCK_DIR / f"{name}.lock"
    f = os.fdopen(os.open(path, os.O_RDWR | os.O_CREAT, 0o644), "r+")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        recorded = f.read().strip()
        f.close()
        raise SystemExit(
            f"device {name} is locked by another run ({path}): "
            f"{recorded or 'the holder recorded nothing'}") from None
    f.seek(0)
    f.truncate()
    f.write(json.dumps(holder) + "\n")
    f.flush()
    print(f"device lock {name} acquired: {json.dumps(holder)}", flush=True)
    return f


# devicectl `device info details --json-output` field names
_DEVICE_INFO_FIELDS = ("marketingName", "deviceName", "productType",
                       "osVersionNumber", "productVersion")


def _find_json_fields(node, wanted) -> dict:
    """First scalar value of each wanted key, anywhere in the JSON tree."""
    out = {}
    stack = [node]
    while stack:
        cur = stack.pop()
        if isinstance(cur, dict):
            for k, v in cur.items():
                if k in wanted and not isinstance(v, (dict, list)):
                    out.setdefault(k, v)
                elif isinstance(v, (dict, list)):
                    stack.append(v)
        elif isinstance(cur, list):
            stack.extend(cur)
    return out


def parse_device_info(doc: dict) -> dict:
    """Model, product type and OS of one devicectl JSON document; a field
    devicectl did not report is absent (thermal state and refresh rate
    come from the on-device runner, never from here)."""
    found = _find_json_fields(doc, _DEVICE_INFO_FIELDS)
    st = {}
    if found.get("marketingName") or found.get("deviceName"):
        st["model"] = str(found.get("marketingName") or found["deviceName"])
    if found.get("productType"):
        st["product_type"] = str(found["productType"])
    if found.get("osVersionNumber") or found.get("productVersion"):
        st["os_version"] = str(found.get("osVersionNumber")
                               or found["productVersion"])
    return st


def device_state(udid: str) -> dict:
    """The declared iPhone's identity as devicectl reports it; it must be
    the declared product type — the harness measures one device."""
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "info.json"
        sh(f"xcrun devicectl device info details --device {udid} "
           f"--json-output '{out}'", capture=True, timeout=DEVICECTL_QUERY_S)
        st = {"udid": udid, **parse_device_info(json.loads(out.read_text()))}
    want = MANIFEST["device"]["product_type"]
    if st.get("product_type") != want:
        raise SystemExit(f"device {udid} reports product type "
                         f"{st.get('product_type')!r}; the manifest "
                         f"declares {want!r} ({MANIFEST['device']['model']})")
    return st


def _utc() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat(
        timespec="milliseconds")


# the fields of a devicectl installed-app entry a row keeps as evidence
_APP_FIELDS = ("bundleIdentifier", "name", "url", "version", "bundleVersion")


def listed_apps(doc: dict, bundle_id: str) -> list[dict]:
    """The entries of one `devicectl device info apps --json-output`
    document whose bundleIdentifier is exactly `bundle_id`, reduced to
    _APP_FIELDS. A document without `result.apps` is not a listing: it
    raises naming what it holds, never reads as "nothing installed"."""
    apps = (doc.get("result") or {}).get("apps") \
        if isinstance(doc, dict) else None
    if not isinstance(apps, list):
        raise RuntimeError("devicectl info apps reported no result.apps: "
                           f"{json.dumps(doc)[:400]}")
    return [{k: a.get(k) for k in _APP_FIELDS} for a in apps
            if a.get("bundleIdentifier") == bundle_id]


def installed_bundle_name(entry: dict) -> str:
    """The .app directory name of an installed app, from the file URL
    devicectl lists it at (…/Bundle/Application/<UUID>/<name>.app/)."""
    path = urllib.parse.unquote(urllib.parse.urlparse(entry["url"] or "").path)
    return Path(path).name


class Devicectl:
    """The device's apps through `xcrun devicectl` — the one channel the
    run lists, installs and uninstalls apps with. Every call carries its
    declared bound; a failure or timeout raises (sh)."""

    def __init__(self, udid: str):
        self.udid = udid

    def listed(self, bundle_id: str) -> list[dict]:
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "apps.json"
            sh(f"xcrun devicectl device info apps --device {self.udid} "
               f"--bundle-id {bundle_id} --json-output '{out}'",
               capture=True, timeout=DEVICECTL_QUERY_S)
            return listed_apps(json.loads(out.read_text()), bundle_id)

    def uninstall(self, bundle_id: str) -> None:
        sh(f"xcrun devicectl device uninstall app --device {self.udid} "
           f"{bundle_id}", capture=True, timeout=DEVICECTL_UNINSTALL_S)

    def install(self, app: Path) -> None:
        sh(f"xcrun devicectl device install app --device {self.udid} "
           f"'{app}'", capture=True, timeout=DEVICECTL_INSTALL_S)


# what a devicectl step can raise: a failed or timed-out command
# (RuntimeError, CommandTimeout), an unreadable or misshapen listing
_DEVICECTL_ERRORS = (RuntimeError, ValueError, OSError)


def clear_bundle_id(dev, bundle_id: str) -> dict:
    """Remove every app carrying `bundle_id` from the device and verify
    that devicectl's installed-apps listing no longer shows one. Returns
    the evidence — what was listed before, the uninstall, what was
    listed after, each timestamped — with an `error` when the listing or
    the uninstall failed or the id is still installed; it never raises
    for the device, so the caller records the evidence either way."""
    ev: dict = {"bundle_id": bundle_id, "started_utc": _utc()}
    try:
        ev["listed_before"] = dev.listed(bundle_id)
        if ev["listed_before"]:
            dev.uninstall(bundle_id)
            ev["uninstall"] = "uninstalled"
        else:
            ev["uninstall"] = "not installed"
        ev["uninstall_finished_utc"] = _utc()
        ev["listed_after"] = dev.listed(bundle_id)
        ev["verified_utc"] = _utc()
    except _DEVICECTL_ERRORS as e:
        ev["error"] = f"uninstall {bundle_id}: {e}"
        return ev
    ev["verified_absent"] = not ev["listed_after"]
    if ev["listed_after"]:
        ev["error"] = (f"{bundle_id} is still installed after its "
                       f"uninstall: {ev['listed_after']}")
    return ev


def install_cycle(dev, bundle_id: str, app: Path,
                  installed: list[str]) -> dict:
    """Install one contestant under the shared `bundle_id`: clear the id
    and verify the device carries no app with it (clear_bundle_id), then
    install `app` and verify the one app listed under the id is this
    stage entry's bundle (its .app name). A failed verification returns
    before anything installs. `bundle_id` joins `installed` before the
    install starts, so an install that fails or is interrupted midway is
    still uninstalled by the caller. The returned evidence goes on every
    row of the contestant's cells; `error` fails them all."""
    cycle: dict = {"bundle_id": bundle_id, "app": app.name,
                   "pre_install": clear_bundle_id(dev, bundle_id)}
    if "error" in cycle["pre_install"]:
        cycle["error"] = ("no install: the pre-install verification "
                          f"failed: {cycle['pre_install']['error']}")
        return cycle
    installed.append(bundle_id)
    ev: dict = {"started_utc": _utc()}
    cycle["install"] = ev
    try:
        dev.install(app)
        ev["finished_utc"] = _utc()
        ev["listed"] = dev.listed(bundle_id)
        ev["verified_utc"] = _utc()
    except _DEVICECTL_ERRORS as e:
        cycle["error"] = f"install {app.name}: {e}"
        return cycle
    names = [installed_bundle_name(a) for a in ev["listed"]]
    if names != [app.name]:
        cycle["error"] = (f"after installing {app.name}, devicectl lists "
                          f"{bundle_id} as {names or 'nothing'}: "
                          f"{ev['listed']}")
    return cycle


_SHA256_RE = re.compile(r"[0-9a-f]{64}")


def verify_stage(tar: Path, want: str) -> None:
    """The staged tarball is exactly the one `build` printed the sha256
    of; any other bytes fail before they are used."""
    got = toolchain.sha256_file(tar)
    if got != want:
        raise SystemExit(f"staged set {tar} has sha256 {got}; "
                         f"--stage-sha256 declares {want}")


def unpack_stage(tar: Path, run_dir: Path) -> tuple[Path, dict]:
    """Extract the build host's staged set into the run dir and verify
    it: the device host's checkout is the HEAD the build was made from
    (its harness and runner sources are the ones the build labels), and
    every artifact hashes to the staging manifest."""
    with tarfile.open(tar) as t:
        t.extractall(run_dir, filter="data")
    stage = run_dir / "stage"
    staging = json.loads((stage / "staging-manifest.json").read_text())
    toolchain.require_clean_checkout()
    head = toolchain.checkout_head()
    if staging["checkout_head"] != head:
        raise SystemExit(
            f"staged set was built at {staging['checkout_head']}, this "
            f"checkout is at {head} — check out the build's HEAD on the "
            "device host")
    check_stage_entries(staging["artifacts"])
    for cid, a in staging["artifacts"].items():
        if dir_sha256(stage / a["app"]) != a["sha256"]:
            raise SystemExit(f"staged {a['app']} ({cid}) does not match "
                             "the staging manifest")
    return stage, staging


def check_stage_entries(artifacts: dict) -> None:
    """Every contestant installs under one shared bundle id, so a
    contestant on the device is told apart only by its stage entry: the
    staged set holds exactly the manifest's contestants, and their .app
    names (what the installed-app check and the trace's owned-process
    match key on) are pairwise distinct."""
    want = {c["id"] for c in MANIFEST["contestants"]}
    if set(artifacts) != want:
        raise SystemExit(f"staged set holds contestants {sorted(artifacts)}; "
                         f"the manifest declares {sorted(want)}")
    names = [a["app"] for a in artifacts.values()]
    if len(set(names)) != len(names):
        raise SystemExit(f"staged .app names are not distinct: {names} — "
                         "contestants sharing one bundle id are identified "
                         "by their .app name")


def build_runner(run_dir: Path) -> dict:
    """Build the XCTest UI runner on this (device) host with its own
    Xcode: xcodegen generates xctest/BenchRunner.xcodeproj, xcodebuild
    build-for-testing produces the unsigned runner app and the .xctestrun
    template under the run dir. The runner's bundle id is the manifest's
    `runner.bundle_id`."""
    toolchain.require_version(
        "xcodegen", ["xcodegen", "--version"],
        MANIFEST["toolchain"]["device_host"]["xcodegen"])
    dd = run_dir / "runner-dd"
    with toolchain.tracked_tree_unchanged("apple runner build", [ROOT]):
        sh(f"xcodegen --spec '{RUNNER_PROJECT / 'project.yml'}'")
        sh("xcodebuild -project xctest/BenchRunner.xcodeproj "
           "-scheme BenchRunner -configuration Release "
           f"-destination 'generic/platform=iOS' -derivedDataPath '{dd}' "
           f"PRODUCT_BUNDLE_IDENTIFIER={MANIFEST['runner']['bundle_id']} "
           "CODE_SIGNING_ALLOWED=NO build-for-testing")
    products = dd / "Build" / "Products"
    xctestruns = sorted(products.glob("*.xctestrun"))
    if len(xctestruns) != 1:
        raise SystemExit(f"runner build produced {len(xctestruns)} "
                         f".xctestrun files in {products}")
    app = products / SUBDIR / "BenchRunner-Runner.app"
    want = MANIFEST["runner"]["bundle_id"] + ".xctrunner"
    if app_info(app)["CFBundleIdentifier"] != want:
        raise SystemExit(f"{app} is {app_info(app)['CFBundleIdentifier']}, "
                         f"expected {want}")
    return {"app": app, "xctestrun": xctestruns[0], "bundle_id": want}


def check_resume(state: dict, staging: dict, machine: dict,
                 udid: str) -> None:
    """A results file only grows with rows of the same staged build, the
    same device host and the same device. The recorded device host must
    carry both identity fields (`machine` comes from host_evidence, which
    never yields an empty one): two unknown identities are never the
    same host."""
    if state.get("build") != staging:
        raise SystemExit("results file was produced from a different "
                         "staged build — refusing to merge")
    prev = state.get("machine") or {}
    unknown = [k for k in ("hw_uuid", "hw_model") if not prev.get(k)]
    if unknown:
        raise SystemExit(
            "results file records no device host identity "
            f"({', '.join(unknown)} missing) — refusing to merge\n"
            f"  recorded: {prev}")
    if (prev.get("hw_uuid"), prev.get("hw_model")) != (
            machine.get("hw_uuid"), machine.get("hw_model")):
        raise SystemExit(
            "results file was produced on a different device host — "
            f"refusing to merge\n  recorded: {prev}\n  current : {machine}")
    if (state.get("device") or {}).get("udid") != udid:
        raise SystemExit("results file was produced on a different device "
                         "— refusing to merge")


def run_device(run_dir: Path, opts: argparse.Namespace) -> None:
    """One measurement run on the device host (inside the GUI-session
    job). Builds the runner, then installs one contestant at a time
    under the one shared harness.contestant_bundle_id (free provisioning
    allows three apps installed at once: the runner and the contestant
    under measurement): before every install it clears the id and
    verifies on the device that no app carries it (install_cycle),
    measures the contestant's cells with that evidence on every row, and
    uninstalls the id again after them."""
    # the staged set is the one `start` verified: re-checked here, before
    # anything else, since the tarball could change while the job waited
    verify_stage(Path(opts.stage), opts.stage_sha256)
    udid = MANIFEST["device"]["udid"]
    lock = device_lock(udid, {
        "pid": os.getpid(), "run_dir": str(run_dir),
        "label": _job_label(run_dir),
        "acquired_utc": datetime.datetime.now(
            datetime.timezone.utc).isoformat(timespec="seconds")})
    dev = Devicectl(udid)
    installed: list[str] = []
    diags: list[str] = []

    def uninstall(bid: str) -> dict:
        """clear_bundle_id after use; a failure becomes a diagnostic so
        it never suppresses the rest of the cleanup."""
        ev = clear_bundle_id(dev, bid)
        if "error" in ev:
            print(f"cleanup failed: {ev['error']}", flush=True)
            diags.append(ev["error"])
        return ev

    def cleanup():
        for bid in reversed(installed):
            uninstall(bid)
        installed.clear()

    def on_signal(sig, _frame):
        _kill_active_procs()   # only PIDs this run spawned
        cleanup()
        raise SystemExit(128 + sig)

    prev = (signal.signal(signal.SIGINT, on_signal),
            signal.signal(signal.SIGTERM, on_signal))
    try:
        stage, staging = unpack_stage(Path(opts.stage), run_dir)
        machine = host_evidence()
        xcode = xcode_identity()
        device = device_state(udid)
        identity = signing_identity()
        contestants = [c for c in MANIFEST["contestants"]
                       if not opts.only or c["id"] in opts.only]
        workloads = [w for w in MANIFEST["workloads"]
                     if not opts.workloads or w in opts.workloads]
        runner_bid = MANIFEST["runner"]["bundle_id"] + ".xctrunner"
        bid = MANIFEST["harness"]["contestant_bundle_id"]
        # exactly two App IDs: the runner's and the one every contestant
        # is signed and installed as
        profiles, missing = {}, []
        for b in (runner_bid, bid):
            try:
                profiles[b] = find_profile(b, identity, udid)
            except LookupError as e:
                missing.append(str(e))
        if missing:
            raise SystemExit("\n".join(missing))

        runner = build_runner(run_dir)
        testroot = run_dir / "testroot"
        prod = testroot / SUBDIR
        prod.mkdir(parents=True)
        signed_copy(runner["app"], prod / runner["app"].name, runner_bid,
                    identity, profiles[runner_bid])
        template = testroot / "runner.xctestrun"
        shutil.copy2(runner["xctestrun"], template)

        results_path = Path(opts.out) if opts.out else \
            run_dir / "results-device.json"
        state = {"machine": machine, "device": device, "build": staging,
                 "device_host_xcode": xcode,
                 "stage_tar_sha256": opts.stage_sha256,
                 "signing": {"identity": identity.name, "profiles": {
                     b: {"uuid": p.uuid, "name": p.name, "team": p.team,
                         "expires_utc": p.expires.isoformat()}
                     for b, p in profiles.items()}},
                 "runs": [], "sizes": {}, "warmed_up": []}
        if results_path.exists():
            prior = json.loads(results_path.read_text())
            check_resume(prior, staging, machine, udid)
            state = {**prior, **{k: v for k, v in state.items()
                                 if k not in ("runs", "sizes",
                                              "warmed_up")}}
        sanitize_runs(state)
        reps = (sorted({int(i) for i in opts.reps.split(",")})
                if opts.reps else list(range(opts.repeats)))
        cells = {(c["id"], w, rep) for c in contestants for w in workloads
                 for rep in reps}
        state["runs"] = [x for x in state["runs"]
                         if (x.get("contestant"), x.get("workload"),
                             x.get("repeat")) not in cells]
        results_dir = run_dir / "cells"
        results_dir.mkdir(exist_ok=True)
        ctx = DeviceCtx(udid=udid, testroot=testroot, template=template,
                        target_key=MANIFEST["runner"]["test_target"],
                        runner_bid=runner_bid, contestant_bid=bid,
                        results_dir=results_dir,
                        keep_traces=opts.keep_traces,
                        disk_floor=int(MANIFEST["harness"]["disk"]
                                       ["recording_floor_bytes"]))

        def save():
            results_path.write_text(json.dumps(state, indent=1))

        installed.append(runner_bid)  # xcodebuild installs it per run
        signed = set()
        for rep in reps:
            # interleave: rotate order so no contestant gets the same slot
            k = rep % len(contestants)
            for c in contestants[k:] + contestants[:k]:
                art = staging["artifacts"][c["id"]]
                app = prod / art["app"]
                if c["id"] not in signed:
                    signed_copy(stage / art["app"], app, bid, identity,
                                profiles[bid])
                    signed.add(c["id"])
                    size = {k2: art[k2] for k2 in
                            ("app_bytes", "unsigned_ipa_bytes")}
                    try:
                        size.update(thinned_size(
                            app, identity, profiles[bid],
                            device["product_type"],
                            run_dir / "thinning" / c["id"]))
                    except (RuntimeError, KeyError, OSError) as e:
                        size["error"] = f"thinning: {e!r}"
                    state["sizes"][c["id"]] = size
                    save()
                # every row of this install carries the same cycle dict:
                # the pre-install clear + verification, the install, and
                # (once the cells are done) the post-cells uninstall
                print(f"install {c['id']} as {bid}", flush=True)
                cycle = install_cycle(dev, bid, app, installed)
                try:
                    if ("error" not in cycle
                            and c["id"] not in state["warmed_up"]):
                        print(f"warm-up {c['id']} (discarded)", flush=True)
                        err = warm_up(ctx, c["id"], app, f"{c['id']}-r{rep}")
                        state.setdefault("warmups", []).append(
                            {"contestant": c["id"], "error": err})
                        state["warmed_up"].append(c["id"])
                        save()
                    for w in workloads:
                        print(f"rep {rep + 1}/{len(reps)} {c['id']} {w} "
                              f"drive={drive_for(w)}", flush=True)
                        # a failed cycle fails the cell; nothing measures
                        rec = ({"error": f"install cycle: {cycle['error']}"}
                               if "error" in cycle else
                               measure_cell(ctx, c["id"], app, w, rep))
                        rec.update({"contestant": c["id"], "workload": w,
                                    "repeat": rep, "drive": drive_for(w),
                                    "artifact_sha256": art["sha256"],
                                    "install_cycle": cycle})
                        state["runs"].append(rec)
                        save()
                finally:
                    # uninstall what this run installed, even when a cell
                    # errored — the free team caps installed apps at three
                    if bid in installed:
                        cycle["post_uninstall"] = uninstall(bid)
                        installed.remove(bid)
                        save()
        print(f"device results → {results_path}")
    finally:
        cleanup()
        signal.signal(signal.SIGINT, prev[0])
        signal.signal(signal.SIGTERM, prev[1])
        lock.close()
        if diags:
            print("cleanup diagnostics: " + "; ".join(diags), flush=True)


def cmd_device_run(args):
    """The device-run job (launchd starts it in the GUI session). Its
    selection comes from <run-dir>/args.json, written by `device-session
    start`; its exit status lands in <run-dir>/status.json whatever ends
    it — success, a failed precondition, an exception or SIGTERM from
    `device-session stop`."""
    run_dir = Path(args.run_dir)
    opts = argparse.Namespace(**json.loads(
        (run_dir / "args.json").read_text()))
    status = {"started_utc": datetime.datetime.now(
        datetime.timezone.utc).isoformat(timespec="seconds")}
    code = 1
    try:
        run_device(run_dir, opts)
        code = 0
    except SystemExit as e:
        code = e.code if isinstance(e.code, int) else 1
        if not isinstance(e.code, int):
            status["error"] = str(e.code)
    except BaseException:  # noqa: BLE001 — recorded, then re-exited
        status["error"] = traceback.format_exc()
        traceback.print_exc()
    finally:
        status["exit_code"] = code
        status["finished_utc"] = datetime.datetime.now(
            datetime.timezone.utc).isoformat(timespec="seconds")
        (run_dir / "status.json").write_text(json.dumps(status, indent=1))
    raise SystemExit(code)


# ------------------------------------------------------- GUI-session job
#
# xctrace and DTServiceHub only reach the device from the logged-in
# user's GUI (Aqua) session, and the login keychain that holds the
# signing identity is unlocked there; an ssh session has neither. The
# operator therefore never runs the measurement in the ssh session:
# `device-session start` writes a one-shot LaunchAgent for the run and
# bootstraps it into gui/<uid>, where launchd starts `device-run`
# detached from ssh. The job owns its log (job.log) and exit status
# (status.json) in the run dir; `status` reads them with launchd's view
# of the job, `stop` boots it out (launchd SIGTERMs it; device-run
# uninstalls what it installed, sweeps its scratch and records the exit
# within the job's ExitTimeOut, JOB_EXIT_TIMEOUT_S, before launchd would
# SIGKILL it).

def _job_label(run_dir: Path) -> str:
    return f"dev.bench.device.{run_dir.name}"


def _launchctl_print(label: str) -> str | None:
    r = subprocess.run(["launchctl", "print",
                        f"gui/{os.getuid()}/{label}"],
                       capture_output=True, text=True, timeout=30)
    return r.stdout if r.returncode == 0 else None


def cmd_session(args):
    if args.action == "start":
        stage = Path(args.stage).resolve()
        if not stage.is_file():
            raise SystemExit(f"no staged set at {stage}")
        # before anything else: the tarball is the one `build` hashed
        verify_stage(stage, args.stage_sha256)
        run_dir = RUNS_DIR / datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y%m%dT%H%M%SZ")
        run_dir.mkdir(parents=True)
        (run_dir / "args.json").write_text(json.dumps({
            "stage": str(stage),
            "stage_sha256": args.stage_sha256,
            "only": args.only.split(",") if args.only else None,
            "workloads": args.workloads.split(",") if args.workloads
            else None,
            "repeats": args.repeats, "reps": args.reps,
            "out": str(Path(args.out).resolve()) if args.out else None,
            "keep_traces": args.keep_traces}, indent=1))
        label = _job_label(run_dir)
        plist = run_dir / "job.plist"
        plist.write_bytes(plistlib.dumps({
            "Label": label,
            "ProgramArguments": [sys.executable, str(ROOT / "bench.py"),
                                 "device-run", "--run-dir", str(run_dir)],
            "WorkingDirectory": str(ROOT),
            # the job runs once; launchd never restarts it
            "RunAtLoad": True,
            "KeepAlive": False,
            "LimitLoadToSessionType": "Aqua",
            "ProcessType": "Interactive",
            # SIGTERM → SIGKILL grace on `stop`: the declared bounds of
            # everything a stopping run still does (JOB_EXIT_TIMEOUT_S),
            # so it is never killed mid-cleanup
            "ExitTimeOut": JOB_EXIT_TIMEOUT_S,
            "StandardOutPath": str(run_dir / "job.log"),
            "StandardErrorPath": str(run_dir / "job.log"),
            # the login shell's PATH (xcodegen, uv-managed tools) and HOME
            "EnvironmentVariables": {"PATH": os.environ["PATH"],
                                     "HOME": os.environ["HOME"],
                                     "LANG": "en_US.UTF-8",
                                     "LC_ALL": "en_US.UTF-8",
                                     "PYTHONUNBUFFERED": "1"},
        }))
        r = subprocess.run(["launchctl", "bootstrap", f"gui/{os.getuid()}",
                            str(plist)], capture_output=True, text=True,
                           timeout=60)
        if r.returncode != 0:
            raise SystemExit(
                f"launchctl bootstrap gui/{os.getuid()} failed "
                f"rc={r.returncode}: {(r.stderr or r.stdout).strip()} — the "
                "user must be logged in to the Mac's GUI session")
        print(json.dumps({"run_dir": str(run_dir), "label": label}))
        return
    run_dir = Path(args.run_dir).resolve()
    label = _job_label(run_dir)
    if args.action == "status":
        st = run_dir / "status.json"
        log = run_dir / "job.log"
        printed = _launchctl_print(label)
        print(json.dumps({
            "run_dir": str(run_dir), "label": label,
            "loaded": printed is not None,
            "launchd": [ln.strip() for ln in (printed or "").splitlines()
                        if re.match(r"\s*(state|pid|last exit code) =", ln)],
            "status": json.loads(st.read_text()) if st.exists() else None,
            "log_tail": log.read_text().splitlines()[-40:]
            if log.exists() else []}, indent=1))
        return
    # stop: boot the job out — a running job gets SIGTERM and records its
    # exit; a finished one is unloaded
    if _launchctl_print(label) is None:
        raise SystemExit(f"job {label} is not loaded in gui/{os.getuid()}")
    # bootout returns once the job is gone, which may take the job's
    # whole ExitTimeOut; launchctl's own work is bounded by the second
    # term
    _out(["launchctl", "bootout", f"gui/{os.getuid()}/{label}"],
         timeout=JOB_EXIT_TIMEOUT_S + 60)
    print(f"booted out {label}")


# ---------------------------------------------------------------- report

def flatten(results: dict) -> dict:
    """(contestant, workload) -> metric -> {median,min,max,samples}."""
    import statistics
    table = {}
    for run in results["runs"]:
        if "error" in run or "metrics" not in run:
            continue
        cell = table.setdefault((run["contestant"], run["workload"]), {})
        for test, mets in run["metrics"].items():
            for tail, m in mets.items():
                c = cell.setdefault(f"{test}:{tail}",
                                    {"unit": m["unit"], "samples": []})
                c["samples"].extend(m["measurements"])
        for key in ("frames", "app_cpu_window_s", "renderserver_cpu_s",
                    "cpu_ms_per_frame", "first_paint_ms"):
            if run.get(key) is not None:
                cell.setdefault(key, {"unit": "", "samples": []})[
                    "samples"].append(run[key])
        fs = run.get("frame_stats") or {}
        for key in ("frame_ms_p50", "frame_ms_p90", "frame_ms_p99",
                    "missed_vsyncs", "fps"):
            if fs.get(key) is not None:
                cell.setdefault(key, {"unit": "", "samples": []})[
                    "samples"].append(fs[key])
    for cell in table.values():
        for m in cell.values():
            s = m["samples"]
            m["n"] = len(s)
            m["median"] = statistics.median(s)
            m["min"], m["max"] = min(s), max(s)
            m["spread"] = m["max"] - m["min"]
    return table


MIN_REPS = 5


def _run_succeeded(r: dict) -> bool:
    """A run counts toward a cell only when it produced the required
    evidence: no error, launch duration, memory and CPU metrics, and
    owned presented frames from the trace. Error rows are kept verbatim
    but never counted."""
    if "error" in r:
        return False
    m = r.get("metrics") or {}
    tails = {tail for mets in m.values() for tail, mm in mets.items()
             if mm.get("measurements")}
    return ({"duration", "physical_peak", "time"} <= tails
            and bool(r.get("frames")))


def required_cells() -> set:
    """(contestant, workload) cells a complete dataset carries: every
    contestant × every workload."""
    return {(c["id"], w) for c in MANIFEST["contestants"]
            for w in MANIFEST["workloads"]}


def completeness(results: dict, min_reps: int = MIN_REPS) -> list:
    """Gaps of the dataset: cells below the rep floor as (cell,
    successes, attempts, reason), and contestants without a measured
    package size. Missing (no attempts), all-failed and mixed-attempt
    cells under min_reps all count — retry attempts are never padded."""
    have = {}
    for r in results["runs"]:
        have.setdefault((r.get("contestant"), r.get("workload")),
                        []).append(r)
    gaps = []
    for cell in sorted(required_cells()):
        rows = have.get(cell, [])
        ok = sum(1 for r in rows if _run_succeeded(r))
        if not rows:
            gaps.append((cell, 0, 0, "missing — no attempts recorded"))
        elif not ok:
            gaps.append((cell, 0, len(rows), "all attempts failed"))
        elif ok < min_reps:
            gaps.append((cell, ok, len(rows),
                         f"{ok}/{min_reps} successful reps"))
    for c in MANIFEST["contestants"]:
        size = (results.get("sizes") or {}).get(c["id"]) or {}
        if "thinned_ipa_bytes" not in size:
            gaps.append(((c["id"], "size"), 0, 0,
                         size.get("error", "no thinned package size")))
    return gaps


def _mb(b) -> str:
    return f"{b / 1e6:.1f}" if b else "—"


def cmd_report(args):
    results = json.loads(Path(args.input).read_text())
    sanitize_runs(results)
    contestants = [c["id"] for c in MANIFEST["contestants"]]
    label = {c["id"]: c["label"] for c in MANIFEST["contestants"]}
    dev = results.get("device") or {}
    build = results.get("build") or {}
    lines = ["# Competitive benchmark — iOS (water-rs/waterui#1262)", "",
             f"Device: {dev.get('model', '?')} ({dev.get('product_type', '?')}"
             f", iOS {dev.get('os_version', '?')}). Device host: "
             f"{(results.get('machine') or {}).get('hw_model', '?')}, Xcode "
             f"{(results.get('device_host_xcode') or {}).get('version', '?')}."
             f" Build host: {(build.get('build_host') or {}).get('hw_model', '?')}"
             f", Xcode {(build.get('xcode') or {}).get('version', '?')}, "
             f"checkout `{build.get('checkout_head', '?')}`.", ""]
    for c in MANIFEST["contestants"]:
        lines.append(f"- **{c['label']}**: {c['framework_version']}")
    lines.append("")
    sizes = results.get("sizes") or {}
    lines += ["## Package size (MB)", "",
              "| size | " + " | ".join(label[c] for c in contestants) + " |",
              "|" + "---|" * (len(contestants) + 1)]
    for key, name in (("app_bytes", "unsigned .app (build host)"),
                      ("unsigned_ipa_bytes", "unsigned .ipa (build host)"),
                      ("thinned_ipa_bytes", "thinned .ipa, download"),
                      ("thinned_app_bytes", "thinned app, installed")):
        lines.append(f"| {name} | " + " | ".join(
            _mb((sizes.get(c) or {}).get(key)) for c in contestants) + " |")
    lines.append("")
    table = flatten(results)
    for w, spec in MANIFEST["workloads"].items():
        wconts = [c for c in contestants if (c, w) in table]
        if not wconts:
            continue
        lines += [f"## {w} — {spec['name']} (drive: {spec['drive']})", "",
                  "| metric — median · min..max · n | "
                  + " | ".join(label[c] for c in wconts) + " |",
                  "|" + "---|" * (len(wconts) + 1)]
        metrics = sorted({mn for c in wconts for mn in table[(c, w)]})
        wu = table.get(("waterui", w), {})
        for mn in metrics:
            row, ratio = [mn], ["× WaterUI"]
            for c in wconts:
                m = table[(c, w)].get(mn)
                row.append(f"{m['median']:.3g} {m['unit']} · "
                           f"{m['min']:.3g}..{m['max']:.3g} · n={m['n']}"
                           if m else "—")
                base = (wu.get(mn) or {}).get("median")
                ratio.append(f"{m['median'] / base:.2f}"
                             if m and base else "—")
            lines += ["| " + " | ".join(row) + " |",
                      "| " + " | ".join(ratio) + " |"]
        lines.append("")
    caps = [r for r in results["runs"]
            if (r.get("capacity") or {}).get("steps") and "error" not in r]
    if caps:
        lines += ["## Capacity (W5/W6)", "",
                  "| contestant | workload | rep | capacity@120Hz | "
                  "capacity@60Hz |", "|---|---|---|---|---|"]
        for r in caps:
            lines.append(f"| {label[r['contestant']]} | {r['workload']} | "
                         f"{r['repeat']} | {r['capacity']['capacity_120hz']} "
                         f"| {r['capacity']['capacity_60hz']} |")
        lines.append("")
        for r in caps:
            lines += [f"### {label[r['contestant']]} {r['workload']} rep "
                      f"{r['repeat']} — per step", "",
                      "| n | frames | p50 ms | p99 ms | %in 120 Hz | "
                      "%in 60 Hz | collapsed | CPU ms/frame | app CPU s | "
                      "render CPU s |", "|---|---|---|---|---|---|---|---|---|---|"]
            for st in r["capacity"]["steps"]:
                fs = st["frame_stats"]
                lines.append(
                    f"| {st['n']} | {st['frames']} | {fs['frame_ms_p50']} | "
                    f"{fs['frame_ms_p99']} | {st['in_120hz_pct']} | "
                    f"{st['in_60hz_pct']} | {st['collapsed']} | "
                    f"{st.get('cpu_ms_per_frame', '—')} | "
                    f"{st['app_cpu_window_s']} | "
                    f"{st['renderserver_cpu_s']} |")
            lines.append("")
    errs = [r for r in results["runs"] if "error" in r]
    if errs:
        lines += ["## Errors", "",
                  "| contestant | workload | repeat | error |",
                  "|---|---|---|---|"]
        for r in errs:
            e = r["error"].replace("\n", " ").replace("|", "\\|")
            lines.append(f"| {label.get(r.get('contestant'), '?')} | "
                         f"{r.get('workload')} | {r.get('repeat')} | "
                         f"{e[:300]} |")
        lines.append("")
    gaps = completeness(results)
    if gaps:
        lines += [f"## DATASET INCOMPLETE (cells need ≥{MIN_REPS} "
                  "successful reps; every contestant needs a thinned size)",
                  "", "| contestant | workload | ok/attempts | reason |",
                  "|---|---|---|---|"]
        for (cid, w), ok, att, why in gaps:
            lines.append(f"| {label[cid]} | {w} | {ok}/{att} | {why} |")
        lines.append("")
    notes = MANIFEST.get("notes") or []
    if notes:
        lines += ["## Harness notes", ""] + [f"- {n}" for n in notes] + [""]
    out = Path(args.out)
    out.write_text("\n".join(lines) + "\n")
    print(f"report → {out}")
    if gaps:
        raise SystemExit("dataset incomplete — see the DATASET INCOMPLETE "
                         "section of the report")


# ----------------------------------------------------------- attribution

def cmd_attribution(args):
    """Print, as JSON, how one all-process Animation Hitches trace from
    the device attributes frames to one contestant: its owned pids, the
    (display, swap) join, the owned present series next to every present
    on the same display, and every frame on that display classified by
    the client updates its swap carried (owned / owned and foreign /
    foreign / none) with the foreign updating processes — over the owned
    span of the whole recording (no window, no gating). It is the
    evidence that decides the attribution rule for render-server-driven
    frames. Signposts of the dev.bench subsystem are listed as found.
    Record the trace with `device-session start --keep-traces`."""
    app = Path(args.app)
    trace = Path(args.trace)
    tables = trace_tables(trace)
    att = trace_attribution(trace, app.name, bundle_executable(app), tables)
    frames, updates = tables[FRAMES_SCHEMA], tables[UPDATES_SCHEMA]
    display_all = sorted(
        t for f in frames
        if f["display"] == att["display"]
        and (t := _present_ns(f)) is not None)
    owned = att.pop("presents_ns")
    span = (owned[0] / 1e6, (owned[-1] - owned[0]) / 1e6)
    refresh_ms = 1000.0 / args.max_fps

    def stats(ns):
        st = lib_frames.frame_statistics(
            [t / 1e6 for t in ns], window_start_ms=span[0],
            capture_ms=span[1], refresh_ms=refresh_ms)
        st.pop("intervals_ms")
        return st
    marks = [{"name": x["name"], "time_ns": x["time"],
              "process": x.get("process")}
             for x in att.pop("signposts")
             if x.get("subsystem") == MARK_SUBSYSTEM]
    classified = classify_display_frames(
        frames, updates, set(att["owned_pids"]), att["display"],
        owned[0], owned[-1])
    print(json.dumps({**att, "owned_presents": len(owned),
                      "span_ms": span,
                      "owned": stats(owned),
                      "display_all": stats(display_all),
                      "frames_by_update": classified,
                      "updates_without_display": sum(
                          1 for u in updates if u["display"] is None),
                      "dev_bench_signposts": marks},
                     indent=2, default=str))


# ---------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    sub.add_parser("bootstrap").set_defaults(f=cmd_bootstrap)
    sub.add_parser("build").set_defaults(f=cmd_build)

    s = sub.add_parser("device-session",
                       help="start/inspect/stop a measurement run in the "
                            "device host's GUI session (run over ssh)")
    s.add_argument("action", choices=["start", "status", "stop"])
    s.add_argument("--stage", help="start: the build host's "
                                   "ios-stage-<head>.tar.gz")
    s.add_argument("--stage-sha256",
                   help="start: the sha256 `build` printed for the "
                        "tarball; it is verified before anything else")
    s.add_argument("--run-dir", help="status/stop: the run dir start "
                                     "printed")
    s.add_argument("--repeats", type=int, default=MIN_REPS)
    s.add_argument("--reps", default=None,
                   help="comma-separated rep indices to (re)measure "
                        "(default: --repeats reps starting at 0)")
    s.add_argument("--only", default=None,
                   help="comma-separated contestant ids (default: all)")
    s.add_argument("--workloads", default=None,
                   help="comma-separated workload ids (default: all)")
    s.add_argument("--out", default=None,
                   help="results file to create or extend (default: "
                        "<run-dir>/results-device.json)")
    s.add_argument("--keep-traces", action="store_true",
                   help="keep every cell's .trace bundles (attribution "
                        "evidence); the disk floor still gates each cell")
    s.set_defaults(f=cmd_session)

    d = sub.add_parser("device-run",
                       help="the measurement job launchd starts in the GUI "
                            "session (device-session start submits it)")
    d.add_argument("--run-dir", required=True)
    d.set_defaults(f=cmd_device_run)

    rp = sub.add_parser("report")
    rp.add_argument("--input", required=True)
    rp.add_argument("--out", default=str(ROOT / "build" / "report.md"))
    rp.set_defaults(f=cmd_report)

    at = sub.add_parser(
        "attribution",
        help="print how a recorded all-process Animation Hitches trace "
             "attributes frames to one contestant (evidence, no window)")
    at.add_argument("--trace", required=True)
    at.add_argument("--app", required=True,
                    help="the contestant's .app bundle (its name and "
                         "CFBundleExecutable select its processes)")
    at.add_argument("--max-fps", type=float, required=True,
                    help="the display's refresh rate, for missed-vsync "
                         "counts")
    at.set_defaults(f=cmd_attribution)

    args = ap.parse_args()
    if args.cmd == "device-session" and (
            (args.action == "start") != bool(args.stage)
            or (args.action == "start") != bool(args.stage_sha256)
            or (args.action != "start") != bool(args.run_dir)):
        ap.error("device-session start takes --stage and --stage-sha256; "
                 "status/stop take --run-dir")
    if (args.cmd == "device-session" and args.stage_sha256
            and not _SHA256_RE.fullmatch(args.stage_sha256)):
        ap.error("--stage-sha256 must be 64 lowercase hex digits, as "
                 "`build` prints it")
    args.f(args)


if __name__ == "__main__":
    main()

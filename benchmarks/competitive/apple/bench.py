#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyobjc-framework-Quartz==12.2.2"]
# ///
"""Competitive benchmark runner — Apple targets (water-rs/waterui#1262).

Single entry point. Run with `uv run bench.py <command>`. The inline
script metadata above is its whole environment: the standard library plus
PyObjC's Quartz bindings, which the wheel driver posts CGEvents through.

Commands:
  build    --platform ios-sim|macos|ios-device   build every contestant, stage artifacts
  run-local --platform ios-sim|macos [--repeats 5]   drive + measure on this machine
  device   --artifacts <dir> --udid <udid> [--repeats 5]   iOS device on the
            M1 host: sign (free team), install, drive, measure. Signs NOTHING here.
  report   --input results.json [--out report.md]

Every contestant is measured the same way: the shared BenchRunner XCUITest
bundle drives the app and XCTest metrics (launch, memory, CPU, hitch, scroll
signposts) are collected externally. No per-contestant measurement shortcuts.
"""
from __future__ import annotations

import sys

if sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {sys.version.split()[0]}); every leg "
        "declares its version in pyproject.toml + .python-version and "
        "runs under the uv-managed interpreter (`uv run`)")

import argparse
import fcntl
import hashlib
import json
import os
import plistlib
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "lib"))
import toolchain  # benchmarks/competitive/lib/toolchain.py
import frame_stats as lib_frames  # benchmarks/competitive/lib/frame_stats.py

ROOT = Path(__file__).resolve().parent
MANIFEST = json.loads((ROOT / "manifest.json").read_text())
LOCK_DIR = Path("/tmp/device-locks")
RESULTS_DEFAULT = ROOT / "build" / "results.json"


# ------------------------------------------------------------------ hosts

_ACTIVE_PROCS: list = []


def _kill_proc(p):
    """Kill a tracked process AND its grandchildren: every spawn is
    session-led (start_new_session), so the whole group dies — a plain
    p.kill() only takes out the /bin/sh wrapper and orphans xcodebuild/
    xctrace under it."""
    try:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL)
    except Exception:
        try:
            p.kill()
        except Exception:
            pass


def _kill_active_procs():
    """Kill every tracked child process (xcodebuild, xctrace, builds).
    Called on signals so an interrupted run never leaves an orphaned
    build/test process driving the shared machine."""
    for p in list(_ACTIVE_PROCS):
        _kill_proc(p)


def resolve_sim_udid(requested: str | None = None) -> str:
    """UDID of the iOS simulator this host builds and runs against.

    An explicit --sim-udid wins; otherwise the newest-runtime available
    iPhone from `simctl list devices available` is selected — build
    destinations and container lookups derive from the resolved device,
    never a baked-in hardware id."""
    global _SIM_UDID_CACHE
    if requested:
        _SIM_UDID_CACHE = requested
        return requested
    if _SIM_UDID_CACHE:
        return _SIM_UDID_CACHE
    r = subprocess.run(
        ["xcrun", "simctl", "list", "devices", "available", "--json"],
        capture_output=True, text=True, timeout=60)
    doc = json.loads(r.stdout)
    cands = []
    for runtime, devs in doc.get("devices", {}).items():
        m = re.search(r"iOS[- ](\d+)[-.](\d+)", runtime)
        ver = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
        for d in devs:
            if d.get("isAvailable", True) and "iPhone" in d.get("name", ""):
                cands.append((ver, d["name"], d["udid"]))
    if not cands:
        raise RuntimeError(
            "no available iPhone simulator — create one with "
            "`xcrun simctl create` or pass --sim-udid")
    cands.sort()
    ver, name, udid = cands[-1]
    _SIM_UDID_CACHE = udid
    print(f"resolved iOS simulator: {name} ({udid}, "
          f"runtime iOS {ver[0]}.{ver[1]})", flush=True)
    return udid


_SIM_UDID_CACHE = None


def sim_udid() -> str:
    return resolve_sim_udid()


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


def host_evidence() -> dict:
    """Evidence that identifies the measurement host's real hardware.

    Recorded on every results file and re-verified on resume, so rows
    from mixed hosts or a paravirtual VM never merge into one dataset."""
    def _sysctl(k):
        r = subprocess.run(["sysctl", "-n", k], capture_output=True,
                           text=True, timeout=30)
        return r.stdout.strip() if r.returncode == 0 else None

    def _ioreg_uuid():
        r = subprocess.run(
            ["ioreg", "-rd1", "-c", "IOPlatformExpertDevice"],
            capture_output=True, text=True, timeout=30)
        if r.returncode != 0:
            return None
        m = re.search(r'"IOPlatformUUID"\s*=\s*"([^"]+)"', r.stdout)
        return m.group(1) if m else None

    ev = {"hw_model": _sysctl("hw.model"),
          # hardware UUID — the machine fingerprint the manifest declares;
          # a model name is never identity (any Mac mini matches)
          "hw_uuid": _ioreg_uuid(),
          "cpu_brand": _sysctl("machdep.cpu.brand_string"),
          "hv_vmm_present": _sysctl("kern.hv_vmm_present") == "1",
          "gpus": []}
    r = subprocess.run(["system_profiler", "SPDisplaysDataType", "-json"],
                       capture_output=True, text=True, timeout=120)
    if r.returncode == 0:
        try:
            for d in json.loads(r.stdout).get("SPDisplaysDataType", []):
                name = d.get("sppci_model") or d.get("_name")
                if name:
                    ev["gpus"].append(name)
        except Exception:
            pass
    return ev


def host_is_virtualized(ev: dict) -> str | None:
    """Reason string when the evidence says VM/paravirtual, else None."""
    if ev.get("hv_vmm_present"):
        return "kern.hv_vmm_present=1 (Hypervisor.framework guest)"
    brand = f"{ev.get('cpu_brand') or ''} {ev.get('hw_model') or ''}"
    if "virtual" in brand.lower():
        return f"virtualized identity: {brand.strip()}"
    for g in ev.get("gpus", []):
        if "virtual" in g.lower() or "paravirtual" in g.lower():
            return f"paravirtual GPU: {g}"
    return None


def water_bin() -> str:
    """The `water` CLI built from THIS checkout (cli/ is a workspace
    member) — `cargo install --locked --path cli` into the suite-shared
    cache under a file lock, once per host. Refuses a dirty tracked
    checkout so the recorded HEAD sha is the real identity."""
    return str(toolchain.provision_water_cli())


def cli_evidence() -> dict:
    """Identity of the `water` binary actually invoked: path, sha256,
    --version output, plus the checkout HEAD it was built from."""
    path = water_bin()
    ev = {"binary": path}
    p = Path(path)
    if not p.exists():
        ev["error"] = "binary not found"
        return ev
    ev["sha256"] = hashlib.sha256(p.read_bytes()).hexdigest()
    r = subprocess.run([str(p), "--version"], capture_output=True,
                       text=True, timeout=30)
    ev["version"] = (r.stdout or r.stderr).strip()
    ev["checkout_head"] = toolchain.checkout_head()
    ev["source"] = "in-tree cli/ (workspace member), cargo install --locked"
    return ev

# ---------------------------------------------------------------- helpers

def sh(cmd, cwd=ROOT, env=None, check=True, capture=False, timeout=None):
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
        return None
    finally:
        if p in _ACTIVE_PROCS:
            _ACTIVE_PROCS.remove(p)
    if check and r.returncode != 0:
        tail = (r.stdout or "")[-3000:] + (r.stderr or "")[-3000:]
        raise RuntimeError(f"command failed ({r.returncode}): {cmd}\n{tail}")
    return r


def du_bytes(path: Path) -> int:
    total = 0
    for p in path.rglob("*"):
        if p.is_file() and not p.is_symlink():
            total += p.lstat().st_size
    return total


def stat_machine() -> dict:
    out = {}
    for k, cmd in {
        "model": "sysctl -n machdep.cpu.brand_string 2>/dev/null || sysctl -n hw.model",
        "os": "sw_vers -productName && sw_vers -productVersion",
        "kernel": "uname -m",
        "memory_gb": "expr $(sysctl -n hw.memsize) / 1073741824",
    }.items():
        r = sh(cmd, capture=True, check=False, timeout=30)
        out[k] = (r.stdout or "").strip().replace("\n", " ")
    out["fingerprint"] = host_evidence()
    return out


def check_host_fingerprint(state: dict, evidence: dict | None = None
                           ) -> None:
    """Refuse to merge runs taken on a different host, and refuse to
    measure at all on a virtualized host — paravirtual timings are not
    hardware evidence and there is no escape flag."""
    fp = evidence if evidence is not None else host_evidence()
    vm = host_is_virtualized(fp)
    if vm:
        raise SystemExit(
            f"refusing to measure on a virtualized host: {vm}. "
            "Measurements here would be paravirtual timings, not "
            "hardware evidence — run on the physical host.")
    prev = (state.get("machine_fingerprint") or
            state.get("machine", {}).get("fingerprint"))
    if prev and prev != fp:
        raise SystemExit(
            "results file was produced on a different host — refusing "
            "to merge mixed hardware evidence\n"
            f"  recorded: {prev}\n  current : {fp}")
    if prev is None and state.get("runs"):
        raise SystemExit(
            "results file predates host fingerprinting — stale/unknown "
            "hardware evidence cannot merge with current runs")
    state["machine"]["fingerprint"] = fp


def cli_check(ev: dict, pin: dict | None) -> str | None:
    """None when a recorded CLI-binary sha matches the fresh binary
    evidence; an error string otherwise. An absent pin is not an error
    here — files that never recorded one are refused by the
    fingerprint gates instead."""
    if not pin or pin.get("sha256") is None:
        return None
    want = pin["sha256"]
    if ev.get("error") or not ev.get("sha256"):
        return f"cannot hash the CLI binary: {ev.get('error', 'no sha256')}"
    if ev["sha256"] != want:
        return (f"CLI binary sha256 mismatch: recorded {want}, "
                f"current {ev['sha256']}")
    return None


def check_source_fingerprint(state: dict) -> None:
    """Resume gate for the framework identity: a results file made by a
    different checkout HEAD or a different water CLI binary — or one
    that predates that provenance — cannot merge with current runs.
    Call BEFORE refreshing pins."""
    declared = toolchain.checkout_head()
    pins = state.get("pins") or {}
    old_head = pins.get("waterui_head")
    if old_head and old_head != declared:
        raise SystemExit(
            "results file was produced at a different waterui checkout "
            "— refusing to merge\n"
            f"  recorded: {old_head}\n  current:   {declared}")
    if state.get("runs") and not old_head:
        raise SystemExit(
            "results file predates checkout fingerprinting — stale/"
            "unknown source provenance cannot merge with current runs")
    old_cli = pins.get("water_cli")
    if old_cli and not str(old_cli).startswith("unresolved"):
        err = cli_check(cli_evidence(), {"sha256": old_cli})
        if err:
            raise SystemExit(
                "results file was produced by a different water CLI "
                f"binary — refusing to merge\n  {err}")


def contestant(cid):
    for c in MANIFEST["contestants"]:
        if c["id"] == cid:
            return c
    raise KeyError(cid)


# ---------------------------------------------------------------- build

def cmd_bootstrap(args):
    """Fresh-clone dependency install for one platform, pinned to the
    committed lockfiles — npm ci (package-lock.json), bundle install +
    pod install (Gemfile.lock / Podfile.lock), xcodegen regen of the
    native project. Any failure aborts; there is no silent retry.

    The whole bootstrap runs on a clean tracked tree and must leave it
    clean: a step that rewrites a committed lockfile or project means the
    committed copy is stale, and the staged set would otherwise carry a
    HEAD label the tree no longer matches."""
    t = MANIFEST["toolchain"]
    with toolchain.tracked_tree_unchanged(f"apple bootstrap {args.platform}"):
        # RN root template files are generated, not committed —
        # materialize them from the pinned init before `npm ci`/`bundle`
        # reads the dir.
        rn = next((c for c in MANIFEST["contestants"] if c["id"] == "rn"),
                  None)
        if rn:
            toolchain.ensure_rn_template(
                ROOT / rn["dir"], t["react_native_cli"],
                t["react_native"], t["react_native_template_sha256"],
                env=os.environ.copy())
        cp_ver = re.match(r"[\d.]+", t["cocoapods"])
        for step in (MANIFEST.get("bootstrap") or {}).get(args.platform, []):
            match step:
                case {"step": "rn-macos-template"}:
                    # the react-native-macos template files the macos/
                    # project does not commit, from the generator `npm
                    # ci` just installed — before `pod install` reads it
                    toolchain.ensure_rn_macos_template(
                        ROOT / contestant("rn")["dir"],
                        t["react_native_macos"], env=os.environ.copy())
                case str():
                    cmd = step
                    if "{SIM_UDID}" in cmd:
                        cmd = cmd.replace("{SIM_UDID}",
                                          resolve_sim_udid(args.sim_udid))
                    if "{COCOAPODS}" in cmd:
                        if not cp_ver:
                            raise SystemExit(
                                "toolchain.cocoapods in manifest.json does "
                                "not start with a version number")
                        cmd = cmd.replace("{COCOAPODS}", cp_ver.group(0))
                    sh(cmd)
                case _:
                    raise SystemExit(
                        f"manifest bootstrap.{args.platform} has an "
                        f"unknown step: {step!r}")
    print(f"bootstrap complete for {args.platform}")


def app_binary(path: Path) -> Path | None:
    """The executable inside a .app bundle: <stem> at top level for iOS,
    Contents/MacOS/<stem> for macOS. None when either candidate is
    absent — an Xcode stub after a failed build-for-testing carries
    Info.plist + PkgInfo but no binary."""
    for cand in (path / path.stem,
                 path / "Contents" / "MacOS" / path.stem):
        if cand.is_file() and cand.stat().st_size > 0:
            return cand
    return None


def dir_sha256(path: Path) -> str:
    """Content hash of an app bundle: sorted (relpath, size, sha256)
    triples — the staging manifest records it so a later run measures
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


def flutter_bin():
    """Resolved flutter binary for this host, version-checked against the
    manifest's declared toolchain — module scope so every consumer
    (ensure_flutter_apple, build command substitution) shares it."""
    root = os.path.expandvars(
        MANIFEST["toolchain"]["flutter_root"])
    b = Path(root) / "bin" / "flutter"
    toolchain.require_version(
        "flutter", [str(b), "--version"],
        MANIFEST["toolchain"]["flutter"])
    return str(b)


def ensure_flutter_apple(d: Path, platforms: list[str], env: dict):
    """The flutter app's ios/ + macos/ directories are generated, not
    committed: produced by the pinned Flutter SDK's `flutter create` in a
    scratch dir, the authored override files then replace the template
    sources, and the iOS bundle id is rewritten to the manifest's
    dev.bench.flutter (the generator emits dev.bench.<ProjectName>, i.e.
    dev.bench.benchFlutter for project name bench_flutter).
    Reuses a stale generated dir only if .bench-generator records the same
    Flutter version — otherwise it is regenerated."""
    fb = flutter_bin()
    ver = subprocess.run([fb, "--version", "--machine"],
                         capture_output=True, text=True)
    tag = ""
    try:
        tag = json.loads(ver.stdout)["frameworkVersion"]
    except Exception:
        raise RuntimeError(
            f"flutter --version --machine unreadable: "
            f"{(ver.stdout or ver.stderr)[:200]}")
    for plat in platforms:
        dst = d / plat
        stamp = dst / ".bench-generator"
        if dst.is_dir() and stamp.exists()                 and stamp.read_text().strip() == tag:
            continue
        shutil.rmtree(dst, ignore_errors=True)
        with tempfile.TemporaryDirectory() as td:
            sh(f"'{fb}' create --platforms={plat} "
               f"--project-name bench_flutter --org dev.bench "
               f"--template app '{td}/app'", cwd=td)
            shutil.copytree(Path(td) / "app" / plat, dst)
        ovr = d / f"{plat}-override"
        if ovr.is_dir():
            for f in ovr.rglob("*"):
                if f.is_file():
                    rel = f.relative_to(ovr)
                    (dst / rel).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy(f, dst / rel)
        if plat == "ios":
            pbx = dst / "Runner.xcodeproj" / "project.pbxproj"
            pbx.write_text(pbx.read_text().replace(
                "dev.bench.benchFlutter", "dev.bench.flutter"))
        stamp.write_text(tag + "\n")


def cmd_build(args):
    plat = args.platform
    # The staged set is labelled with the checkout HEAD: the bootstrap
    # runs first, on a clean tree it must leave clean (cmd_bootstrap's
    # tracked_tree_unchanged), so a dirty checkout is refused before any
    # staged artifact is deleted.
    try:
        cmd_bootstrap(args)
    except Exception as e:
        # a failed bootstrap leaves every later build unproven — stop at
        # the first bootstrap failure
        raise SystemExit(f"bootstrap failed: {e}") from e
    staged = ROOT / "build" / "artifacts" / plat
    # stale staged artifacts are deleted wholesale, not overwritten in
    # place: a run must never measure a file left over from an earlier
    # build graph
    shutil.rmtree(staged, ignore_errors=True)
    staged.mkdir(parents=True, exist_ok=True)
    failures = {}
    # the water CLI consumed by {WATER} build commands is provisioned
    # from this checkout on first use, never a PATH entry
    _wb = {}

    def wb():
        if not _wb:
            _wb["b"] = water_bin()
        return _wb["b"]

    for c in MANIFEST["contestants"]:
        if c["id"] == "flutter" and not c.get("build", {}).get(plat):
            continue
        # a contestant build must leave the tracked tree as committed
        with toolchain.tracked_tree_unchanged(f"apple build {plat} {c['id']}"):
            if c["id"] == "flutter":
                ensure_flutter_apple(ROOT / c["dir"],
                                     ["macos" if plat == "macos" else "ios"],
                                     os.environ.copy())
            cmds = c.get("build", {}).get(plat)
            if cmds:
                for cmd in cmds:
                    if "{SIM_UDID}" in cmd:
                        cmd = cmd.replace(
                            "{SIM_UDID}", resolve_sim_udid(args.sim_udid))
                    if "{WATER}" in cmd:
                        cmd = cmd.replace("{WATER}", wb())
                    if "{FLUTTER}" in cmd:
                        cmd = cmd.replace("{FLUTTER}", flutter_bin())
                    try:
                        sh(cmd)
                    except Exception as e:  # report exact error, don't drop silently
                        failures[c["id"]] = str(e)[-2000:]
                        break
        art = c.get("artifact", {}).get(plat)
        if c["id"] not in failures and art:
            src = ROOT / art
            if not src.exists():
                failures[c["id"]] = f"build ok but artifact missing: {art}"
                continue
            dst = staged / src.name
            shutil.rmtree(dst, ignore_errors=True)
            shutil.copytree(src, dst, symlinks=True)
            if src.name.endswith(".app") and not app_binary(dst):
                failures[c["id"]] = (
                    f"staged artifact {art} carries no executable — the "
                    "build left an empty Xcode stub, not a success")
                shutil.rmtree(dst, ignore_errors=True)
                continue
            if plat == "ios-device":
                # water package aborts at signing before embedding the rust
                # cdylib; every missing @rpath dep must resolve inside the
                # package output — an unresolved dep is a build failure,
                # never a warning that ships a dead binary
                exe = dst / dst.stem
                if exe.exists():
                    out = subprocess.run(["otool", "-L", str(exe)],
                                         capture_output=True, text=True).stdout
                    for line in out.splitlines()[1:]:
                        name = line.strip().split(" ")[0]
                        if not name.startswith("@rpath/"):
                            continue
                        lib = name[len("@rpath/"):]
                        dd = dst / "Frameworks" / lib
                        if dd.exists():
                            continue
                        search = [src.parent, src.parent / "Frameworks",
                                  src / "Frameworks"]
                        for cand in (p / lib for p in search):
                            if cand.exists():
                                dd.parent.mkdir(exist_ok=True)
                                shutil.copy2(cand, dd)
                                print(f"  staged nested dylib {lib}")
                                break
                        else:
                            failures[c["id"]] = (
                                f"unresolved @rpath dep {name} — "
                                "the packaged artifact does not carry it")
                            break
                    if c["id"] in failures:
                        continue
            print(f"staged {c['id']}: {dst} ({du_bytes(dst)/1e6:.1f} MB)")
    # stage the test-runner products next to the artifacts
    if plat in ("ios-sim", "ios-device", "macos"):
        key = {"ios-sim": "ios_sim_xctestrun", "ios-device": "ios_device_xctestrun",
               "macos": "macos_xctestrun"}[plat]
        tmpl = ROOT / MANIFEST["harness"][key]
        if tmpl.exists():
            dst = staged / tmpl.name
            shutil.rmtree(dst, ignore_errors=True)
            shutil.copy2(tmpl, dst)
            products = tmpl.parent
            for sub in products.iterdir():
                if not (sub.is_dir() and sub.name.startswith("Release")):
                    continue
                for p in sub.iterdir():
                    if p.name.endswith(("-Runner.app", ".xctest")):
                        dd = staged / p.name
                        shutil.rmtree(dd, ignore_errors=True)
                        if p.is_dir():
                            shutil.copytree(p, dd, symlinks=True)
                        else:
                            shutil.copy2(p, dd)
    if failures:
        print(json.dumps({"build_failures": failures}, indent=2))
        sys.exit(2)
    # staging manifest: which artifacts (and which source identity and
    # build host) produced this staged set — the run verifies each
    # artifact's hash against it before measuring, and records it as
    # per-row provenance
    staging = {"checkout_head": toolchain.checkout_head(),
               "built_utc": time.time(),
               "host_fingerprint": host_evidence(),
               "artifacts": {p.name: dir_sha256(p)
                             for p in sorted(staged.iterdir())
                             if p.is_dir() and p.suffix == ".app"}}
    (staged / "staging-manifest.json").write_text(
        json.dumps(staging, indent=1) + "\n")
    print("all contestants built and staged")


# ------------------------------------------------------- xctestrun surgery

def write_xctestrun(template: Path, out: Path, target_key: str,
                    products_subdir: str, app_name: str, bundle_id: str,
                    workload: str, drive: str, duration: int, runner_app: str,
                    nonce: int, no_hitch: bool = False,
                    only_test: str | None = None, step: int | None = None):
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
    warmup_ms = MANIFEST["harness"]["warmup_ms"]
    if warmup_ms <= 0:
        raise RuntimeError(
            "harness.warmup_ms must be > 0 — the warmup is declared in "
            "the manifest and is never zero (METHOD)")
    tolerance_ms = MANIFEST["harness"]["anchor_tolerance_ms"]
    if tolerance_ms <= 0:
        raise RuntimeError(
            "harness.anchor_tolerance_ms must be > 0 — the runner holds "
            "the contestant this long past the capture so the trace's "
            "window ends inside the held span")
    env.update({
        "BENCH_BUNDLE_ID": bundle_id,
        "BENCH_WORKLOAD": workload,
        "BENCH_DRIVE": drive,
        "BENCH_DURATION": str(duration),
        "BENCH_WARMUP_MS": str(warmup_ms),
        "BENCH_ANCHOR_TOLERANCE_MS": str(tolerance_ms),
        # the recorder-go latch this invocation must match — a latched
        # signal left by another invocation never releases this one
        "BENCH_RUN_NONCE": str(nonce),
        # the fling program the XCTest's swipe branch executes — from the
        # manifest, never Swift literals
        "BENCH_FLING": json.dumps({
            "flings_down": int(MANIFEST["harness"]["fling"]["down"]),
            "flings_up": int(MANIFEST["harness"]["fling"]["up"]),
            "start_fraction":
                float(MANIFEST["harness"]["fling"]["swipe_start_fraction"]),
            "end_fraction":
                float(MANIFEST["harness"]["fling"]["swipe_end_fraction"]),
            "duration_ms":
                float(MANIFEST["harness"]["fling"]["swipe_duration_ms"]),
            "pause_s":
                float(MANIFEST["harness"]["fling"]["swipe_pause_s"]),
            "hold_s":
                float(MANIFEST["harness"]["fling"]["swipe_hold_s"]),
        }),
    })
    if step is not None:
        env["BENCH_STEP"] = str(step)
    if no_hitch:
        # deterministic per-platform attachment (ios-sim has no GPU frame
        # telemetry) — the row records the hitch metric as not collected
        env["BENCH_NO_HITCH"] = "1"
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
        ["xcrun", "xcresulttool", "get", "test-results", "metrics", "--path", str(path)],
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


# Per-workload default drive when the manifest has no override: the
# scroll workloads are fling-driven; w1 taps the counter; w3 and w5 are
# self-animating (no drive).
_DEFAULT_DRIVE = {"w1": "tap", "w3": "none", "w5": "none"}


def drive_for(c: dict, plat: str, workload: str = "") -> str:
    """One drive per contestant per platform, recorded per row.
       Resolution order: contestant's own `drive` override (manifest),
       then the platform-level `drive_overrides` for this workload, then
       the shared `*` overrides, then `swipe`. There is no `auto` drive —
       the app never scrolls itself and never paces a ladder."""
    if (d := c.get("drive", {}).get(plat)) is not None:
        if isinstance(d, dict):
            return d.get(workload, d.get("*", "swipe"))
        return d
    ov = MANIFEST.get("drive_overrides", {})
    pov = {**ov.get("*", {}), **ov.get(plat, {})}
    pov.pop("note", None)
    return pov.get(workload, pov.get("*",
        _DEFAULT_DRIVE.get(workload, "swipe")))


def sanitize_runs(state: dict) -> None:
    """Drop rows from superseded harness generations. Rows with no
    `drive` key predate per-row drive recording; rows whose drive
    disagrees with the manifest's current drive_for() decision are stale
    duplicates the (plat, contestant, workload, drive) prune key leaves
    behind whenever the drive convention changes."""
    by_id = {c["id"]: c for c in MANIFEST["contestants"]}
    kept = []
    for x in state.get("runs", []):
        plat, cid, w, dr = (x.get("platform"), x.get("contestant"),
                            x.get("workload"), x.get("drive"))
        if "error" in x:
            # a failed attempt is evidence — sanitize drops superseded
            # harness rows, never error records
            kept.append(x)
            continue
        if dr is None or cid not in by_id or w not in MANIFEST["workloads"]:
            continue
        if dr != drive_for(by_id[cid], plat, w):
            continue
        kept.append(x)
    state["runs"] = kept


def _ps_time_to_s(v: str) -> float:
    """ps -o time value: [[dd-]hh:]mm:ss(.cc) → seconds."""
    neg = v.startswith("-")
    v = v.lstrip("-")
    days = 0.0
    if "-" in v:
        d, v = v.split("-", 1)
        days = float(d) * 86400
    parts = v.split(":")
    try:
        if len(parts) == 3:
            s = float(parts[0]) * 3600 + float(parts[1]) * 60 + float(parts[2])
        elif len(parts) == 2:
            s = float(parts[0]) * 60 + float(parts[1])
        else:
            s = float(parts[0])
    except ValueError:
        return 0.0
    return days + (-s if neg else s)


def _sim_launchctl_pid(udid: str, name: str) -> int | None:
    """PID of a process inside a booted simulator via launchctl list."""
    r = subprocess.run(
        ["xcrun", "simctl", "spawn", udid, "launchctl", "list"],
        capture_output=True, text=True)
    for line in r.stdout.splitlines():
        p = line.split()
        # apps register as UIKitApplication:<bid>[hex][...]; daemons use
        # their own label (e.g. com.apple.backboardd).
        if len(p) >= 3 and p[0].isdigit() and (
                p[2] == name or p[2].startswith(f"UIKitApplication:{name}")):
            return int(p[0])
    return None


def _host_pid(name: str) -> int | None:
    r = subprocess.run(["pgrep", "-x", name], capture_output=True, text=True)
    for tok in r.stdout.split():
        if tok.isdigit():
            return int(tok)
    return None


def _host_pid_path(exe_path: str) -> int | None:
    """Pid of the process running exactly `exe_path` — the app binary
    this run launched, not any process sharing its name. `ps comm` is
    the executable path itself, so an exact string equality filters out
    both name collisions and cmdline-substring matches."""
    out = subprocess.run(["ps", "-axo", "pid=,comm="],
                         capture_output=True, text=True).stdout
    for line in out.splitlines():
        p = line.strip().split(None, 1)
        if len(p) == 2 and p[0].isdigit() and p[1].strip() == exe_path:
            return int(p[0])
    return None


def _children_of(pid: int) -> list[int]:
    """Direct children of a host pid (Electron's renderer/GPU helpers
    live here — its main process alone is not the memory footprint)."""
    out = subprocess.run(["ps", "-axo", "pid=,ppid="],
                         capture_output=True, text=True).stdout
    kids = []
    for line in out.splitlines():
        p = line.strip().split()
        if len(p) == 2 and p[0].isdigit() and p[1].isdigit() \
                and int(p[1]) == pid:
            kids.append(int(p[0]))
    return kids


def cpu_pids_resolver(plat: str, udid: str, bundle_id: str, exe: str):
    """Return a () -> {label: [pids]} lookup for a platform.

    The render server is the process Core Animation hands committed work
    to — backboardd inside the sim's runtime / WindowServer on macOS —
    so W3-type animations that run there cost every contestant the same
    way. `ps` polling needs no entitlements or attach consent. On macOS
    the app's helper children (renderer/GPU/utility processes — exactly
    how Electron's footprint splits) are summed as `helpers`.
    App resolution retries: the runner launches the app mid-test."""
    if plat == "ios-sim":
        def res():
            return {
                "app": [p] if (p := _sim_launchctl_pid(udid, bundle_id))
                else [],
                "render_server":
                [p] if (p := _sim_launchctl_pid(udid, "com.apple.backboardd"))
                else [],
            }
        return res
    if plat == "macos":
        def res():
            # resolve by the launched executable path, not a name — a
            # pid found by name could be a contestant from another run
            app = _host_pid_path(exe)
            return {
                "app": [app] if app else [],
                "render_server":
                [p] if (p := _host_pid("WindowServer")) else [],
                "helpers": _children_of(app) if app else [],
            }
        return res
    return lambda: {"app": [], "render_server": [], "helpers": []}


RENDER_NAME = {"ios-sim": "backboardd", "macos": "WindowServer",
               "ios-device": "backboardd"}


class CpuSampler(threading.Thread):
    """Poll `ps -o pid,time,rss` for {label: [pids]} every interval.

    Each series row carries the pid SET it was sampled from: a label is
    re-resolved every tick (an app killed between invocations gets a new
    pid), and `delta` across a pid-set change returns None instead of
    subtracting the cumulative CPU of two different processes. Memory
    (rss kB summed per label) rides the same samples."""

    def __init__(self, resolver, interval=0.5):
        super().__init__(daemon=True)
        self.resolver = resolver
        self.interval = interval
        self.pids: dict[str, list[int]] = {}
        # (epoch, {label: {"pids": [...], "cpu_s": float, "rss_kb": int}})
        self.series: list[tuple[float, dict]] = []
        self._stop = threading.Event()

    def run(self):
        while not self._stop.is_set():
            t = time.time()
            try:
                for k, v in (self.resolver() or {}).items():
                    cur = sorted(v or [])
                    if cur and self.pids.get(k) != cur:
                        # re-latch on a changed pid set — the row records
                        # which processes the numbers belong to
                        self.pids[k] = cur
            except Exception:
                pass
            sel = {k: v for k, v in self.pids.items() if v}
            if sel:
                all_pids = [p for vs in sel.values() for p in vs]
                out = subprocess.run(
                    ["ps", "-p", ",".join(map(str, all_pids)),
                     "-o", "pid=,time=,rss="],
                    capture_output=True, text=True).stdout
                by_pid = {}
                for line in out.splitlines():
                    p = line.strip().split()
                    if len(p) == 3 and p[0].isdigit():
                        by_pid[int(p[0])] = (_ps_time_to_s(p[1]),
                                             float(p[2]))
                row = {}
                for k, v in sel.items():
                    hit = [by_pid[p] for p in v if p in by_pid]
                    row[k] = {
                        "pids": list(v),
                        "cpu_s": sum(h[0] for h in hit),
                        "rss_kb": sum(h[1] for h in hit),
                    }
                self.series.append((t, row))
            self._stop.wait(self.interval)

    def stop(self):
        self._stop.set()

    def _nearest(self, label: str, t: float):
        vals = [(ts, d.get(label)) for ts, d in self.series
                if d.get(label) is not None]
        if not vals:
            return None
        return min(vals, key=lambda x: abs(x[0] - t))[1]

    def delta(self, label: str, t0: float | None = None,
              t1: float | None = None):
        """CPU seconds for `label` between t0 and t1 — None when the two
        window ends were sampled from different process sets (an app
        relaunch inside the window makes a cumulative difference
        meaningless)."""
        if t0 is None or t1 is None:
            vals = [d.get(label) for _, d in self.series
                    if d.get(label) is not None]
            if len(vals) < 2 or vals[-1]["pids"] != vals[0]["pids"]:
                return None
            return round(vals[-1]["cpu_s"] - vals[0]["cpu_s"], 3)
        a, b = self._nearest(label, t0), self._nearest(label, t1)
        if (a is None or b is None or a["pids"] != b["pids"]
                or b["cpu_s"] < a["cpu_s"]):
            return None
        return round(b["cpu_s"] - a["cpu_s"], 3)

    def peak_rss_mb(self, label: str, t0: float | None = None,
                    t1: float | None = None):
        """Peak summed RSS (MB) for `label` inside [t0, t1]."""
        vals = [d[label]["rss_kb"] for ts, d in self.series
                if d.get(label) is not None
                and (t0 is None or ts >= t0)
                and (t1 is None or ts <= t1)]
        if not vals:
            return None
        return round(max(vals) / 1024.0, 1)


CAPACITY_WORKLOADS = ("w5", "w6")
# w5/w6 cells exist only in the five iOS contestants (manifest declares
# the platform+contestant sets per workload).
CAPACITY_CONTESTANTS = {"waterui", "swiftui", "uikit", "flutter", "rn"}


def capacity_steps(workload: str) -> list[int]:
    """Ladder for a capacity workload, from the manifest — a missing or
    empty list fails instead of guessing a default."""
    steps = MANIFEST["workloads"][workload].get("steps")
    if not steps:
        raise SystemExit(f"manifest workloads.{workload}.steps missing")
    return [int(n) for n in steps]


def capacity_cell(plat, cid, w) -> bool:
    return (w in CAPACITY_WORKLOADS and cid in CAPACITY_CONTESTANTS
            and plat in ("ios-sim", "ios-device"))


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
# The column mnemonics are fixed here; scratchpad recipe
# s8-apple-trace-discovery.sh prints every schema the template records
# so they are confirmed against the pinned Xcode.

FRAMES_SCHEMA = "hitches-frame-lifetimes"
UPDATES_SCHEMA = "hitches-updates"
SIGNPOST_SCHEMA = "os-signpost"
FRAME_COLS = ("start", "duration", "swap-id", "display")
UPDATE_COLS = ("process", "swap-id")
SIGNPOST_COLS = ("time", "name", "subsystem")
# The runner's own marks, emitted as Points of Interest events: the
# drive starts at drive-begin; measure-end closes the held span.
MARK_SUBSYSTEM = "dev.bench"
MARK_DRIVE_BEGIN = "drive-begin"
MARK_MEASURE_END = "measure-end"
# recorded with the frames recorder so the marks share the trace clock
FRAMES_TEMPLATE = "Animation Hitches"
FRAMES_INSTRUMENTS = ("Points of Interest",)


class TraceAttributionError(RuntimeError):
    """The trace cannot say which frames are the contestant's, or the
    window the METHOD defines is not covered by what the runner held."""


def _export_table(trace: Path, schema: str):
    """`xctrace export` rows of every table with `schema` → (rows, err).

    Row children carry the column's ENGINEERING TYPE as their tag and
    appear in schema column order (an empty cell is a `<sentinel/>`), so
    each row is zipped against the `<schema><col><mnemonic>` list of
    the node it belongs to. Elements either define a value (`id` attr,
    raw text or `fmt`) or repeat one (`ref` attr pointing at an earlier
    `id` of the same tag) — refs are resolved so every row is
    self-contained."""
    r = _xctrace(["export", "--input", str(trace), "--xpath",
                  f'/trace-toc/run[@number="1"]/data/table[@schema="{schema}"]'],
                 timeout=600)
    if r.returncode != 0:
        return None, (r.stderr or "")[-400:]
    return parse_export_rows(r.stdout)


def parse_export_rows(xml_text: str):
    """Pure parse of an `xctrace export` table document → (rows, err).

    The document holds one `<node>` per matching table; each node
    carries its own `<schema>`, and its rows zip against that schema."""
    import xml.etree.ElementTree as ET
    root = ET.fromstring(xml_text)
    nodes = root.findall(".//node")
    if not nodes:
        return None, "export carries no <node>"
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
    rows = []
    for node in nodes:
        schema = node.find("schema")
        if schema is None:
            return None, "export node carries no <schema>"
        cols = [c.findtext("mnemonic") for c in schema.findall("col")]
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
    bundle (`…/<bundle_name>/…`): the app itself plus helpers it ships
    (Electron's GPU/renderer processes live in Contents/Frameworks).
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


def owned_presents(frames: list[dict], updates: list[dict],
                   owned: set[int]) -> dict:
    """Join frame lifetimes to the contestant's client updates by swap.

    A frame lifetime ends at its presentation (start + duration); an
    open lifetime (no duration) never presented and is not a present.
    Returns {presents_ns (sorted), display, owned_swaps, frames_total,
    updates_owned}. Raises when nothing joins or the owned frames span
    more than one display — swap ids are not proven unique across
    displays, so a multi-display join is not attribution."""
    _require_cols(frames, FRAME_COLS, FRAMES_SCHEMA)
    _require_cols(updates, UPDATE_COLS, UPDATES_SCHEMA)
    swaps = set()
    n_upd = 0
    for u in updates:
        pid = _pid_of_process(u["process"])
        if pid in owned and u["swap-id"] is not None:
            swaps.add(_int(u["swap-id"], f"{UPDATES_SCHEMA} swap-id"))
            n_upd += 1
    if not swaps:
        raise TraceAttributionError(
            f"no {UPDATES_SCHEMA} row from owned pids {sorted(owned)} "
            f"carries a swap ({len(updates)} updates in the trace)")
    presents, displays = [], set()
    for f in frames:
        if f["swap-id"] is None or f["duration"] is None:
            continue
        if _int(f["swap-id"], f"{FRAMES_SCHEMA} swap-id") not in swaps:
            continue
        presents.append(_int(f["start"], f"{FRAMES_SCHEMA} start")
                        + _int(f["duration"], f"{FRAMES_SCHEMA} duration"))
        displays.add(f["display"])
    if not presents:
        raise TraceAttributionError(
            f"no {FRAMES_SCHEMA} row presented a swap carrying an owned "
            f"update ({len(swaps)} owned swaps, {len(frames)} frames)")
    if len(displays) != 1:
        raise TraceAttributionError(
            f"owned frames span displays {sorted(map(str, displays))} — "
            "swap ids are joined per trace, so the measured contestant "
            "must present on one display")
    return {"presents_ns": sorted(presents), "display": displays.pop(),
            "owned_swaps": len(swaps), "frames_total": len(frames),
            "updates_owned": n_upd}


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
                       tolerance_ms: float) -> dict:
    """METHOD's window on the trace clock: [first owned present +
    warmup, + capture]. The runner starts the drive at its own
    readiness + warmup; that start must sit within `tolerance_ms` of the
    window start (METHOD: the drive starts at window start), and the
    runner must have held the contestant through the window end.
    Returns milliseconds on the trace clock."""
    first = presents_ns[0] / 1e6
    w0 = first + warmup_ms
    w1 = w0 + capture_ms
    drive = marks[MARK_DRIVE_BEGIN] / 1e6
    held = marks[MARK_MEASURE_END] / 1e6
    offset = drive - w0
    if abs(offset) > tolerance_ms:
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


def trace_attribution(trace: Path, bundle_name: str,
                      main_rel: str) -> dict:
    """Read one all-process trace: owned pids from its TOC, owned
    presents by the swap join, the runner marks. No window gating — the
    `attribution` command prints this as evidence."""
    main_pid, owned = owned_processes(_trace_toc(trace), bundle_name,
                                      main_rel)
    tables = {}
    for schema in (FRAMES_SCHEMA, UPDATES_SCHEMA, SIGNPOST_SCHEMA):
        rows, err = _export_table(trace, schema)
        if rows is None:
            raise TraceAttributionError(f"{schema}: {err}")
        tables[schema] = rows
    joined = owned_presents(tables[FRAMES_SCHEMA], tables[UPDATES_SCHEMA],
                            owned)
    return {"main_pid": main_pid, "owned_pids": sorted(owned),
            "signposts": tables[SIGNPOST_SCHEMA], **joined}


def trace_frame_window(trace: Path, bundle_name: str, main_rel: str,
                       capture_ms: float, refresh_ms: float) -> dict:
    """Frame statistics of the contestant's own presents over METHOD's
    window, plus the window itself (trace ms) and the attribution
    evidence. Raises TraceAttributionError when either is unsound."""
    att = trace_attribution(trace, bundle_name, main_rel)
    marks = runner_marks(att.pop("signposts"))
    h = MANIFEST["harness"]
    win = measurement_window(att["presents_ns"], marks,
                             warmup_ms=float(h["warmup_ms"]),
                             capture_ms=capture_ms,
                             tolerance_ms=float(h["anchor_tolerance_ms"]))
    stats = lib_frames.frame_statistics(
        [t / 1e6 for t in att.pop("presents_ns")],
        window_start_ms=win["window_start_ms"], capture_ms=capture_ms,
        refresh_ms=refresh_ms)
    return {"frame_stats": stats, "window": win, "attribution": att}


def _proc_name(fmt: str | None):
    """Process name from an export `process` cell ("Name (pid)")."""
    m = re.match(r"^(.*) \((\d+)\)$", (fmt or "").strip())
    return m.group(1) if m else None


def _pid_of_thread(fmt: str) -> int | None:
    """Owning pid from an export `thread` cell ("… (name, pid: N)")."""
    m = re.findall(r"\(([^()]*), pid: (\d+)\)", fmt or "")
    return int(m[-1][1]) if m else None


_PAINT_RE = re.compile(r"waterui_first_paint_ms=\s*([0-9]+(?:\.[0-9]+)?)")


def first_paint_ms(plat: str, udid: str | None,
                   trace: Path | None = None,
                   since: float | None = None,
                   proc: str | None = None) -> float | None:
    """Latest `waterui_first_paint_ms=N` the app emitted (os_log
    `dev.waterui`, notice level → persisted). Emitted once per process
    by the apple backend's WuiLaunchTiming.

    - macos: host unified log.
    - ios-sim: `simctl spawn <udid> log show` inside the simulator.
    - ios-device: devicectl exposes no console read, so the cell's own
      all-process Logging-template xctrace is the source: its os-log
      rows are selected by `proc` (the contestant's executable name).

    `since` (epoch seconds) bounds the search to this rep: without it a
    `--last 5m` window can return a PREVIOUS rep's marker — the newest
    hit is then stale evidence. Log lines carry a compact timestamp
    prefix ("YYYY-MM-DD HH:MM:SS.mmm"); rows before `since` are skipped.
    """
    def _log_epoch(line: str) -> float | None:
        m = re.match(r"^(\d{4}-\d{2}-\d{2}) (\d{2}):(\d{2}):(\d{2})",
                     line)
        if not m:
            return None
        try:
            import datetime as _dt
            return _dt.datetime.strptime(
                f"{m.group(1)} {m.group(2)}:{m.group(3)}:{m.group(4)}",
                "%Y-%m-%d %H:%M:%S").timestamp()
        except ValueError:
            return None
    cmd = None
    if plat == "macos":
        cmd = ["log", "show", "--last", "5m", "--style", "compact",
               "--predicate", 'subsystem == "dev.waterui"']
    elif plat == "ios-sim":
        cmd = ["xcrun", "simctl", "spawn", str(udid), "log", "show",
               "--last", "5m", "--style", "compact",
               "--predicate", 'subsystem == "dev.waterui"']
    if cmd is not None:
        try:
            out = subprocess.run(cmd, capture_output=True, text=True,
                                 timeout=180).stdout
        except Exception:
            return None
        vals = []
        for line in out.splitlines():
            m = _PAINT_RE.search(line)
            if not m:
                continue
            if since is not None:
                t = _log_epoch(line)
                if t is None or t < since:
                    continue
            vals.append(float(m.group(1)))
        return vals[-1] if vals else None
    if trace is not None and trace.exists():
        rows, _ = _export_table(trace, "os-log")
        for row in rows or []:
            if _proc_name(row.get("process")) != proc:
                continue
            m = _PAINT_RE.search(str(row.get("message") or ""))
            if m:
                return float(m.group(1))
    return None


def collect_pins() -> dict:
    """Exact commit pins behind a run — the apple-backend#281
    Swift→objc2 migration baseline consumes these. The CLI pin is the
    executed binary's own sha256, not a checkout HEAD or PATH guess."""
    def _git(path, *a):
        r = subprocess.run(["git", "-C", str(path), *a],
                           capture_output=True, text=True, timeout=30)
        return r.stdout.strip() if r.returncode == 0 else None

    out = {}
    head = toolchain.checkout_head()
    if head:
        # one commit pins framework, CLI, apple backend and hydrolysis —
        # the in-tree model's whole identity
        out["waterui_head"] = head
    # the apple backend revision is the scaffolded project's backend pin
    # once it exists; the checkout pin remains the primary identity
    pbx = (ROOT / "native-bench" / "NativeBench.xcodeproj"
           / "project.pbxproj")
    for cand in (pbx,
                 (ROOT.parent / "apps" / "waterui" / "backends" / "apple"
                  / "Waterui_bench.xcodeproj" / "project.pbxproj")):
        if cand.exists():
            m = re.search(r'revision = "([0-9a-f]{7,40})"',
                          cand.read_text())
            if m:
                out["apple_backend_revision"] = m.group(1)
            break
    cli = cli_evidence()
    out["water_cli"] = cli.get("sha256") or f"unresolved: {cli.get('error')}"
    out["water_cli_binary"] = cli.get("binary")
    out["water_cli_version"] = cli.get("version", "")
    out["water_cli_source"] = cli.get("source")
    # observed tool versions on this host — evidence, distinct from the
    # manifest's declared requirements (which are enforced at build)
    probes = {}
    fl_root = os.path.expandvars(
        MANIFEST["toolchain"]["flutter_root"])
    for name, cmd in {
        "xcodebuild": ["xcodebuild", "-version"],
        "flutter": [fl_root + "/bin/flutter", "--version"],
        "node": ["node", "--version"],
    }.items():
        try:
            r = subprocess.run(cmd, capture_output=True, text=True,
                               timeout=30)
            line = (r.stdout or r.stderr or "").splitlines()
            probes[name] = (line[0].strip() if line else
                            f"rc={r.returncode}")
        except Exception as e:
            probes[name] = f"unavailable: {e.__class__.__name__}"
    out["observed_toolchain"] = probes
    return out


def _spawn_pty(cmd: list[str]):
    """Spawn `cmd` session-led and tracked, its stdout+stderr on a
    pseudo-terminal: a tty line-buffers the child's stdio, so a status
    line it prints is readable the moment it is printed rather than when
    the process exits. Returns (proc, line reader)."""
    import pty
    master, slave = pty.openpty()
    try:
        p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=slave,
                             stderr=slave, start_new_session=True)
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


class XctraceRecorder:
    """One `xctrace record --all-processes` session.

    Recording every process means nothing has to exist when the recorder
    arms: it is armed BEFORE the runner's recorder-go is released (so it
    covers the contestant from its launch), and the contestant's rows
    are selected at export by process. Arming is xctrace's own
    "recording started" line, read from its tty — no attach retry and no
    sleep standing in for readiness."""

    ARMED = "Ctrl-C to stop the recording"

    def __init__(self, out: Path, template: str, device_udid: str | None,
                 time_limit_s: int, instruments: tuple[str, ...] = ()):
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
        self.proc, self.lines = _spawn_pty(cmd)
        self._drain = None
        self._result = None

    def arm(self, bound_s: float = 120.0):
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

    def stop(self, bound_s: float = 300.0) -> str | None:
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
            self._drain.join(timeout=10)
        self.lines.close()
        err = None
        if self.proc.returncode != 0 or not self.out.exists():
            err = (f"xctrace {self.template} rc={self.proc.returncode}: "
                   f"{self.log[-5:]}")
        self._result = (err,)
        return err


def _notify_cmd(plat: str, udid: str | None, *args: str) -> list[str]:
    """notifyutil in the notify namespace the runner and the contestant
    use: the simulator's own notifyd for ios-sim, the host's for macOS.
    ios-device has no host-reachable namespace."""
    if plat == "ios-sim":
        return ["xcrun", "simctl", "spawn", str(udid), "notifyutil", *args]
    if plat == "macos":
        return ["notifyutil", *args]
    raise ValueError(f"no host notify channel into {plat}")


class RecorderGo:
    """Host side of the runner's recorder-go gate, latched so neither
    side can miss the other:

    - macOS / ios-sim: a holder notifyutil registers `dev.bench.recorder`
      for exactly two notifications, sets its notify state to this
      invocation's nonce, then posts it (notifyutil runs its commands
      left to right) — its own post is the first of the two, so it stays
      registered and the state lives — until close() posts the second,
      on which the holder exits by itself. The holder's lifetime is
      therefore owned inside the notify namespace it runs in: killing
      the host-side `simctl spawn` would leave the notifyutil it started
      inside the simulator registered. The runner arms its own
      registration, then reads the state: a post after the read wakes
      it, one before it shows in the state.
    - ios-device: the host cannot reach the device's notify namespace,
      so `bench-recorder-go-<nonce>` is copied into the runner's tmp;
      the file persists and the runner watches the directory.

    The nonce rides into the runner through BENCH_RUN_NONCE, so a latch
    left by another invocation can never release this one."""

    NAME = "dev.bench.recorder"

    def __init__(self, plat: str, udid: str | None,
                 runner_bid: str | None, results_dir: Path):
        import secrets
        self.plat = plat
        self.udid = udid
        self.runner_bid = runner_bid
        self.results_dir = results_dir
        self.nonce = secrets.randbits(63) | 1
        self._holder = None

    def release(self):
        if self.plat == "ios-device":
            if not self.runner_bid:
                raise RuntimeError("ios-device recorder-go needs the "
                                   "runner bundle id")
            sentinel = self.results_dir / f"bench-recorder-go-{self.nonce}"
            sentinel.write_text("armed\n")
            try:
                if sh(f"xcrun devicectl device copy to --device {self.udid} "
                      f"--domain-type appDataContainer "
                      f"--domain-identifier {self.runner_bid} "
                      f"--source '{sentinel}' "
                      f"--destination 'tmp/{sentinel.name}'",
                      capture=True, timeout=120) is None:
                    raise RuntimeError("devicectl copy of the recorder-go "
                                       "sentinel timed out after 120s")
            finally:
                sentinel.unlink(missing_ok=True)
            return
        self._holder = subprocess.Popen(
            _notify_cmd(self.plat, self.udid, "-2", self.NAME,
                        "-s", self.NAME, str(self.nonce), "-p", self.NAME),
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        _ACTIVE_PROCS.append(self._holder)

    def close(self, bound_s: float = 60.0) -> str | None:
        """Release the holder with its second notification and wait for
        it to exit on its own; an error string when it does not."""
        if self._holder is None:
            return None
        holder, self._holder = self._holder, None
        err = None
        try:
            subprocess.run(_notify_cmd(self.plat, self.udid, "-p",
                                       self.NAME),
                           capture_output=True, check=True, timeout=60)
            holder.wait(timeout=bound_s)
        except (subprocess.SubprocessError, OSError) as e:
            err = (f"recorder-go holder (pid {holder.pid}) did not exit on "
                   f"its release post: {e}")
            _kill_proc(holder)
            holder.wait()
        finally:
            if holder in _ACTIVE_PROCS:
                _ACTIVE_PROCS.remove(holder)
        return err


def pull_runner_log(plat: str, udid: str, runner_bid: str | None,
                    out: Path):
    """Fetch the runner's tmp/bench-runner.log — it carries the
    measure-window markers, the per-row device record (thermalState,
    maximumFramesPerSecond read inside the runner process) and the
    ready/drive diagnostics."""
    if plat == "ios-sim":
        for bid in [b for b in (runner_bid, "dev.bench.runner",
                                "dev.bench.runner.xctrunner") if b]:
            r = subprocess.run(
                ["xcrun", "simctl", "get_app_container", udid, bid,
                 "data"], capture_output=True, text=True, timeout=60)
            if r.returncode == 0:
                src = Path(r.stdout.strip()) / "tmp" / "bench-runner.log"
                if src.exists():
                    shutil.copy2(src, out)
                    return
    elif plat == "macos":
        src = Path(os.environ.get("TMPDIR", "/tmp")) / "bench-runner.log"
        if src.exists():
            shutil.copy2(src, out)
    elif plat == "ios-device" and runner_bid:
        sh(f"xcrun devicectl device copy from --device {udid} "
           f"--domain-type appDataContainer "
           f"--domain-identifier {runner_bid} "
           f"--source tmp/bench-runner.log --destination '{out}'",
           check=False, capture=True)
    if not out.exists():
        out.write_text("")


def read_runner_log(path: Path, since: float):
    """Parse the runner log for this rep: the runner's marks and the
    device record it wrote inside the measured iteration. Entries are
    '<epoch> <text>' lines; only rows at/after `since` count so a
    previous rep's marks can't alias into this one. Returns
    ({drive-begin, measure-end: epoch s}, device record); a mark the
    log does not carry, or a pair out of order, is absent."""
    marks = {}
    device = {}
    if not path.exists():
        return marks, device
    for line in path.read_text().splitlines():
        parts = line.split(" ", 1)
        if len(parts) != 2:
            continue
        try:
            t = float(parts[0])
        except ValueError:
            continue
        if t < since:
            continue
        body = parts[1]
        for name in (MARK_DRIVE_BEGIN, MARK_MEASURE_END):
            if body == name:
                marks[name] = t
        if body.startswith("device-record"):
            for kv in body.split():
                if "=" in kv:
                    k, v = kv.split("=", 1)
                    device[k] = v
    if (len(marks) != 2
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


def _run_xctest_invocation(xr: Path, res: Path, dest: str,
                           results_dir: Path, tag: str):
    """One xcodebuild invocation, tracked so a signal kills its group.
    Returns (rc | None, output_tail)."""
    cmd = (f"xcodebuild test-without-building -xctestrun '{xr}' "
           f"-destination '{dest}' -resultBundlePath '{res}' "
           f"-collect-test-diagnostics never")
    xblog = results_dir / f"{tag}.xcodebuild.log"
    xbf = open(xblog, "w")
    xb = subprocess.Popen(cmd, shell=True, cwd=ROOT, stdout=xbf,
                          stderr=subprocess.STDOUT, start_new_session=True)
    _ACTIVE_PROCS.append(xb)
    try:
        xb.wait(timeout=2400)
        rc = xb.returncode
    except subprocess.TimeoutExpired:
        _kill_proc(xb)
        xb.wait()
        rc = None
    finally:
        if xb in _ACTIVE_PROCS:
            _ACTIVE_PROCS.remove(xb)
        xbf.close()
    xb_out = xblog.read_text() if xblog.exists() else ""
    xblog.unlink(missing_ok=True)
    return rc, xb_out


def _xctest_spawn(xr: Path, res: Path, dest: str, results_dir: Path,
                  tag: str):
    """Spawn an xcodebuild invocation without waiting — used for the
    workload invocation whose recorders arm while the runner waits on
    the recorder-go handshake."""
    cmd = (f"xcodebuild test-without-building -xctestrun '{xr}' "
           f"-destination '{dest}' -resultBundlePath '{res}' "
           f"-collect-test-diagnostics never")
    xblog = results_dir / f"{tag}.xcodebuild.log"
    xbf = open(xblog, "w")
    xb = subprocess.Popen(cmd, shell=True, cwd=ROOT, stdout=xbf,
                          stderr=subprocess.STDOUT, start_new_session=True)
    _ACTIVE_PROCS.append(xb)
    return xb, xbf, xblog

def _window_center_for_pid(pid: int):
    """Centre of the largest on-screen window owned by `pid`, from the
    Quartz window list. The owner is resolved by process id — never by
    name — so the drive can only land in a window this run launched."""
    import Quartz
    wl = Quartz.CGWindowListCopyWindowInfo(
        Quartz.kCGWindowListOptionOnScreenOnly, Quartz.kCGNullWindowID)
    best = None
    for w in wl:
        if w.get("kCGWindowOwnerPID") != pid:
            continue
        b = w.get("kCGWindowBounds", {})
        area = (b.get("Width", 0) or 0) * (b.get("Height", 0) or 0)
        if best is None or area > best[0]:
            best = (area,
                    (b.get("X", 0) or 0) + (b.get("Width", 0) or 0) / 2,
                    (b.get("Y", 0) or 0) + (b.get("Height", 0) or 0) / 2)
    if best is None:
        return None
    return (best[1], best[2])


class SimulatorHost:
    """The Simulator.app process that hosts the booted device's window.

    Only runs with a wheel cell need it: the wheel driver posts host
    scroll events into this window. Other ios-sim cells leave the device
    to xcodebuild, which boots it without a host window. The run
    launches it itself — its executable with `-CurrentDeviceUDID
    <udid>`, through subprocess — so the pid the wheel driver targets
    comes from that launch, never from a lookup by app name or window
    title. Any Simulator host already running — from any Xcode, for any
    device — and any other booted device are refused before the first
    cell: the driver must own the only device window there is."""

    def __init__(self, udid: str):
        self.udid = udid
        self.proc = None

    @staticmethod
    def executable() -> Path:
        simctl = subprocess.run(["xcrun", "--find", "simctl"],
                                capture_output=True, text=True, check=True,
                                timeout=60).stdout.strip()
        # <DEVELOPER_DIR>/usr/bin/simctl → <DEVELOPER_DIR>/Applications
        exe = (Path(simctl).parents[2] / "Applications" / "Simulator.app"
               / "Contents" / "MacOS" / "Simulator")
        if not exe.is_file():
            raise SystemExit(f"Simulator host executable missing: {exe}")
        return exe

    # the host executable inside any Simulator.app, whichever Xcode (or
    # copy of it) it belongs to
    HOST_SUFFIX = "/Simulator.app/Contents/MacOS/Simulator"

    def start(self):
        exe = self.executable()
        out = subprocess.run(["ps", "-axo", "pid=,comm="],
                             capture_output=True, text=True,
                             check=True).stdout
        for line in out.splitlines():
            pid, _, comm = line.strip().partition(" ")
            if not comm.strip().endswith(self.HOST_SUFFIX):
                continue
            argv = subprocess.run(["ps", "-o", "args=", "-p", pid],
                                  capture_output=True, text=True).stdout
            m = re.search(r"-CurrentDeviceUDID\s+(\S+)", argv)
            raise SystemExit(
                f"a Simulator host is already running (pid {pid}, "
                f"{comm.strip()}, device "
                f"{m.group(1) if m else 'unspecified'}) — quit it: wheel "
                f"cells drive the host this run launches for {self.udid}")
        booted = json.loads(subprocess.run(
            ["xcrun", "simctl", "list", "devices", "booted", "--json"],
            capture_output=True, text=True, check=True,
            timeout=60).stdout)
        others = [d["udid"] for devs in booted["devices"].values()
                  for d in devs if d["udid"] != self.udid]
        if others:
            raise SystemExit(
                "other simulators are booted (" + ", ".join(others)
                + f") — shut them down: only {self.udid} may own a "
                "device window while ios-sim cells run")
        self.proc = subprocess.Popen(
            [str(exe), "-CurrentDeviceUDID", self.udid],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        _ACTIVE_PROCS.append(self.proc)
        # blocks until the device has finished booting (booting it if the
        # host has not yet)
        r = subprocess.run(["xcrun", "simctl", "bootstatus", self.udid,
                            "-b"], capture_output=True, text=True,
                           timeout=600)
        if r.returncode != 0:
            self.stop()
            raise SystemExit(f"simulator {self.udid} did not boot: "
                             f"{(r.stderr or r.stdout)[-400:]}")
        if self.proc.poll() is not None:
            raise SystemExit(f"Simulator host exited rc={self.proc.returncode}"
                             f" while {self.udid} booted")

    @property
    def pid(self) -> int:
        if self.proc is None or self.proc.poll() is not None:
            raise RuntimeError("the Simulator host this run launched is "
                               "not running")
        return self.proc.pid

    def stop(self):
        if self.proc is not None:
            _kill_proc(self.proc)
            self.proc.wait()
            if self.proc in _ACTIVE_PROCS:
                _ACTIVE_PROCS.remove(self.proc)
            self.proc = None


class WheelDriver:
    """Host-side OS-level scroll drive for the `wheel` drive mode.

    The listener (`notifyutil -1 dev.bench.begin -g dev.bench.begin`)
    is armed in start(): notifyutil runs its commands left to right, so
    the `-g` state line it prints proves the registration exists, and
    run_one releases the runner's recorder-go only after that — the
    runner's single `dev.bench.begin`, posted when its measure block
    opens, cannot be missed. On begin this driver posts the shared fling
    protocol (manifest `fling` block) as CGEvent scroll-wheel detents
    DIRECTLY to the target process (`CGEventPostToPid`, the event
    located at the window's centre): no cursor warp, no HID-tap
    broadcast. It then holds until begin + the declared hold (the
    capture plus harness.anchor_tolerance_ms, the same span the runner
    holds on every other drive) and posts `dev.bench.end` — a name only
    the driver uses — into the same namespace (the simulator's for
    ios-sim, the host's for macOS). The app never scrolls itself and
    never signals completion.

    The listener runs inside that namespace (for ios-sim, inside the
    simulator, where killing the host-side `simctl spawn` would not
    reach it), so its lifetime is ended there: it exits on its one
    begin, and a cell that ends before any begin releases it with a
    begin of its own after the runner is gone — the driver has stopped
    by then, so that post drives nothing.

    The driver refuses to run anywhere but the declared measurement host
    (`measurement_host.hw_uuid` in the manifest — the Mac mini's
    hardware UUID, printed by `bench.py fingerprint`): a wheel program
    that can post into any process is not a tool to leave live on a
    development machine, and a model name is not an identity — any Mac
    mini contains "Macmini". A mismatch refuses before any cell starts
    (cmd_run_local checks up front)."""

    def __init__(self, plat: str, udid: str | None,
                 pid_resolver, fling: dict, hold_s: float):
        self.plat = plat
        self.udid = udid
        self.pid_resolver = pid_resolver
        self.fling = fling
        self.hold_s = float(hold_s)
        self.error = None
        self._listener = None
        self._lines = None
        self._thread = None
        self._stop = threading.Event()

    @staticmethod
    def confinement_error() -> str | None:
        """None when this host is the declared measurement host, else the
        reason string. Identity is the hardware UUID the manifest
        declares — exact equality, never a model-name substring."""
        mh = MANIFEST.get("measurement_host") or {}
        want = mh.get("hw_uuid")
        ev = host_evidence()
        vm = host_is_virtualized(ev)
        if vm:
            return f"wheel driver: virtualized host ({vm})"
        if not want:
            return ("wheel driver: manifest measurement_host.hw_uuid is "
                    "unset — declare the measurement host's hardware "
                    "UUID (see `bench.py fingerprint`)")
        if ev.get("hw_uuid") != want:
            return ("wheel driver: host is not the declared measurement "
                    f"host (hw_uuid={ev.get('hw_uuid')!r}, expected "
                    f"{want!r} — {mh.get('kind', 'the Mac mini')})")
        return None

    def start(self):
        err = self.confinement_error()
        if err:
            raise RuntimeError(err)
        name = "dev.bench.begin"
        self._listener, self._lines = _spawn_pty(
            _notify_cmd(self.plat, self.udid, "-1", name, "-g", name))
        try:
            armed = self._lines.readline(time.monotonic() + 60)
        except TimeoutError:
            self.stop()
            raise RuntimeError("wheel driver: begin listener not armed "
                               "within 60s") from None
        if armed is None or not armed.startswith(name + " "):
            self.stop()
            raise RuntimeError(
                f"wheel driver: begin listener did not arm: {armed!r}")
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self, bound_s: float = 60.0):
        """End the driver; an unfired listener is released inside its
        namespace and must exit on that release. Records a listener that
        does not exit as the driver's error."""
        self._stop.set()
        if self._listener is not None:
            listener, self._listener = self._listener, None
            try:
                if listener.poll() is None:
                    subprocess.run(_notify_cmd(self.plat, self.udid, "-p",
                                               "dev.bench.begin"),
                                   capture_output=True, check=True,
                                   timeout=60)
                listener.wait(timeout=bound_s)
            except (subprocess.SubprocessError, OSError) as e:
                self.error = self.error or (
                    f"wheel driver: begin listener (pid {listener.pid}) "
                    f"did not exit on its release post: {e}")
                _kill_proc(listener)
                listener.wait()
            finally:
                if listener in _ACTIVE_PROCS:
                    _ACTIVE_PROCS.remove(listener)
        if self._thread is not None:
            self._thread.join(timeout=10)
            self._thread = None
        if self._lines is not None:
            self._lines.close()
            self._lines = None

    def _run(self):
        try:
            fired = self._lines.readline(None)
            if fired is None or self._stop.is_set():
                # EOF, or the release post of a cell that ended before
                # any begin: the runner fails that cell on its own
                return
            if fired.strip() != "dev.bench.begin":
                raise RuntimeError(f"unexpected listener output {fired!r}")
            begin = time.monotonic()
            self._program()
            remaining = begin + self.hold_s - time.monotonic()
            if remaining < 0:
                raise RuntimeError(
                    f"fling program overran the {self.hold_s:g}s hold by "
                    f"{-remaining:.3f}s")
            if self._stop.wait(remaining):
                return
            subprocess.run(_notify_cmd(self.plat, self.udid, "-p",
                                       "dev.bench.end"),
                           capture_output=True, check=True, timeout=60)
        except Exception as e:  # never kill the runner from a thread
            self.error = f"wheel driver: {e}"

    def _program(self):
        # The begin notification is the readiness event itself: it is
        # posted from inside the measure block, so the target exists —
        # resolve its pid once (no launch-pid poll loop).
        pid = self.pid_resolver()
        center = _window_center_for_pid(pid)
        if center is None:
            raise RuntimeError(
                f"no on-screen window owned by pid {pid}")
        import Quartz
        f = self.fling
        seq = [1] * int(f["down"]) + [-1] * int(f["up"])
        step_ms = float(f["duration_ms"]) / int(f["detents"])
        px = int(f["detent_px"])
        point = Quartz.CGPointMake(center[0], center[1])
        # Quartz wheel semantics: a POSITIVE wheel1 delta scrolls content
        # toward the top (the "scroll up" direction); scrolling the feed
        # DOWN is a negative delta — direction verified against
        # NSScrollView on the measurement host.
        for direction in seq:
            for _ in range(int(f["detents"])):
                ev = Quartz.CGEventCreateScrollWheelEvent(
                    None, Quartz.kCGScrollEventUnitPixel, 1,
                    -px * direction)
                # carry the target window's centre so the event lands in
                # the contestant's window regardless of cursor position
                Quartz.CGEventSetLocation(ev, point)
                Quartz.CGEventPostToPid(pid, ev)
                time.sleep(step_ms / 1000)
            time.sleep(float(f["pause_ms"]) / 1000)


# xctrace recordings never outlive the cell, which stops them on SIGINT;
# the limit only bounds a cell that hangs (runner thermal gate 600 s,
# launch + ready 60 s, warmup, the capture, and margin)
XCTRACE_LIMIT_S = 1800


def bundle_executable(app_path: Path, plat: str) -> tuple[str, str]:
    """(CFBundleExecutable, its path relative to the bundle root). The
    executable need not equal the bundle stem — Electron names it
    "Electron" — so it comes from the bundle's own Info.plist."""
    plist_f = (app_path / "Contents" / "Info.plist" if plat == "macos"
               else app_path / "Info.plist")
    exe = plistlib.loads(plist_f.read_bytes()).get("CFBundleExecutable")
    if not exe:
        raise RuntimeError(f"{plist_f} has no CFBundleExecutable")
    return exe, (f"Contents/MacOS/{exe}" if plat == "macos" else exe)


def run_one(plat, contestant_id, app_path: Path, bundle_id, workload,
            drive, duration, rep, dest, products_dir: Path, subdir: str,
            template: Path, target_key: str, results_dir: Path,
            no_hitch: bool = False, device_udid: str | None = None,
            runner_bid: str | None = None, artifact_sha: str | None = None,
            runner_log_since: float = 0.0,
            step: int | None = None,
            sim_host: SimulatorHost | None = None) -> dict:
    """Stage app + injected xctestrun, run two single-test invocations.

    One test per xcodebuild invocation: the launch test and the workload
    test never share a process lifetime. External observation, identical
    for every contestant:
    - macos / ios-device: `xctrace record --all-processes` with the
      Animation Hitches template plus Points of Interest, armed BEFORE
      the runner's recorder-go is released. The contestant's frames are
      the frame lifetimes whose swap composited an update from a process
      inside its bundle; METHOD's window [first owned present + warmup,
      + capture] is computed on that trace's clock, and the runner's
      drive-begin / measure-end signposts in the same trace prove the
      drive started within harness.anchor_tolerance_ms of the window
      start and the contestant was held through its end. On ios-device
      the app and backboardd CPU come from the same trace and window.
      ios-device also records the Logging template the same way (the
      first-paint marker channel), for every contestant.
    - ios-sim / macos: a CpuSampler polls `ps -o time/rss` on the app,
      its helper children and the render server (sim backboardd / macOS
      WindowServer) so work Core Animation executes outside the app
      process is still counted. On macOS it is windowed to the trace's
      window, carried to the host clock through the drive-begin mark
      both clocks carry; ios-sim records no trace, so its (development
      only) window is the runner's drive-begin + the capture.
    - wheel drive: the host driver's listener is armed before
      recorder-go too; it ends the cell itself at begin + the hold.
    - capacity workloads (w5/w6) run one launch per ladder step: the
      caller passes `step` and merges the per-step rows.
    """
    tag = f"{contestant_id}-{workload}"
    if step is not None:
        tag += f"-s{step}"
    tag += f"-r{rep}"
    udid = device_udid or (sim_udid() if plat == "ios-sim" else None)
    dst = products_dir / subdir / app_path.name
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(app_path, dst, symlinks=True)
    go = RecorderGo(plat, udid, runner_bid, results_dir)
    xr_launch = products_dir / f"{tag}-launch.xctestrun"
    xr_work = products_dir / f"{tag}-work.xctestrun"
    for xr, only in ((xr_launch, "testLaunch"), (xr_work, "testWorkload")):
        write_xctestrun(template, xr, target_key, subdir, app_path.name,
                        bundle_id, workload, drive, duration,
                        runner_app="", nonce=go.nonce, no_hitch=no_hitch,
                        only_test=only, step=step)
    res_launch = results_dir / f"{tag}-launch.xcresult"
    res_work = results_dir / f"{tag}-work.xcresult"
    trace = results_dir / f"{tag}.trace"
    fp_trace = results_dir / f"{tag}-log.trace"
    runner_log = results_dir / f"{tag}-runner.log"
    for p in (res_launch, res_work, trace, fp_trace, runner_log):
        if p.is_dir():
            shutil.rmtree(p, ignore_errors=True)
        else:
            p.unlink(missing_ok=True)

    exe, main_rel = bundle_executable(app_path, plat)
    capture_ms = float(duration) * 1000.0
    hold_s = float(duration) + float(
        MANIFEST["harness"]["anchor_tolerance_ms"]) / 1000.0

    rec = {}
    sampler = None
    recorders: dict[str, XctraceRecorder] = {}
    wheel = None
    t_start = time.time()
    try:
        # --- invocation 1: launch test (its own process lifetime) ----
        rc, out = _run_xctest_invocation(xr_launch, res_launch, dest,
                                       results_dir, tag + "-launch")
        if rc is None:
            rec["error"] = "xcodebuild(launch) timed out after 2400s"
        elif rc != 0:
            rec["error"] = f"xcodebuild(launch) rc={rc}: {out[-800:]}"
        else:
            rec["metrics"] = _metrics_record(res_launch).get("metrics")

        # --- invocation 2: workload test, recorders armed first ------
        if plat in ("ios-sim", "macos"):
            sampler = CpuSampler(cpu_pids_resolver(
                plat, udid, bundle_id,
                str(app_path / main_rel) if plat == "macos" else exe),
                interval=0.5)
            sampler.start()
        xb, xbf, xblog = _xctest_spawn(xr_work, res_work, dest,
                                     results_dir, tag + "-work")
        try:
            try:
                if plat in ("macos", "ios-device"):
                    recorders["frames"] = XctraceRecorder(
                        trace, FRAMES_TEMPLATE, device_udid,
                        XCTRACE_LIMIT_S, instruments=FRAMES_INSTRUMENTS)
                if plat == "ios-device":
                    recorders["log"] = XctraceRecorder(
                        fp_trace, "Logging", device_udid, XCTRACE_LIMIT_S)
                for r in recorders.values():
                    r.arm()
                if drive == "wheel":
                    exe_path = str(app_path / main_rel)
                    if plat == "ios-sim":
                        if sim_host is None:
                            raise RuntimeError(
                                "ios-sim wheel drive needs the Simulator "
                                "host this run launched")
                        def resolve():
                            return sim_host.pid
                    else:
                        def resolve():
                            pid = _host_pid_path(exe_path)
                            if pid is None:
                                raise RuntimeError(
                                    f"no process runs {exe_path} at "
                                    "measure begin")
                            return pid
                    wheel = WheelDriver(plat, udid, resolve,
                                        MANIFEST["harness"]["fling"],
                                        hold_s)
                    wheel.start()
                go.release()
            except RuntimeError as e:
                # nothing was released: the runner would only time out
                # on its recorder-go gate — end the invocation now
                rec.setdefault("error", f"recorder arming: {e}")
                _kill_proc(xb)
            try:
                xb.wait(timeout=2400)
                rc = xb.returncode
            except subprocess.TimeoutExpired:
                _kill_proc(xb)
                xb.wait()
                rc = None
            if xb in _ACTIVE_PROCS:
                _ACTIVE_PROCS.remove(xb)
            xbf.close()
            xb_out = xblog.read_text() if xblog.exists() else ""
            xblog.unlink(missing_ok=True)
            if "error" not in rec:
                if rc is None:
                    rec["error"] = "xcodebuild timed out after 2400s"
                elif rc != 0:
                    rec["error"] = f"xcodebuild rc={rc}: {xb_out[-800:]}"
                else:
                    wrec = _metrics_record(res_work)
                    if "error" in wrec:
                        rec["error"] = wrec["error"]
                        rec["xcresult_error"] = wrec.get("xcresult_error")
                    wm = wrec.get("metrics") or {}
                    rec["metrics"] = {**(rec.get("metrics") or {}), **wm}
        finally:
            if (e := go.close()) is not None:
                rec.setdefault("error", e)
            if wheel is not None:
                wheel.stop()
                if wheel.error and "error" not in rec:
                    rec["error"] = wheel.error
            for r in recorders.values():
                if (e := r.stop()) is not None:
                    rec.setdefault("trace_errors", []).append(e)
            if sampler is not None:
                sampler.stop()
                sampler.join(timeout=10)

        if artifact_sha:
            rec["artifact_sha256"] = artifact_sha

        # --- the measurement window --------------------------------
        pull_runner_log(plat, udid or "", runner_bid, runner_log)
        marks, device_rec = read_runner_log(
            runner_log, max(runner_log_since, t_start - 5))
        if device_rec:
            rec["device_record"] = device_rec
        if not marks:
            rec.setdefault(
                "error", "runner log has no drive-begin/measure-end pair "
                         "— the runner never reached its measured span")
        if "error" in rec:
            return rec

        host_window = None   # epoch s, for the host-side sampler
        trace_window = None  # trace ms, for tables of the same trace
        if "frames" in recorders:
            if not device_rec.get("maxFps"):
                rec["error"] = "runner log has no device-record maxFps"
                return rec
            try:
                tw = trace_frame_window(
                    trace, app_path.name, main_rel, capture_ms,
                    refresh_ms=1000.0 / float(device_rec["maxFps"]))
                render = [p["pid"] for p in _trace_toc(trace)
                          if p["name"] == RENDER_NAME[plat]]
            except TraceAttributionError as e:
                rec["error"] = f"frame attribution: {e}"
                return rec
            rec["frame_stats"] = tw["frame_stats"]
            rec["frames"] = tw["frame_stats"]["presents"]
            rec["window"] = {"source": "trace", **tw["window"]}
            rec["attribution"] = tw["attribution"]
            win = tw["window"]
            trace_window = (win["window_start_ms"], win["window_end_ms"])
            # the runner's drive-begin is one instant on both clocks
            off = marks[MARK_DRIVE_BEGIN] - win["drive_begin_ms"] / 1000.0
            host_window = (trace_window[0] / 1000.0 + off,
                           trace_window[1] / 1000.0 + off)
        else:
            d0 = marks[MARK_DRIVE_BEGIN]
            if marks[MARK_MEASURE_END] < d0 + capture_ms / 1000.0:
                rec["error"] = ("runner released the contestant before "
                                "drive-begin + the capture")
                return rec
            host_window = (d0, d0 + capture_ms / 1000.0)
            rec["window"] = {"source": "runner-drive-begin",
                             "start_epoch_s": host_window[0],
                             "end_epoch_s": host_window[1]}

        if sampler is not None:
            w0, w1 = host_window
            rec["renderserver"] = RENDER_NAME[plat]
            rec["cpu_window_source"] = rec["window"]["source"]
            for label, key in (("app", "app_cpu_window_s"),
                               ("render_server", "renderserver_cpu_s"),
                               ("helpers", "helpers_cpu_s")):
                d = sampler.delta(label, w0, w1)
                if d is not None:
                    rec[key] = d
            for label, key in (("app", "app_mem_peak_mb"),
                               ("helpers", "helpers_mem_peak_mb")):
                m = sampler.peak_rss_mb(label, w0, w1)
                if m is not None:
                    rec[key] = m
        if plat == "ios-device":
            # the all-process recording carries every process's samples;
            # the contestant and the render server are selected by pid
            rec["renderserver"] = RENDER_NAME[plat]
            rec["cpu_window_source"] = "trace"
            if len(render) != 1:
                rec["error"] = (f"trace lists {len(render)} "
                                f"{RENDER_NAME[plat]} processes")
                return rec
            for pid, key in ((rec["attribution"]["main_pid"],
                              "app_cpu_window_s"),
                             (render[0], "renderserver_cpu_s")):
                cpu_s, e = _trace_proc_cpu(trace, pid, trace_window)
                if cpu_s is not None:
                    rec[key] = cpu_s
                if e:
                    rec.setdefault("trace_errors", []).append(
                        f"pid {pid}: {e}")

        # apple-backend#281 baseline fields: app size on every row, and
        # waterui's first-paint marker where a readable channel exists.
        rec["app_bytes"] = du_bytes(app_path)
        if contestant_id == "waterui":
            fp = first_paint_ms(
                plat, udid,
                trace=fp_trace if plat == "ios-device" else None,
                since=t_start, proc=exe)
            if fp is not None:
                rec["first_paint_ms"] = fp
        fr = rec.get("frames")
        if fr and rec.get("app_cpu_window_s") is not None:
            rec["cpu_ms_per_frame"] = round(
                rec["app_cpu_window_s"] * 1000.0 / fr, 3)
    finally:
        if sampler is not None:
            sampler.stop()
        # keep xcresult small: delete the bundles after parsing; the
        # .trace stays (it is the frame-interval evidence)
        shutil.rmtree(res_launch, ignore_errors=True)
        shutil.rmtree(res_work, ignore_errors=True)
        xr_launch.unlink(missing_ok=True)
        xr_work.unlink(missing_ok=True)
    return rec


# ------------------------------------------------------------- run-local

def runner_bundle_id(staged: Path) -> str | None:
    """CFBundleIdentifier of the staged *-Runner.app — the recipient of
    the device-side recorder-go sentinel and the owner of the runner log
    the window markers are pulled from."""
    for app in sorted(staged.glob("*-Runner.app")):
        plist_f = app / "Info.plist"
        if not plist_f.exists():
            plist_f = app / "Contents" / "Info.plist"
        if plist_f.exists():
            # an unreadable or id-less plist is a broken staged runner —
            # fail, never skip to the next glob or a silent None
            return plistlib.loads(
                plist_f.read_bytes())["CFBundleIdentifier"]
        raise RuntimeError(f"{app} has no Info.plist")
    return None


def _install_signal_handlers(on_sig):
    """Register SIGINT/SIGTERM cleanup for a run command; returns the
    previous handlers for restoration."""
    prev_int = signal.signal(signal.SIGINT, on_sig)
    prev_term = signal.signal(signal.SIGTERM, on_sig)
    return prev_int, prev_term


def cmd_run_local(args):
    plat = args.platform
    repeats = args.repeats
    # shared-host resources: only one measurement run at a time per
    # platform (the ios-sim drive posts into a shared notify namespace;
    # macOS WindowServer CPU attribution is global)
    lock = device_lock(plat)
    # --drive overrides the manifest's per-contestant decision for every
    # contestant (recorded per row); unset → drive_for() decides.
    drive_override = args.drive
    staged = ROOT / "build" / "artifacts" / plat
    if plat == "ios-sim":
        target_key = MANIFEST["harness"]["test_target_key"]["ios"]
        subdir = "Release-iphonesimulator"
        dest = f"platform=iOS Simulator,id={resolve_sim_udid(args.sim_udid)}"
    elif plat == "macos":
        target_key = MANIFEST["harness"]["test_target_key"]["macos"]
        subdir = "Release"
        dest = "platform=macOS"
    else:
        raise SystemExit("run-local supports ios-sim | macos")

    # the staged artifacts dir is canonical at run time; the manifest's dd
    # path is the build-time source of the same files
    # the staged .xctestrun the build produced is the only template —
    # a missing one fails, never a silent manifest-template fallback
    template = next(iter(sorted(staged.glob("*.xctestrun"))), None)
    if template is None:
        raise SystemExit(
            f"no staged .xctestrun in {staged} — run the build first; "
            "a manifest-template fallback would run unverified "
            "products")

    # harness products dir: copy every staged .app — contestants AND test
    # runners. `*-Runner.app` alone misses a contestant actually named
    # "Runner.app" (flutter's scheme) and leaves a stale build to run.
    products_dir = ROOT / "build" / "harness" / plat / "Products"
    products_dir.mkdir(parents=True, exist_ok=True)
    (products_dir / subdir).mkdir(exist_ok=True)
    for p in sorted(staged.glob("*.app")):
        dd = products_dir / subdir / p.name
        shutil.rmtree(dd, ignore_errors=True)
        shutil.copytree(p, dd, symlinks=True)

    if plat == "macos":
        # unsigned local builds trip Gatekeeper on macOS 26 ("is damaged");
        # clear quarantine + ad-hoc sign every staged app and the runner.
        # Same treatment for every contestant; a signing failure aborts —
        # an unsigned/missigned app would fail launch mid-run.
        for app in sorted(staged.glob("*.app")) + \
                sorted(products_dir.rglob("*.app")):
            subprocess.run(["xattr", "-dr", "com.apple.quarantine", str(app)],
                           capture_output=True)
            for nested in app.rglob("*"):
                if nested.suffix in (".framework", ".dylib"):
                    r = subprocess.run(
                        ["codesign", "-f", "-s", "-", str(nested)],
                        capture_output=True, text=True)
                    if r.returncode != 0:
                        raise SystemExit(
                            f"codesign failed on {nested}: "
                            f"{r.stderr[-300:]}")
            r = subprocess.run(
                ["codesign", "-f", "-s", "-", "--deep", str(app)],
                capture_output=True, text=True)
            if r.returncode != 0:
                raise SystemExit(
                    f"codesign failed on {app}: {r.stderr[-300:]}")

    contestants = [c for c in MANIFEST["contestants"]
                   if c.get("artifact", {}).get(plat)]
    # N5: every staged .app must hash equal to the staging manifest the
    # build wrote — a re-staged/modified artifact that diverges from
    # recorded provenance refuses to run rather than silently measuring
    # an unrecorded binary
    sman_path = staged / "staging-manifest.json"
    artifact_shas = {}
    staged_apps = ([d.name for d in staged.iterdir() if d.suffix == ".app"]
                   if staged.is_dir() else [])
    if staged_apps and not sman_path.exists():
        raise SystemExit(
            f"staged artifacts exist without a staging manifest at "
            f"{sman_path} — build to re-establish provenance; a manifest-"
            "less artifact is unverifiable")
    if sman_path.exists():
        sman = json.loads(sman_path.read_text())
        artifact_shas = sman.get("artifacts") or {}
        for name, want in artifact_shas.items():
            p = staged / name
            if p.is_dir() and dir_sha256(p) != want:
                raise SystemExit(
                    f"staged artifact {name} no longer matches the "
                    "staging manifest — rebuild to re-establish "
                    "provenance")
        missing = [n for n in staged_apps if n not in artifact_shas]
        if missing:
            raise SystemExit(
                "staged artifact(s) absent from the staging manifest: "
                + ", ".join(sorted(missing))
                + " — rebuild to re-establish provenance")
    runner_bid = runner_bundle_id(staged)
    workloads = list(MANIFEST["workloads"].keys())
    if getattr(args, "workloads", None):
        wanted_w = set(args.workloads.split(","))
        workloads = [w for w in workloads if w in wanted_w]

    # the wheel drive posts CGEvents into the launched process — it only
    # ever runs on the declared measurement host (measurement_host
    # .hw_uuid), and the refusal happens BEFORE any cell starts, not
    # after a 600 s timeout per rep
    needs_wheel = any((drive_override or drive_for(c, plat, w)) == "wheel"
                      for c in contestants for w in workloads)
    if needs_wheel:
        if err := WheelDriver.confinement_error():
            raise SystemExit(err)

    results_path = Path(args.out) if args.out else RESULTS_DEFAULT
    state = {"machine": stat_machine(), "platform": plat,
             "manifest": MANIFEST["toolchain"], "pins": collect_pins(),
             "runs": [], "sizes": {}}
    if plat == "ios-sim":
        # simulator rows are development evidence, not hardware timing:
        # the sim renders into a host window with no vsync'd display
        # pipeline and jitters under host load
        state["development_only"] = True
    if results_path.exists():
        state = json.loads(results_path.read_text())
        state.setdefault("machine", {})
        # fingerprint gates run on the recorded provenance, before the
        # pins refresh below would overwrite it
        check_source_fingerprint(state)
        state["pins"] = collect_pins()  # refresh: pin may have moved
        if plat == "ios-sim":
            state["development_only"] = True
    check_host_fingerprint(state)
    if getattr(args, "only", None):
        wanted = set(args.only.split(","))
        contestants = [c for c in contestants if c["id"] in wanted]
    sanitize_runs(state)
    reps_run = (sorted({int(i) for i in args.reps.split(",")})
                if getattr(args, "reps", None) else list(range(repeats)))
    # replace only the cells this invocation will (re)measure: platform ×
    # contestant × workload × drive × rep, so patch runs never clobber
    # neighbouring cells or another platform's rows in the shared file.
    cells = {(plat, c["id"], w, drive_override or drive_for(c, plat, w), rep)
             for c in contestants for w in workloads for rep in reps_run
             if not (w in CAPACITY_WORKLOADS and
                     not capacity_cell(plat, c["id"], w))}
    state["runs"] = [
        x for x in state["runs"]
        if (x.get("platform"), x.get("contestant"), x.get("workload"),
            x.get("drive"), x.get("repeat")) not in cells]

    results_path.parent.mkdir(parents=True, exist_ok=True)

    # sizes (same method for every contestant: on-disk .app bytes)
    for c in contestants:
        app = staged / Path(c["artifact"][plat]).name
        if app.exists():
            state["sizes"].setdefault(plat, {})[c["id"]] = {
                "app_bytes": du_bytes(app),
                "note": "on-disk .app size; .ipa thinning requires signing (device host)",
            }

    # The first-ever XCUITest attach of a session can fail inside the
    # framework itself ("Failed to initialize for UI testing: XCTFuture") —
    # burn one discarded run per contestant before measured reps so rep 0
    # does not carry that failure. Persists across re-runs of the same
    # results file.
    warmed = set(state.get("warmed_up", []))
    # XCTHitchMetric is a platform capability (manifest
    # measurement.hitch_metric): attached everywhere except ios-sim,
    # which has no GPU frame telemetry. It is never learned mid-sweep —
    # no run's outcome decides another contestant's metrics.
    no_hitch = plat == "ios-sim"

    def _on_sig(sig, _frame):
        _kill_active_procs()
        lock.close()
        sys.exit(128 + sig)

    _prev = _install_signal_handlers(_on_sig)
    # ios-sim wheel cells: this run launches the Simulator host for the
    # device it measures, so the wheel driver's target pid comes from
    # that launch; a run without wheel cells needs no host window
    sim_host = None
    try:
      if plat == "ios-sim" and needs_wheel:
          sim_host = SimulatorHost(resolve_sim_udid(args.sim_udid))
          sim_host.start()
      for rep in reps_run:
        # interleave: rotate order so no side gets the same slot every round
        order = contestants[rep % len(contestants):] + contestants[:rep % len(contestants)]
        for c in order:
            app = staged / Path(c["artifact"][plat]).name
            if not app.exists():
                state["runs"].append({
                    "contestant": c["id"], "workload": None,
                    "repeat": rep, "platform": plat,
                    "error": f"missing artifact {app}"})
                continue
            bid = c["bundle_id"].get("ios" if plat != "macos" else "macos")
            if c["id"] not in warmed:
                print(f"[{plat}] warm-up {c['id']} (discarded)", flush=True)
                wd = drive_override or drive_for(c, plat, "w1")
                run_one(plat, c["id"], app, bid, "w1", wd,
                        MANIFEST["workloads"]["w1"]["duration_s"], rep,
                        dest, products_dir, subdir, template, target_key,
                        results_path.parent / "xcresults",
                        no_hitch=True, device_udid=None,
                        runner_bid=runner_bid,
                        artifact_sha=artifact_shas.get(app.name),
                        sim_host=sim_host)
                warmed.add(c["id"])
                state["warmed_up"] = sorted(warmed)
                results_path.write_text(json.dumps(state, indent=1))
            for w in workloads:
                if (w in CAPACITY_WORKLOADS
                        and not capacity_cell(plat, c["id"], w)):
                    continue  # W5/W6 ship in the five iOS contestants only
                drive = drive_override or drive_for(c, plat, w)
                duration = MANIFEST["workloads"][w]["duration_s"]
                # the thermal gate is the runner's own: setUp blocks on
                # thermalStateDidChangeNotification until nominal (macOS
                # included) and fails the row past its bound
                steps = (capacity_steps(w) if w in CAPACITY_WORKLOADS
                         else [None])
                rec = None
                for si, n in enumerate(steps):
                    print(f"[{plat}] rep {rep+1}/{repeats} {c['id']} {w}"
                          + (f" step={n}" if n is not None else "")
                          + f" drive={drive}", flush=True)
                    rec = run_one(
                        plat, c["id"], app, bid, w, drive, duration,
                        rep, dest, products_dir, subdir, template,
                        target_key, results_path.parent / "xcresults",
                        no_hitch=no_hitch, device_udid=None,
                        runner_bid=runner_bid,
                        artifact_sha=artifact_shas.get(app.name),
                        step=n, sim_host=sim_host)
                    rec.update({"contestant": c["id"], "workload": w,
                                "repeat": rep, "platform": plat,
                                "drive": drive})
                    if n is not None:
                        rec.setdefault("capacity", {}).setdefault(
                            "steps", []).append({
                                "step": si, "n": n, **{
                                    k: v for k, v in rec.items()
                                    if k in ("metrics", "frame_stats",
                                             "app_cpu_window_s",
                                             "renderserver_cpu_s",
                                             "helpers_cpu_s",
                                             "app_mem_peak_mb",
                                             "helpers_mem_peak_mb",
                                             "cpu_ms_per_frame",
                                             "frames", "error")}})
                    if rec.get("error"):
                        break
                if rec is not None and rec.get("capacity", {}).get("steps"):
                    cap120 = cap60 = 0
                    for st in rec["capacity"]["steps"]:
                        fs = st.get("frame_stats") or {}
                        if fs.get("missed_vsyncs") == 0 and st.get("frames"):
                            cap120 = cap60 = st["n"]
                        elif (fs.get("frame_ms_p90") or 9e9) <= 16.67 * 1.5:
                            cap60 = st["n"]
                    rec["capacity"]["capacity_120hz"] = cap120
                    rec["capacity"]["capacity_60hz"] = cap60
                state["runs"].append(rec)
                results_path.write_text(json.dumps(state, indent=1))
                # Hitch evidence is per-row: a xcresult that emits no
                # hitch identifiers is visible in flatten as missing
                # metric, never silently rewritten or propagated — the
                # platform's attachment declaration (no_hitch) does not
                # change mid-sweep.
    finally:
        if sim_host is not None:
            sim_host.stop()
        signal.signal(signal.SIGINT, _prev[0])
        signal.signal(signal.SIGTERM, _prev[1])
        lock.close()
    print(f"results → {results_path}")


# ---------------------------------------------------------------- device

def ensure_profile(bundle_id: str, team: str, udid: str, workdir: Path):
    """Mint a provisioning profile for bundle_id without compiling."""
    prof_dir = Path.home() / "Library/Developer/Xcode/UserData/Provisioning Profiles"
    have = None
    for p in prof_dir.glob("*.mobileprovision"):
        try:
            r = subprocess.run(["security", "cms", "-D", "-i", str(p)],
                               capture_output=True, text=True)
            d = plistlib.loads(r.stdout.encode())
            if (d.get("Entitlements", {}).get("application-identifier", "")
                    .endswith(bundle_id)
                    and d.get("ExpirationDate").timestamp() > time.time() + 86400):
                have = p
                break
        except Exception:
            continue
    if have:
        return have
    # generate an empty application target to force automatic provisioning
    proj = workdir / "prov" / bundle_id
    proj.mkdir(parents=True, exist_ok=True)
    name = "ProvApp"
    (proj / "project.yml").write_text(f"""
name: ProvApp
options:
  bundleIdPrefix: {bundle_id.rsplit('.', 1)[0]}
targets:
  {name}:
    type: application
    platform: iOS
    deploymentTarget: "17.0"
    sources: []
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: {bundle_id}
        DEVELOPMENT_TEAM: {team}
        CODE_SIGN_STYLE: Automatic
        GENERATE_INFOPLIST_FILE: YES
""")
    sh(f"xcodegen --spec '{proj}/project.yml' --project '{proj}'")
    # The dummy build signs with the development identity, which the keychain
    # only releases inside the GUI security session — the same session
    # _codesign enters. sudo drops the environment, so DEVELOPER_DIR is passed
    # explicitly.
    developer_dir = subprocess.run(["xcode-select", "-p"], capture_output=True,
                                   text=True).stdout.strip()
    developer_dir = os.environ.get("DEVELOPER_DIR", developer_dir)
    sh(f"sudo -n launchctl asuser {os.getuid()} sudo -n -u {os.getenv('USER')!s} "
       f"env DEVELOPER_DIR='{developer_dir}' "
       "xcodebuild -project '{p}/{n}.xcodeproj' -scheme '{n}' "
       "-destination 'id={u}' -allowProvisioningUpdates "
       "-allowProvisioningDeviceRegistration build"
       .format(p=proj.resolve(), n=name, u=udid))
    for p in prof_dir.glob("*.mobileprovision"):
        r = subprocess.run(["security", "cms", "-D", "-i", str(p)],
                           capture_output=True, text=True)
        try:
            d = plistlib.loads(r.stdout.encode())
        except Exception:
            continue
        if d.get("Entitlements", {}).get("application-identifier", "").endswith(bundle_id):
            return p
    raise RuntimeError(f"no provisioning profile minted for {bundle_id}")


def sign_app(app: Path, profile: Path, ident_hash: str):
    """Inside-out codesign via the GUI security session."""
    shutil.copy2(profile, app / "embedded.mobileprovision")
    r = subprocess.run(["security", "cms", "-D", "-i", str(profile)],
                       capture_output=True, text=True)
    ents = plistlib.loads(r.stdout.encode())["Entitlements"]
    ents_plist = app.parent / (app.stem + ".entitlements")
    ents_plist.write_bytes(plistlib.dumps(ents))
    # sign nested frameworks/dylibs first
    for nested in sorted(app.rglob("*"), key=lambda p: -len(p.parts)):
        if nested.suffix in (".framework", ".dylib") and nested.is_dir() or \
                nested.suffix == ".dylib":
            inner = nested / nested.stem if nested.suffix == ".framework" else nested
            if inner.exists():
                _codesign(inner, ident_hash)
    _codesign(app, ident_hash, ents_plist)


def _codesign(path: Path, ident_hash: str, ents: Path | None = None):
    cmd = (f"sudo -n launchctl asuser {os.getuid()} sudo -n -u {os.getenv('USER')!s} "
           f"/usr/bin/codesign -f -s {ident_hash} --timestamp=none")
    if ents:
        cmd += f" --entitlements '{ents}'"
    sh(f"{cmd} '{path}'")


def device_lock(name: str):
    LOCK_DIR.mkdir(exist_ok=True)
    f = open(LOCK_DIR / f"{name}.lock", "w")
    print(f"waiting for device lock {name}…", flush=True)
    fcntl.flock(f, fcntl.LOCK_EX)
    print(f"device lock {name} acquired", flush=True)
    return f


# devicectl `device info details --json-output` field names -> state keys
_DEVICE_INFO_FIELDS = ("thermalState", "maximumFramesPerSecond",
                       "marketingName", "deviceName", "productType",
                       "osVersionNumber", "productVersion")


def parse_device_info(doc) -> dict:
    """Pure parse of one devicectl JSON document. Missing fields stay
    absent/None — the caller must not treat 'unknown' as nominal, and
    there is no spec-sheet refresh fallback."""
    st = {"refresh_hz": None, "thermal": "unknown"}
    found = _find_json_fields(doc, _DEVICE_INFO_FIELDS)
    if found.get("thermalState") is not None:
        st["thermal"] = str(found["thermalState"])
    if found.get("maximumFramesPerSecond") is not None:
        try:
            st["refresh_hz"] = int(
                float(str(found["maximumFramesPerSecond"])))
            st["refresh_hz_source"] = "devicectl maximumFramesPerSecond"
        except ValueError:
            st["read_error"] = ("maximumFramesPerSecond unparsable: "
                                f"{found['maximumFramesPerSecond']!r}")
    if found.get("marketingName") or found.get("deviceName"):
        st["model"] = str(found.get("marketingName")
                          or found["deviceName"])
    if found.get("productType"):
        st["product_type"] = str(found["productType"])
    if found.get("osVersionNumber") or found.get("productVersion"):
        st["os_version"] = str(found.get("osVersionNumber")
                               or found["productVersion"])
    return st


def device_state(udid: str) -> dict:
    """Model + refresh rate + thermal state, parsed from devicectl's
    real JSON output. Unreadable output records a read_error and leaves
    thermal 'unknown' / refresh_hz null."""
    st = {"udid": udid, "refresh_hz": None, "thermal": "unknown"}
    out = Path(tempfile.gettempdir()) / f"bench-dinfo-{os.getpid()}.json"
    sh(f"xcrun devicectl device info details --device {udid} "
       f"--json-output '{out}'", check=False, capture=True, timeout=60)
    try:
        st.update(parse_device_info(json.loads(out.read_text())))
    except Exception as e:
        st["read_error"] = f"{type(e).__name__}: {e}"
    finally:
        out.unlink(missing_ok=True)
    st["udid"] = udid
    return st


def uninstall_app(udid: str, bundle_id: str):
    """Removes one app from the device; an app that is not installed is fine."""
    installed = subprocess.run(
        ["xcrun", "devicectl", "device", "info", "apps", "--device", udid,
         "--bundle-id", bundle_id, "--json-output", "/dev/stdout"],
        capture_output=True, text=True, timeout=60)
    if bundle_id not in installed.stdout:
        return
    sh(f"xcrun devicectl device uninstall app --device {udid} {bundle_id}",
       timeout=120)


def resolve_device_udid(requested: str | None = None) -> str:
    """Explicit --udid wins; otherwise the single device `devicectl list
    devices` reports as available is selected. Zero or several
    candidates is an error naming them — never a baked-in hardware id."""
    if requested:
        return requested
    out = Path(tempfile.gettempdir()) / f"bench-devices-{os.getpid()}.json"
    sh(f"xcrun devicectl list devices --json-output '{out}'",
       capture=True, timeout=30)
    try:
        doc = json.loads(out.read_text())
    finally:
        out.unlink(missing_ok=True)
    devices = ((doc.get("result") or {}).get("devices")
               or doc.get("devices") or [])
    def ok(d):
        cp = d.get("connectionProperties") or {}
        return (d.get("identifier")
                and cp.get("tunnelState", "available")
                in ("available", "connected"))
    cands = [d for d in devices if ok(d)]
    if len(cands) == 1:
        udid = cands[0]["identifier"]
        name = (cands[0].get("deviceProperties") or {}).get("name", "?")
        print(f"resolved device: {name} ({udid})", flush=True)
        return udid
    listing = "\n".join(
        f"  {d.get('identifier')} "
        f"{(d.get('deviceProperties') or {}).get('name', '?')}"
        for d in devices) or "  (none reported)"
    raise SystemExit(
        f"--udid required: {len(cands)} device candidates:\n{listing}")


def signing_identity(selector: str | None = None):
    """(cert hash, name, team) of the keychain's Apple Development
    identity. `selector` (--identity) picks among several; no default
    author identity is baked in."""
    out = subprocess.run(
        ["security", "find-identity", "-v", "-p", "codesigning"],
        capture_output=True, text=True, timeout=30).stdout
    ids = re.findall(r'([0-9A-F]{40})\s+"(Apple Development:[^"]+)"',
                     out)
    if selector:
        ids = [i for i in ids if selector in i[1]]
    if not ids:
        raise SystemExit(
            "no usable 'Apple Development' signing identity in the "
            "keychain:\n" + out)
    if len(ids) > 1:
        raise SystemExit(
            "multiple 'Apple Development' identities — pass --identity "
            "with a substring of one:\n"
            + "\n".join(f"  {h} {n}" for h, n in ids))
    ih, name = ids[0]
    m = re.search(r"\(([A-Z0-9]{10})\)", name)
    return ih, name, (m.group(1) if m else None)


def _cleanup_step(label: str, fn) -> str | None:
    """Run one cleanup action; failures are returned as diagnostics,
    never raised — one broken cleanup must not suppress the rest."""
    try:
        fn()
        return None
    except Exception as e:
        print(f"cleanup failed ({label}): {e}", flush=True)
        return f"{label}: {e}"


def cmd_device(args):
    """iOS device measurement on the Apple Silicon host. Expects staged
    unsigned artifacts: <artifacts>/<id>.app plus the runner products
    and template .xctestrun. Device and signing identity are resolved
    from the actual host (or explicit --udid/--identity/--team), never
    baked in."""
    artifacts = Path(args.artifacts)
    udid = resolve_device_udid(args.udid)
    ih, ident_name, ident_team = signing_identity(args.identity)
    team = args.team or ident_team
    if not team:
        raise SystemExit(
            "cannot derive DEVELOPMENT_TEAM from identity "
            f"{ident_name!r} — pass --team explicitly")
    print(f"signing with {ident_name} (team {team})", flush=True)
    lock = device_lock(udid)
    installed = []   # bundle ids this run installed (cleanup unowns only these)
    own_dirs = []    # directories this run created (testroot, provwork)

    _cleaned = {"done": False}

    def cleanup() -> list:
        if _cleaned["done"]:
            return []
        _cleaned["done"] = True
        diags = []
        for bid in reversed(installed):
            d = _cleanup_step(
                f"uninstall {bid}", lambda b=bid: uninstall_app(udid, b))
            if d:
                diags.append(d)
        for p in own_dirs:
            d = _cleanup_step(
                f"remove {p}",
                lambda q=p: shutil.rmtree(q) if Path(q).exists() else None)
            if d:
                diags.append(d)
        return diags

    def on_signal(sig, _frame):
        _kill_active_procs()   # only PIDs this run spawned
        cleanup()
        sys.exit(128 + sig)

    prev_int = signal.signal(signal.SIGINT, on_signal)
    prev_term = signal.signal(signal.SIGTERM, on_signal)
    try:
        # Stage a device testroot: Release-iphoneos/ holds the signed runner
        # plus each signed contestant; the injected .xctestrun sits beside it.
        testroot = artifacts / "device-testroot"
        provwork = artifacts / "provwork"
        for _d in (testroot, provwork):
            # cleanup may only delete what this run created — a dir that
            # predates the run belongs to an earlier one
            if not _d.exists():
                own_dirs.append(_d)

        # Stage a device testroot: Release-iphoneos/ holds the signed runner
        # plus each signed contestant; the injected .xctestrun sits beside it.
        prod = testroot / "Release-iphoneos"
        prod.mkdir(parents=True, exist_ok=True)
        template = artifacts / "device.xctestrun"
        if not template.exists():
            # fall back to a template staged next to the artifacts
            cand = list(artifacts.glob("*.xctestrun"))
            if not cand:
                raise SystemExit("no template .xctestrun in artifacts dir")
            shutil.copy2(cand[0], template)

        runner_bid = runner_bundle_id(artifacts)
        # every staged .app must hash equal to the staging manifest the
        # build wrote
        sman_path = artifacts / "staging-manifest.json"
        artifact_shas = {}
        if sman_path.exists():
            sman = json.loads(sman_path.read_text())
            artifact_shas = sman.get("artifacts") or {}
            for name, want in artifact_shas.items():
                p = artifacts / name
                if p.is_dir() and dir_sha256(p) != want:
                    raise SystemExit(
                        f"staged artifact {name} no longer matches the "
                        "staging manifest — rebuild to re-establish "
                        "provenance")

        # stage and sign the UI-test runner once
        runner_app = prod / "BenchRunner-Runner.app"
        staged_runner = artifacts / runner_app.name
        if not staged_runner.exists():
            raise SystemExit(f"no UI-test runner at {staged_runner}")
        shutil.rmtree(runner_app, ignore_errors=True)
        shutil.copytree(staged_runner, runner_app, symlinks=True)
        runner_id = None
        if runner_app.exists():
            # The runner app's identifier is the test target's plus
            # `.xctrunner`; its profile must match that exact App ID.
            with open(runner_app / "Info.plist", "rb") as f:
                runner_id = plistlib.load(f)["CFBundleIdentifier"]
            installed.append(runner_id)  # xcodebuild installs it per run
            rprof = ensure_profile(runner_id, team, udid, provwork)
            sign_app(runner_app, rprof, ih)
            xct = runner_app / "PlugIns/BenchRunner.xctest"
            if xct.exists():
                _codesign(xct, ih)

        contestants = [c for c in MANIFEST["contestants"]
                       if c.get("bundle_id", {}).get("ios")]
        if getattr(args, "only", None):
            wanted = set(args.only.split(","))
            contestants = [c for c in contestants if c["id"] in wanted]
        workloads = list(MANIFEST["workloads"].keys())
        if getattr(args, "workloads", None):
            wanted_w = set(args.workloads.split(","))
            workloads = [w for w in workloads if w in wanted_w]
        state = {"machine": stat_machine(), "platform": "ios-device",
                 "manifest": MANIFEST["toolchain"], "pins": collect_pins(),
                 "runs": [], "sizes": {}}
        # Record the attached device identity once; per-run
        # device_state rows keep the sampled refresh/thermal values.
        state["device"] = device_state(udid)
        results_path = Path(args.out or artifacts / "results-device.json")
        if results_path.exists():
            state = json.loads(results_path.read_text())
            check_source_fingerprint(state)
            check_host_fingerprint(state)
            prev_dev = (state.get("device") or {}).get("udid")
            if prev_dev and prev_dev != udid:
                raise SystemExit(
                    "results file was produced on a different device — "
                    "refusing to merge\n"
                    f"  recorded: {prev_dev}\n  current:   {udid}")
            state["pins"] = collect_pins()
        for c in contestants:
            app = artifacts / Path(c["artifact"]["ios-device"]).name
            if app.exists():
                state["sizes"].setdefault("ios-device", {})[c["id"]] = {
                    "app_bytes": du_bytes(app),
                    "note": "signed device .app"}
        # A free developer profile allows three installed apps per device, and
        # the UI-test runner takes one, so each contestant is installed only
        # while it is measured. Clear this benchmark's own leftovers first.
        for c in contestants:
            uninstall_app(udid, c["bundle_id"]["ios"])
        sanitize_runs(state)
        reps_run = (sorted({int(i) for i in args.reps.split(",")})
                    if getattr(args, "reps", None)
                    else list(range(args.repeats)))
        # replace only the cells this invocation will (re)measure — same
        # platform × contestant × workload × drive × rep key as run-local.
        cells = {("ios-device", c["id"], w, drive_for(c, "ios-device", w), rep)
                 for c in contestants for w in workloads for rep in reps_run
                 if capacity_cell("ios-device", c["id"], w)
                 or w not in CAPACITY_WORKLOADS}
        state["runs"] = [
            x for x in state["runs"]
            if (x.get("platform"), x.get("contestant"), x.get("workload"),
                x.get("drive"), x.get("repeat")) not in cells]
        # XCTHitchMetric attaches on ios-device per the platform
        # declaration (measurement.hitch_metric); presented frames come
        # from the xctrace Animation Hitches attach. The declaration
        # never mutates mid-sweep.
        no_hitch = False
        warmed = set(state.get("warmed_up", []))
        try:
            for rep in reps_run:
                order = (contestants[rep % len(contestants):]
                         + contestants[:rep % len(contestants)])
                for c in order:
                    app = artifacts / Path(c["artifact"]["ios-device"]).name
                    if not app.exists():
                        state["runs"].append({
                            "contestant": c["id"], "workload": None,
                            "repeat": rep, "platform": "ios-device",
                            "error": f"missing artifact {app}"})
                        results_path.write_text(
                            json.dumps(state, indent=1))
                        continue
                    bid = c["bundle_id"]["ios"]
                    prof = ensure_profile(bid, team, udid, provwork)
                    try:
                        sign_app(app, prof, ih)
                        sh("xcrun devicectl device install app "
                           f"--device {udid} '{app}'", timeout=300)
                        installed.append(bid)
                        dst = prod / app.name
                        shutil.rmtree(dst, ignore_errors=True)
                        shutil.copytree(app, dst, symlinks=True)
                        st = device_state(udid)
                        if c["id"] not in warmed:
                            # One discarded run per contestant: the first-ever
                            # XCUITest attach on a freshly installed device can
                            # fail inside the framework ("Failed to initialize
                            # for UI testing: XCTFuture") and must not land on
                            # a measured rep.
                            print("[ios-device] warm-up "
                                  f"{c['id']} (discarded)", flush=True)
                            run_one("ios-device", c["id"], app, bid,
                                    "w1", drive_for(c, "ios-device", "w1"),
                                    MANIFEST["workloads"]["w1"]["duration_s"],
                                    rep, f"platform=iOS,id={udid}",
                                    testroot, "Release-iphoneos", template,
                                    MANIFEST["harness"]["test_target_key"]["ios"],
                                    artifacts / "xcresults-device",
                                    no_hitch=True, device_udid=udid,
                                    runner_bid=runner_bid,
                                    artifact_sha=artifact_shas.get(
                                        app.name))
                            warmed.add(c["id"])
                            state["warmed_up"] = sorted(warmed)
                            results_path.write_text(
                                json.dumps(state, indent=1))
                        for w in workloads:
                            if (w in CAPACITY_WORKLOADS
                                    and not capacity_cell(
                                        "ios-device", c["id"], w)):
                                continue
                            # the thermal gate lives in the runner's
                            # setUp — devicectl has no thermal channel
                            wspec = MANIFEST["workloads"][w]
                            drive = drive_for(c, "ios-device", w)
                            steps = (capacity_steps(w)
                                     if w in CAPACITY_WORKLOADS else [None])
                            rec = None
                            for si, n in enumerate(steps):
                                rec = run_one(
                                    "ios-device", c["id"], app, bid, w,
                                    drive, wspec["duration_s"], rep,
                                    f"platform=iOS,id={udid}",
                                    testroot, "Release-iphoneos",
                                    template,
                                    MANIFEST["harness"]["test_target_key"]["ios"],
                                    artifacts / "xcresults-device",
                                    no_hitch=no_hitch, device_udid=udid,
                                    runner_bid=runner_bid,
                                    artifact_sha=artifact_shas.get(app.name),
                                    step=n)
                                rec.update({"contestant": c["id"], "workload": w,
                                            "repeat": rep,
                                            "platform": "ios-device",
                                            "drive": drive,
                                            "device_state": st})
                                if n is not None:
                                    rec.setdefault("capacity", {}).setdefault(
                                        "steps", []).append({
                                            "step": si, "n": n, **{
                                                k: v for k, v in rec.items()
                                                if k in ("metrics",
                                                         "frame_stats",
                                                         "app_cpu_window_s",
                                                         "renderserver_cpu_s",
                                                         "frames",
                                                         "cpu_ms_per_frame",
                                                         "error")}})
                                if rec.get("error"):
                                    break
                            if (rec is not None
                                    and rec.get("capacity", {}).get("steps")):
                                cap120 = cap60 = 0
                                for st in rec["capacity"]["steps"]:
                                    fs = st.get("frame_stats") or {}
                                    if (fs.get("missed_vsyncs") == 0
                                            and st.get("frames")):
                                        cap120 = cap60 = st["n"]
                                    elif (fs.get("frame_ms_p90") or 9e9)                                             <= 16.67 * 1.5:
                                        cap60 = st["n"]
                                rec["capacity"]["capacity_120hz"] = cap120
                                rec["capacity"]["capacity_60hz"] = cap60
                            state["runs"].append(rec)
                            results_path.write_text(
                                json.dumps(state, indent=1))
                            # Hitch evidence is per-row: an xcresult
                            # emitting no hitch identifiers shows as a
                            # missing metric in flatten — attachment
                            # never changes mid-sweep.
                    finally:
                        # uninstall only what this run installed — even
                        # when the contestant errored mid-run (free-team
                        # cap ~3 app IDs makes leftovers break later runs)
                        if bid in installed:
                            _cleanup_step(
                                f"uninstall {bid}",
                                lambda b=bid: uninstall_app(udid, b))
                            installed.remove(bid)
        finally:
            diags = cleanup()
            signal.signal(signal.SIGINT, prev_int)
            signal.signal(signal.SIGTERM, prev_term)
            if diags:
                print("cleanup diagnostics: " + "; ".join(diags),
                      flush=True)
        print(f"device results → {results_path}")
    finally:
        lock.close()


# ---------------------------------------------------------------- report

KNOWN = {
    "AppLaunch.duration": "cold launch (s)",
    "physical_peak": "peak memory (kB)",
    "physical_absolute": "steady memory (kB)",
    "time": "CPU time (s)",
    "hitch_time_ratio": "hitch time ratio (ms/s)",
    "total_hitches": "hitches (count)",
    "scrollDecelerationAndScrolling.frameRate": "scroll fps",
}


def flatten(results: dict, plat: str) -> dict:
    """(contestant, workload) -> metric -> {median,min,max,samples}."""
    import statistics
    table = {}
    for run in results["runs"]:
        if run.get("platform") != plat or "metrics" not in run:
            continue
        key = (run["contestant"], run["workload"], run.get("drive", "swipe"))
        cell = table.setdefault(key, {})
        for test, mets in run["metrics"].items():
            if not isinstance(mets, dict):
                continue  # e.g. the verbatim "_error" diagnostics string
            for tail, m in mets.items():
                name = f"{test}:{tail}"
                for v in m["measurements"]:
                    cell.setdefault(name, {"unit": m["unit"], "samples": []})
                    cell[name]["samples"].append(v)
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
    evidence, not just ANY metric: launch duration, memory and CPU must
    be present, plus frame/pacing evidence (a scroll-fps signpost
    metric, a hitch metric, or xctrace presented frames on device).
    Error rows are kept verbatim but never counted."""
    m = r.get("metrics")
    if not (isinstance(m, dict)
            and any(isinstance(v, dict) for v in m.values())):
        return False
    tails = {tail for mets in m.values() if isinstance(mets, dict)
             for tail, mm in mets.items()
             if isinstance(mm, dict) and mm.get("measurements")}
    if not {"duration", "physical_peak", "time"} <= tails:
        return False
    frame_evidence = (
        "scrollDecelerationAndScrolling.frameRate" in tails
        or "hitch_time_ratio" in tails
        or bool(r.get("frames")))
    return frame_evidence


def required_cells(plat: str) -> set:
    """(contestant, workload, drive) cells a complete dataset carries:
    every contestant declaring an artifact for the platform x every
    non-capacity workload at the manifest-resolved drive."""
    reqs = set()
    for c in MANIFEST["contestants"]:
        if not c.get("artifact", {}).get(plat):
            continue
        for w in MANIFEST["workloads"]:
            if w in CAPACITY_WORKLOADS and not capacity_cell(
                    plat, c["id"], w):
                continue
            reqs.add((c["id"], w, drive_for(c, plat, w)))
    return reqs


def completeness(results: dict, plat: str, min_reps: int = MIN_REPS):
    """Cells below the rep floor: (cell, successes, attempts, reason).
    Missing (no attempts), all-failed, and mixed-attempt cells under
    min_reps all count — retry attempts are never padded."""
    have = {}
    for r in results["runs"]:
        if r.get("platform") != plat:
            continue
        key = (r.get("contestant"), r.get("workload"),
               r.get("drive", "swipe"))
        have.setdefault(key, []).append(r)
    gaps = []
    for cell in sorted(required_cells(plat)):
        rows = have.get(cell, [])
        ok = sum(1 for r in rows if _run_succeeded(r))
        if not rows:
            gaps.append((cell, 0, 0, "missing — no attempts recorded"))
        elif not ok:
            gaps.append((cell, 0, len(rows), "all attempts failed"))
        elif ok < min_reps:
            gaps.append((cell, ok, len(rows),
                         f"{ok}/{min_reps} successful reps"))
    return gaps


def _file_provenance(doc: dict, name: str):
    """(host fingerprint, cli sha256) a results file recorded — None
    where the file predates provenance recording."""
    fp = (doc.get("machine") or {}).get("fingerprint")
    cli = (doc.get("pins") or {}).get("water_cli")
    return fp, (cli if isinstance(cli, str)
                and not cli.startswith("unresolved") else None)


def cmd_report(args):
    files = [Path(p) for p in str(args.input).split(",")]
    docs = [(p, json.loads(p.read_text())) for p in files]
    results = docs[0][1]
    if len(docs) > 1:
        # merged inputs must carry identical actual host/GPU and
        # declared-CLI fingerprints — stale or unknown provenance is
        # never merged
        provs = [(p.name,) + _file_provenance(d, p.name)
                 for p, d in docs]
        if any(fp is None for _, fp, _ in provs):
            raise SystemExit(
                "refusing to merge: file(s) without a host "
                "fingerprint: "
                + ", ".join(n for n, fp, _ in provs if fp is None))
        if any(cli is None for _, _, cli in provs):
            raise SystemExit(
                "refusing to merge: file(s) without a CLI sha256: "
                + ", ".join(n for n, _, c in provs if c is None))
        if len({json.dumps(fp, sort_keys=True) for _, fp, _ in provs}) > 1:
            raise SystemExit(
                "refusing to merge results recorded on different "
                "hosts:\n" + "\n".join(
                    f"  {n}: {fp}" for n, fp, _ in provs))
        if len({c for _, _, c in provs}) > 1:
            raise SystemExit(
                "refusing to merge results produced by different water "
                "CLI builds:\n" + "\n".join(
                    f"  {n}: {c}" for n, _, c in provs))
    for _, other in docs[1:]:
        results["runs"].extend(other.get("runs", []))
        for plat, sizes in other.get("sizes", {}).items():
            results["sizes"].setdefault(plat, {}).update(sizes)
    sanitize_runs(results)
    incomplete = []
    required_plats = MANIFEST.get("measurement", {}).get(
        "required_platforms") or []
    plats = sorted(set(required_plats) | {
        r.get("platform") for r in results["runs"] if r.get("platform")})
    lines = ["# Competitive benchmark — Apple (water-rs/waterui#1262)", "",
             f"Machine: `{results['machine'].get('model','?')}`, "
             f"{results['machine'].get('os','?')} {results['machine'].get('kernel','')}, "
             f"{results['machine'].get('memory_gb','?')} GB", ""]
    for c in MANIFEST["contestants"]:
        lines.append(f"- **{c['label']}**: {c['framework_version']}")
    pins = results.get("pins") or {}
    if pins:
        lines.append("")
        lines.append("Commit pins: " + ", ".join(
            f"{k} `{v}`" for k, v in pins.items()))
    lines.append("")
    for plat in plats:
        table = flatten(results, plat)
        # every contestant declaring an artifact for this platform is a
        # required contestant — a platform with no rows still renders
        # its (empty) section and its completeness gaps
        contestants = [c["id"] for c in MANIFEST["contestants"]
                       if c.get("artifact", {}).get(plat)]
        lines += [f"## {plat}", ""]
        sizes = results.get("sizes", {}).get(plat, {})
        def cname(cid):
            """Column name for a contestant on this platform — a manifest
            `platform_label` override (e.g. a non-comparable build mode)
            wins over the shared label, which wins over the raw id."""
            entry = next((c for c in MANIFEST["contestants"]
                          if c["id"] == cid), {})
            return (entry.get("platform_label", {}).get(plat)
                    or entry.get("label") or cid)

        # A contestant whose platform label marks it non-comparable is
        # excluded from the ×-WaterUI ratio rows (its raw medians still
        # print under the label).
        noncomparable = {cid for cid in contestants
                         if (next((c for c in MANIFEST["contestants"]
                                   if c["id"] == cid), {})
                             .get("comparable", {}).get(plat) is False)}

        lines.append("### Package size (on-disk .app, MB)")
        lines.append("| " + " | ".join(
            ["metric"] + [cname(c) for c in contestants]) + " |")
        lines.append("|" + "---|" * (len(contestants) + 1))
        row = ["app MB"]
        for cid in contestants:
            b = sizes.get(cid, {}).get("app_bytes")
            row.append(f"{b/1e6:.1f}" if b else "—")
        lines.append("| " + " | ".join(row) + " |")
        wu = sizes.get("waterui", {}).get("app_bytes")
        if wu:
            row = ["× WaterUI"]
            for cid in contestants:
                b = sizes.get(cid, {}).get("app_bytes")
                row.append("1.00" if cid == "waterui" else
                           ("—" if cid in noncomparable
                            else (f"{b/wu:.2f}" if b else "—")))
            lines.append("| " + " | ".join(row) + " |")
        lines.append("")
        workloads = list(MANIFEST["workloads"].keys())
        for w in workloads:
            drives = sorted({k[2] for k in table if k[1] == w})
            for drive in drives:
                wconts = [cid for cid in contestants
                          if (cid, w, drive) in table]
                if not wconts:
                    continue
                suffix = f" (drive: {drive})" if drive != "swipe" else ""
                lines.append(f"### {w} — {MANIFEST['workloads'][w]['name']}{suffix}")
                metrics = sorted({mn for (cid, wl, dr), cell in table.items()
                                  if wl == w and dr == drive and cid in wconts
                                  for mn in cell})
                lines.append("| " + " | ".join(
                    ["metric — median · min..max · n"]
                    + [cname(c) for c in wconts]) + " |")
                lines.append("|" + "---|" * (len(wconts) + 1))
                for mn in metrics:
                    row = [mn]
                    for cid in wconts:
                        m = table.get((cid, w, drive), {}).get(mn)
                        row.append(
                            (f"{m['median']:.3g} {m['unit']} · "
                             f"{m['min']:.3g}..{m['max']:.3g} · "
                             f"n={m['n']}") if m else "—")
                    lines.append("| " + " | ".join(row) + " |")
                    row = ["× WaterUI"]
                    wm = table.get(("waterui", w, drive), {}).get(mn)
                    for cid in wconts:
                        m = table.get((cid, w, drive), {}).get(mn)
                        if m and wm and wm["median"]:
                            row.append("1.00" if cid == "waterui"
                                       else ("—" if cid in noncomparable
                                             else f"{m['median']/wm['median']:.2f}"))
                        else:
                            row.append("—")
                    lines.append("| " + " | ".join(row) + " |")
                lines.append("")
        # Render-server accounting: app CPU vs render-server CPU per cell,
        # external and identical for every contestant (defence for W3-type
        # workloads whose real work happens in the compositor, not the app).
        rs = [r for r in results["runs"]
              if r.get("platform") == plat
              and ("renderserver_cpu_s" in r or "app_cpu_window_s" in r
                   or "frames" in r)]
        if rs:
            lines += [f"### {plat} — render server & frames", "",
                      "| contestant | workload | rep | app CPU s | "
                      "render-server CPU s | frames |",
                      "|---|---|---|---|---|---|"]
            for r in rs:
                lines.append(
                    f"| {cname(r['contestant'])} | {r.get('workload')} "
                    f"| {r.get('repeat')} | "
                    f"{r.get('app_cpu_window_s', '—')} | "
                    f"{r.get('renderserver_cpu_s', '—')} "
                    f"({r.get('renderserver', '?')}) | "
                    f"{r.get('frames', '—')} |")
            lines.append("")

        # capacity workloads (W5/W6): per-contestant largest step inside the
        # frame budget + per-step percentiles — one launch per pinned step,
        # the evidence lives in the .trace files
        caps = [r for r in results["runs"]
                if r.get("platform") == plat and r.get("capacity")]
        if caps:
            lines += [f"### {plat} — capacity (W5/W6)", "",
                      "| contestant | workload | capacity@120Hz | capacity@60Hz |",
                      "|---|---|---|---|"]
            for r in caps:
                steps = r["capacity"].get("steps", [])
                framed = any(st.get("frames") for st in steps)
                c0 = (r["capacity"].get("capacity_120hz", 0)
                      if framed else "— (no frame source)")
                c6 = (r["capacity"].get("capacity_60hz", 0)
                      if framed else "—")
                lines.append(f"| {cname(r['contestant'])} | {r['workload']} "
                             f"(r{r.get('repeat')}) | {c0} | {c6} |")
            lines.append("")
            for r in caps:
                lines.append(f"#### {cname(r['contestant'])} {r['workload']} "
                             f"rep {r.get('repeat')} — per-step")
                lines.append("| step | n | frames | p50 ms | p99 ms | "
                             "%in 8.33ms | %in 16.67ms | CPU ms/frame | "
                             "app CPU ms | render CPU ms |")
                lines.append("|---|---|---|---|---|---|---|---|---|---|")
                for st in r["capacity"].get("steps", []):
                    lines.append(
                        f"| {st['step']} | {st['n']} | "
                        f"{st.get('frames', '—')} "
                        f"| {st.get('p50_ms') or '—'} "
                        f"| {st.get('p99_ms') or '—'} "
                        f"| {st.get('in8.33ms_pct') or '—'} "
                        f"| {st.get('in16.67ms_pct') or '—'} "
                        f"| {st.get('cpu_ms_per_frame') or '—'} "
                        f"| {st.get('cpu_ms', '—')} "
                        f"| {st.get('render_cpu_ms', '—')} |")
                for e in r["capacity"].get("errors", []):
                    lines.append(f"- trace error: {e}")
                lines.append("")

        # failed cells: never dropped silently
        errs = [r for r in results["runs"]
                if r.get("platform") == plat and "error" in r]
        if errs:
            lines += [f"### {plat} — errors / limitations", "",
                      "| contestant | workload | repeat | error |",
                      "|---|---|---|---|"]
            for r in errs:
                e = r["error"].replace("\n", " ").replace("|", "\\|")
                lines.append(f"| {cname(r['contestant'])} | {r.get('workload')} | "
                             f"{r.get('repeat')} | {e[:300]} |")
            lines.append("")
        # rep-floor sufficiency: every required contestant/workload cell
        # needs >= MIN_REPS successful reps; failed attempts stay in the
        # errors table above and never count toward the floor
        gaps = completeness(results, plat)
        if gaps:
            incomplete.append(plat)
            lines += [
                f"### {plat} — DATASET INCOMPLETE "
                f"(cells need ≥{MIN_REPS} successful reps)", "",
                "| contestant | workload | drive | ok/attempts | reason |",
                "|---|---|---|---|---|"]
            for (cid, w, dr), ok, att, why in gaps:
                lines.append(f"| {cname(cid)} | {w} | {dr} | "
                             f"{ok}/{att} | {why} |")
            lines.append("")
    notes = MANIFEST.get("notes") or []
    if notes:
        lines += ["## Harness & scope notes", ""]
        lines += [f"- {n}" for n in notes]
        lines.append("")
    lines += ["## Raw results", "",
              "Per-run samples (including all failed attempts verbatim) are in",
              f"`{Path(args.input).name}` shipped next to this report. "
              "`harness.diff` carries the exact code state of the harness "
              "(`git diff` of `benchmarks/competitive/apple` vs `dev`).", ""]
    if incomplete:
        lines += [
            "## DATASET INCOMPLETE", "",
            "This report does not meet the ≥5-successful-reps floor for "
            "every required cell on: " + ", ".join(incomplete) + ". "
            "Missing, all-failed and mixed-attempt cells are listed "
            "above; raw failed attempts are retained in the results "
            "JSON and the per-platform errors tables.", ""]
    out = Path(args.out)
    out.write_text("\n".join(lines) + "\n")
    print(f"report → {out}")
    if incomplete:
        raise SystemExit(
            "dataset incomplete on " + ", ".join(incomplete)
            + " — see DATASET INCOMPLETE sections in the report")


# ---------------------------------------------------------------- main

def cmd_attribution(args):
    """Print, as JSON, how one all-process Animation Hitches trace
    attributes frames to one contestant: its owned pids, the swap join,
    and the owned present series next to every present on the same
    display, over the whole recording (no window, no gating) — the
    evidence that the join is sound for that contestant's rendering
    model. Signposts of the dev.bench subsystem are listed as found."""
    app = Path(args.app)
    _, main_rel = bundle_executable(app, args.platform)
    att = trace_attribution(Path(args.trace), app.name, main_rel)
    frames, err = _export_table(Path(args.trace), FRAMES_SCHEMA)
    if frames is None:
        raise SystemExit(f"{FRAMES_SCHEMA}: {err}")
    display_all = sorted(
        _int(f["start"], "start") + _int(f["duration"], "duration")
        for f in frames
        if f["duration"] is not None and f["display"] == att["display"])
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
    print(json.dumps({**att, "owned_presents": len(owned),
                      "span_ms": span,
                      "owned": stats(owned),
                      "display_all": stats(display_all),
                      "dev_bench_signposts": marks},
                     indent=2, default=str))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("build")
    b.add_argument("--platform", choices=["ios-sim", "macos", "ios-device"],
                   required=True)
    b.add_argument("--sim-udid", default=None,
                   help="iOS Simulator UDID for ios-sim destinations "
                        "(default: newest available iPhone simulator)")
    b.set_defaults(f=cmd_build)

    bs = sub.add_parser("bootstrap")
    bs.add_argument("--platform", choices=["ios-sim", "macos",
                                           "ios-device"], required=True)
    bs.add_argument("--sim-udid", default=None)
    bs.set_defaults(f=cmd_bootstrap)

    r = sub.add_parser("run-local")
    r.add_argument("--platform", choices=["ios-sim", "macos"], required=True)
    r.add_argument("--repeats", type=int, default=5)
    r.add_argument("--drive", default=None,
                   choices=["swipe", "wheel", "none", "tap"],
                   help="override the manifest's per-contestant drive for "
                        "every contestant (default: manifest decision — "
                        "swipe on iOS devices, wheel on macOS/simulator)")
    r.add_argument("--only", default=None,
                   help="comma-separated contestant ids (default: all); "
                        "re-runs append to the existing results file")
    r.add_argument("--workloads", default=None,
                   help="comma-separated workload ids (default: all w1-w6)")
    r.add_argument("--reps", default=None,
                   help="comma-separated rep indices to (re)measure "
                        "(default: --repeats reps starting at 0)")
    r.add_argument("--sim-udid", default=None,
                   help="iOS Simulator UDID (default: newest available "
                        "iPhone simulator on this host)")
    r.add_argument("--out", default=None)
    r.set_defaults(f=cmd_run_local)

    d = sub.add_parser("device")
    d.add_argument("--artifacts", required=True)
    d.add_argument("--udid", default=None,
                   help="attached iOS device UDID (default: the single "
                        "available device `devicectl list devices` "
                        "reports; required when several are attached)")
    d.add_argument("--identity", default=None,
                   help="substring selecting among multiple "
                        "'Apple Development' signing identities")
    d.add_argument("--team", default=None,
                   help="DEVELOPMENT_TEAM (default: the team of the "
                        "selected signing identity)")
    d.add_argument("--repeats", type=int, default=5)
    d.add_argument("--only", default=None,
                   help="comma-separated contestant ids (default: all)")
    d.add_argument("--reps", default=None,
                   help="comma-separated rep indices to (re)measure "
                        "(default: --repeats reps starting at 0)")
    d.add_argument("--workloads", default=None,
                   help="comma-separated workload ids (default: all w1-w6)")
    d.add_argument("--out", default=None)
    d.set_defaults(f=cmd_device)

    rp = sub.add_parser("report")
    rp.add_argument("--input", default=str(RESULTS_DEFAULT))
    rp.add_argument("--out", default=str(ROOT / "build" / "report.md"))
    rp.set_defaults(f=cmd_report)

    fp = sub.add_parser(
        "fingerprint",
        help="print this host's measurement fingerprint — the value "
             "manifest measurement_host.hw_uuid must declare")
    fp.set_defaults(f=lambda a: print(json.dumps(
        host_evidence(), indent=2, sort_keys=True)))

    at = sub.add_parser(
        "attribution",
        help="print how a recorded all-process Animation Hitches trace "
             "attributes frames to one contestant (evidence, no window)")
    at.add_argument("--trace", required=True)
    at.add_argument("--app", required=True,
                    help="the contestant's .app bundle (its name and "
                         "CFBundleExecutable select its processes)")
    at.add_argument("--platform", choices=["macos", "ios-device"],
                    required=True)
    at.add_argument("--max-fps", type=float, required=True,
                    help="the display's refresh rate, for missed-vsync "
                         "counts")
    at.set_defaults(f=cmd_attribution)

    args = ap.parse_args()
    args.f(args)


if __name__ == "__main__":
    main()

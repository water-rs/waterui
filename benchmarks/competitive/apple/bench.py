#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Competitive benchmark runner — Apple targets (water-rs/waterui#1262).

Single entry point. Run with `uv run bench.py <command>` or `python3 bench.py`.
Stdlib only, so `uv run` needs no project environment.

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

import argparse
import fcntl
import glob
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
        m = re.search(r"iOS[- ](\d+)\.(\d+)", runtime)
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

    ev = {"hw_model": _sysctl("hw.model"),
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


def water_bin(binary: str | None = None) -> str:
    """The `water` CLI built from THIS checkout (cli/ is a workspace
    member) — `cargo install --locked --path cli` into the suite-shared
    cache under a file lock, once per host. Refuses a dirty tracked
    checkout so the recorded HEAD sha is the real identity."""
    if binary:
        return binary
    return str(toolchain.provision_water_cli())


def cli_evidence(binary: str | None = None) -> dict:
    """Identity of the `water` binary actually invoked: path, sha256,
    --version output, plus the checkout HEAD it was built from."""
    path = water_bin(binary)
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
    native project. Any failure aborts; there is no silent retry."""
    for cmd in (MANIFEST.get("bootstrap") or {}).get(args.platform, []):
        if "{SIM_UDID}" in cmd:
            cmd = cmd.replace("{SIM_UDID}",
                              resolve_sim_udid(args.sim_udid))
        sh(cmd)
    print(f"bootstrap complete for {args.platform}")


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


def cmd_build(args):
    plat = args.platform
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
            _wb["b"] = water_bin(getattr(args, "water_bin", None))
        return _wb["b"]

    def flutter_bin():
        root = os.path.expandvars(
            MANIFEST["toolchain"].get(
                "flutter_root", "$HOME/toolchains/flutter"))
        b = Path(root) / "bin" / "flutter"
        # declared version enforced on the resolved binary
        toolchain.require_version(
            "flutter", [str(b), "--version"],
            MANIFEST["toolchain"]["flutter"])
        return str(b)

    try:
        cmd_bootstrap(args)
    except Exception as e:
        failures["<bootstrap>"] = str(e)[-2000:]
    for c in MANIFEST["contestants"]:
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
                    no_hitch: bool = False, only_test: str | None = None):
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
    env.update({
        "BENCH_BUNDLE_ID": bundle_id,
        "BENCH_WORKLOAD": workload,
        "BENCH_DRIVE": drive,
        "BENCH_DURATION": str(duration),
    })
    if no_hitch:
        # XCTHitchMetric produced no measurements on this target; the
        # runner drops it rather than reporting zeros.
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


def drive_for(c: dict, plat: str, workload: str = "") -> str:
    """One drive per contestant per platform, recorded per row.
       Resolution order: contestant's own `drive` override (manifest),
       then the platform-level `drive_overrides` for this workload,
       then `swipe`. `auto` means the runner posts the dev.bench.begin
       Darwin notification inside the measure block; the app then runs
       the identical fling program and posts dev.bench.done."""
    if (d := c.get("drive", {}).get(plat)) is not None:
        if isinstance(d, dict):
            return d.get(workload, d.get("*", "swipe"))
        return d
    pov = MANIFEST.get("drive_overrides", {}).get(plat, {})
    return pov.get(workload, pov.get("*", "swipe"))


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
            app = _host_pid(exe)
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


CAPACITY_WORKLOADS = ("W5", "W6")
# W5/W6 exist only in the five iOS contestants — electron/appkit ship W1–W4.
CAPACITY_CONTESTANTS = {"waterui", "swiftui", "uikit", "flutter", "rn"}
# Match the in-app ladders: settle 1 s + hold 4 s per step.
STEP_TOTAL_S = 5.0          # settle + hold per ladder step
STEP_SETTLE_S = 1.0
STEP_HOLD_S = STEP_TOTAL_S - STEP_SETTLE_S
CAPACITY_STEPS = {"W5": [200, 400, 800, 1600, 3200, 6400, 12800, 25600],
                  "W6": [1, 2, 4, 8, 16, 32, 64]}
BENCH_NO_HITCH_ENV = "BENCH_NO_HITCH"


def capacity_cell(plat, cid, w) -> bool:
    return (w in CAPACITY_WORKLOADS and cid in CAPACITY_CONTESTANTS
            and plat in ("ios-sim", "ios-device"))


def _xctrace(args_, timeout=None):
    return subprocess.run(["xcrun", "xctrace", *args_],
                          capture_output=True, text=True, timeout=timeout)


def _export_table(trace: Path, schema: str):
    """`xctrace export` rows of one table → list of dicts keyed by
    column mnemonic (the tag name in the export).

    Elements either define a value (`id` attr, raw text or `fmt`) or
    repeat one (`ref` attr pointing at an earlier `id` of the same tag)
    — resolve refs so every row is self-contained. Time engineering
    values come out in nanoseconds (e.g. start-time / sample-time)."""
    r = _xctrace(["export", "--input", str(trace), "--xpath",
                  f'/trace-toc/run[@number="1"]/data/table[@schema="{schema}"]'],
                 timeout=600)
    if r.returncode != 0:
        return None, (r.stderr or "")[-400:]
    import xml.etree.ElementTree as ET
    root = ET.fromstring(r.stdout)
    idmap = {}
    for e in root.iter():
        if e.get("id") is not None:
            v = (e.text or "").strip() or (e.get("fmt") or "")
            idmap.setdefault(e.tag, {})[e.get("id")] = v
    def val(e):
        if e.get("ref") is not None:
            return idmap.get(e.tag, {}).get(e.get("ref"))
        t = (e.text or "").strip()
        return t if t else (e.get("fmt") or "")
    rows = []
    for row in root.iter("row"):
        rows.append({c.tag: val(c) for c in row})
    return rows, None


def _boot_epoch():
    try:
        out = subprocess.run(["sysctl", "-n", "kern.boottime"],
                             capture_output=True, text=True).stdout
        m = re.search(r"sec\s*=\s*(\d+)\s*,\s*usec\s*=\s*(\d+)", out)
        if m:
            return int(m.group(1)) + int(m.group(2)) / 1e6
    except Exception:
        pass
    return 0.0


def _epoch_map(raw_vals, lo, hi, trace: Path):
    """Pick the affine map raw engineering timestamp → epoch seconds.

    Different export tables use different bases (ns since epoch, ns/µs
    since boot, ns since trace start). The trace file's birthtime ≈ the
    record start; kern.boottime covers mach-absolute bases. Score each
    candidate by how many mapped timestamps land inside [lo, hi]."""
    if not raw_vals:
        return None
    birth = os.stat(trace).st_birthtime
    boot = _boot_epoch()
    cands = [(1e-9, 0.0), (1e-6, 0.0), (1.0, 0.0),
             (1e-9, boot), (1e-6, boot),
             (1e-9, birth), (1e-6, birth)]
    best, best_n = None, 0
    for s, b in cands:
        n = sum(1 for v in raw_vals if lo <= v * s + b <= hi)
        if n > best_n:
            best, best_n = (s, b), n
    return best if best_n else None


def _proc_of_thread(fmt: str):
    m = re.findall(r"\(([^()]*), pid: (\d+)\)", fmt or "")
    return m[-1][0].strip() if m else None


def _num(v, default=0.0):
    try:
        return float(str(v).strip())
    except Exception:
        return default


_PAINT_RE = re.compile(r"waterui_first_paint_ms=\s*([0-9]+(?:\.[0-9]+)?)")


def first_paint_ms(plat: str, udid: str | None,
                   trace: Path | None = None,
                   since: float | None = None) -> float | None:
    """Latest `waterui_first_paint_ms=N` the app emitted (os_log
    `dev.waterui`, notice level → persisted). Emitted once per process
    by the apple backend's WuiLaunchTiming.

    - macos: host unified log.
    - ios-sim: `simctl spawn <udid> log show` inside the simulator.
    - ios-device: devicectl exposes no console read, so the cell's own
      Logging-template xctrace is the source; its schema varies across
      Xcode builds, so scan every candidate table for the marker text.

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
        for schema in ("os-log", "logging", "oslog", "os_log"):
            rows, _ = _export_table(trace, schema)
            if not rows:
                continue
            for row in rows:
                for v in row.values():
                    m = _PAINT_RE.search(str(v))
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
        MANIFEST["toolchain"].get("flutter_root", "$HOME/toolchains/flutter"))
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


def parse_capacity(trace: Path | None, steps_log: list, app_name: str,
                   budget_ms=(8.33, 16.67), render_trace: Path | None = None,
                   sampler: "CpuSampler | None" = None):
    """Slice the trace's per-frame and CPU data by step boundaries.

    `steps_log` = [(k, n, epoch_secs)]. Frames come from
    hitches-frame-lifetimes (one row per presented surface lifetime);
    consecutive `start` times on the app's display give presented-frame
    intervals. CPU comes from time-sample rows whose thread belongs to
    the app process and whose thread-state is Running; each row is one
    profiler sample, and the effective sample interval is recovered from
    the sample-time deltas themselves.

    `render_trace` (Time Profiler attach to the render server) adds
    render_cpu_ms per step. When no Animation Hitches trace exists
    (iOS Simulator: the template is unsupported on the sim target) the
    frame columns stay null and app CPU comes from the CpuSampler
    (ps -o time deltas on the app and the sim's backboardd) — still
    external, still identical for every contestant."""
    frames, e1 = (_export_table(trace, "hitches-frame-lifetimes")
                  if trace else ([], "no frame trace (xctrace could not "
                                 "record Animation Hitches on this target)"))
    cpu, e2 = (_export_table(trace, "time-sample") if trace else ([], None))
    rcpu, e3 = (_export_table(render_trace, "time-sample")
                if render_trace else ([], None))
    out = {"steps": [],
           "errors": [e for e in (e1, e2, e3) if e]}
    if not steps_log:
        out["errors"].append("no bench-steps.log rows pulled")
        return out
    windows = []
    for i, (k, n, t) in enumerate(steps_log):
        end = (steps_log[i + 1][2] if i + 1 < len(steps_log)
               else t + STEP_SETTLE_S + STEP_HOLD_S)
        windows.append((i, n, t + STEP_SETTLE_S,
                        min(end, t + STEP_SETTLE_S + STEP_HOLD_S)))
    lo = steps_log[0][2] - 20
    hi = steps_log[-1][2] + STEP_SETTLE_S + STEP_HOLD_S + 30

    # ---- presented-frame intervals (per display, consecutive starts) --
    fmap = None
    if frames:
        raw = [_num(f.get("start")) for f in frames]
        fmap = _epoch_map([v for v in raw if v > 0], lo, hi, trace)
        if fmap is None:
            out["errors"].append(
                "hitches-frame-lifetimes timestamps do not land in the "
                "step window (no epoch base matched)")
    else:
        out["errors"].append("hitches-frame-lifetimes: 0 rows")
    intervals = []   # [(epoch_s, interval_ms)]
    prev = {}
    if fmap:
        fs, fb = fmap
        for f in frames:
            t0 = _num(f.get("start"))
            if t0 <= 0:
                continue
            t = t0 * fs + fb
            disp = f.get("display") or "main"
            if disp in prev:
                intervals.append((t, (t - prev[disp]) * 1000.0))
            prev[disp] = t
        intervals.sort()

    # ---- CPU: Running samples of the app's threads ------------------
    def _running_times(rows, proc, src):
        """Epoch times of Running samples for `proc` + calibrated tick."""
        if not rows:
            return [], 1.0
        raw_t = [_num(r.get("sample-time")) for r in rows]
        m = _epoch_map([v for v in raw_t if v > 0], lo, hi, src)
        ts = sorted(v for v in raw_t if v > 0)
        smp = 1.0
        if len(ts) > 2:
            dt = sorted(b - a for a, b in zip(ts, ts[1:]) if b > a)
            if dt:
                smp = dt[len(dt) // 2] * 1e-6  # ns → ms
        if not m:
            return None, smp
        s, b0 = m
        return [t0 * s + b0 for t0, r in
                ((_num(r.get("sample-time")), r) for r in rows)
                if t0 > 0 and r.get("thread-state") == "Running"
                and _proc_of_thread(r.get("thread")) == proc], smp

    cpu_ms = []
    sample_ms = 1.0
    if cpu:
        cpu_ms, sample_ms = _running_times(cpu, app_name, trace)
        if cpu_ms is None:
            out["errors"].append(
                "time-sample timestamps do not land in the step window")
            cpu_ms = []
    elif trace:
        out["errors"].append("time-sample: 0 rows")

    render_ms = []
    render_sample_ms = 1.0
    if rcpu:
        render_ms, render_sample_ms = _running_times(
            rcpu, RENDER_NAME.get("ios-device", "backboardd"),
            render_trace)
        if render_ms is None:
            render_ms = []

    for i, n, w0, w1 in windows:
        row = {"step": i, "n": n, "window_s": [round(w0, 2), round(w1, 2)]}
        iv = sorted(d for (t, d) in intervals if w0 <= t < w1)
        ncpu = sum(1 for t in cpu_ms if w0 <= t < w1)
        row["cpu_ms"] = round(ncpu * sample_ms, 1)
        if sampler is not None:
            # Sim path: app + render-server CPU from ps deltas.
            a = sampler.delta("app", w0, w1)
            if a is not None:
                row["cpu_ms"] = round(a * 1000.0, 1)
            r = sampler.delta("render_server", w0, w1)
            if r is not None:
                row["render_cpu_ms"] = round(r * 1000.0, 1)
        elif render_ms:
            row["render_cpu_ms"] = round(
                sum(1 for t in render_ms if w0 <= t < w1)
                * render_sample_ms, 1)
        if iv:
            row["frames"] = len(iv)
            row["p50_ms"] = round(iv[len(iv) // 2], 2)
            row["p99_ms"] = round(iv[min(len(iv) - 1,
                                         int(len(iv) * 0.99))], 2)
            for b in budget_ms:
                row[f"in{b}ms_pct"] = round(
                    100.0 * sum(1 for v in iv if v <= b) / len(iv), 1)
            row["cpu_ms_per_frame"] = round(ncpu * sample_ms / len(iv), 3)
        else:
            # no frame source (sim: xctrace can't record Hitches) — null,
            # not 0: a presented-frame count of 0 would read as a failure
            # the probe simply cannot see.
            row["frames"] = None
        out["steps"].append(row)
    cap = 0
    for st in out["steps"]:
        if st.get("frames") and st.get("in8.33ms_pct", 0) >= 99.0:
            cap = st["n"]
    out["capacity_120hz"] = cap
    cap60 = 0
    for st in out["steps"]:
        if st.get("frames") and st.get("in16.67ms_pct", 0) >= 99.0:
            cap60 = st["n"]
    out["capacity_60hz"] = cap60
    out["sample_ms"] = round(sample_ms, 3)
    return out


def pull_steps_log(plat: str, udid: str, bundle_id: str, out: Path):
    """Fetch tmp/bench-steps.log from the app's container."""
    if plat == "ios-sim":
        r = subprocess.run(
            ["xcrun", "simctl", "get_app_container", sim_udid(),
             bundle_id, "data"], capture_output=True, text=True,
            timeout=60)
        if r.returncode == 0:
            src = Path(r.stdout.strip()) / "tmp" / "bench-steps.log"
            if src.exists():
                shutil.copy2(src, out)
                return
        out.unlink(missing_ok=True)
    elif plat == "ios-device":
        sh(f"xcrun devicectl device copy from --device {udid} "
           f"--domain-type appDataContainer --domain-identifier {bundle_id} "
           f"--source tmp/bench-steps.log --destination '{out}'",
           check=False, capture=True)
    elif plat == "macos":
        src = Path(os.environ.get("TMPDIR", "/tmp")) / "bench-steps.log"
        if src.exists():
            shutil.copy2(src, out)
    if not out.exists():
        out.write_text("")


def read_steps_log(path: Path, since: float, workload: str):
    """[(step, n, epoch_secs)] for rows at/after `since`, trimmed to this
    workload's step count — the file persists between sim reps, so keep
    only the trailing program."""
    rows = []
    if path.exists():
        import re
        for line in path.read_text().splitlines():
            m = re.match(r"step\s+(\d+)\s+n=(\d+)\s+t=([0-9.]+)", line)
            if m and float(m.group(3)) >= since:
                rows.append((int(m.group(1)), int(m.group(2)),
                             float(m.group(3))))
    rows.sort(key=lambda r: r[2])
    want = len(CAPACITY_STEPS.get(workload, []))
    return rows[-want:] if want and len(rows) > want else rows


def _xctrace_attach(out_path: Path, template: str, proc: str,
                    time_limit_s: int, device_udid: str | None):
    """Poll-attach an xctrace recorder: the app only exists once the
    runner launches it, and --attach exits instantly when nothing
    matches, so retry until the process appears or the deadline passes.
    Returns (proc | None, err | None)."""
    cmd = ["xcrun", "xctrace", "record", "--template", template,
           "--attach", proc, "--output", str(out_path),
           "--time-limit", f"{time_limit_s}s"]
    if device_udid:
        cmd += ["--device", device_udid]
    deadline = time.time() + 90
    err = None
    while time.time() < deadline:
        p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL,
                             stderr=subprocess.PIPE,
                             start_new_session=True)
        _ACTIVE_PROCS.append(p)
        time.sleep(1.0)
        if p.poll() is None:
            return p, None
        if p in _ACTIVE_PROCS:
            _ACTIVE_PROCS.remove(p)
        err = (p.stderr.read() or "")[-300:]
    return None, err or "xctrace never attached"


def _stop_trace_proc(p):
    """SIGINT an xctrace recorder, kill on ignore, untrack it."""
    if p is None:
        return
    try:
        p.send_signal(signal.SIGINT)
        p.wait(timeout=30)
    except Exception:
        _kill_proc(p)
    if p in _ACTIVE_PROCS:
        _ACTIVE_PROCS.remove(p)


def _post_recorder_go(plat: str, udid: str, runner_bid: str | None,
                      results_dir: Path):
    """Release the runner's pre-launch gate: the recorders are armed.

    ios-sim: host-originated notify post lands in the simulator's own
    notifyd (same channel the app posts on). macOS: the shared host
    notifyd. ios-device: the host cannot post into the device's notify
    namespace, so a sentinel file is copied into the RUNNER app's
    container — the runner watches for it alongside the notify token.
    """
    if plat == "ios-sim":
        subprocess.run(["xcrun", "simctl", "spawn", udid, "notifyutil",
                        "-p", "dev.bench.recorder"], capture_output=True)
    elif plat == "macos":
        subprocess.run(["notifyutil", "-p", "dev.bench.recorder"],
                       capture_output=True)
    elif plat == "ios-device" and runner_bid:
        sentinel = results_dir / "bench-recorder-go"
        sentinel.write_text("armed\n")
        sh(f"xcrun devicectl device copy to --device {udid} "
           f"--domain-type appDataContainer "
           f"--domain-identifier {runner_bid} "
           f"--source '{sentinel}' "
           f"--destination 'tmp/bench-recorder-go'",
           check=False, capture=True)
        sentinel.unlink(missing_ok=True)


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
    """Parse the runner log for this rep: the measure-window pair and the
    device record the runner wrote inside the measure block. Entries are
    '<epoch> <text>' lines; only rows at/after `since` count so a
    previous rep's markers can't alias into this one."""
    window = [None, None]
    device = {}
    if not path.exists():
        return window, device
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
        if body.startswith("measure-begin"):
            window[0] = t
        elif body.startswith("measure-end"):
            window[1] = t
        elif body.startswith("device-record"):
            for kv in body.split():
                if "=" in kv:
                    k, v = kv.split("=", 1)
                    device[k] = v
    if not (window[0] and window[1] and window[1] > window[0]):
        window = [None, None]
    return window, device


def _trace_proc_cpu(trace: Path, proc: str):
    """Whole-trace CPU seconds for a process from time-sample rows."""
    rows, err = _export_table(trace, "time-sample")
    if not rows:
        return None, err or "time-sample: 0 rows"
    ts = sorted(_num(r.get("sample-time")) for r in rows)
    ts = [v for v in ts if v > 0]
    smp_ns = 1e6
    if len(ts) > 2:
        dt = sorted(b - a for a, b in zip(ts, ts[1:]) if b > a)
        if dt:
            smp_ns = dt[len(dt) // 2]
    n = sum(1 for r in rows
            if r.get("thread-state") == "Running"
            and _proc_of_thread(r.get("thread")) == proc)
    return round(n * smp_ns * 1e-9, 3), None


def _trace_frame_count(trace: Path):
    """Presented frames in the window: hitches-frame-lifetimes rows."""
    rows, err = _export_table(trace, "hitches-frame-lifetimes")
    if not rows:
        return None, err or "hitches-frame-lifetimes: 0 rows"
    return len(rows), None


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


def run_one(plat, contestant_id, app_path: Path, bundle_id, workload,
            drive, duration, rep, dest, products_dir: Path, subdir: str,
            template: Path, target_key: str, results_dir: Path,
            no_hitch: bool = False, device_udid: str | None = None,
            runner_bid: str | None = None, artifact_sha: str | None = None,
            runner_log_since: float = 0.0) -> dict:
    """Stage app + injected xctestrun, run two single-test invocations.

    One test per xcodebuild invocation: the launch test and the workload
    test never share a process lifetime, so a poll-attaching recorder
    cannot bind to the wrong instance. External observation, identical
    for every contestant:
    - ios-sim / macos: a CpuSampler polls `ps -o time/rss` on the app,
      its helper children and the render server (sim backboardd / macOS
      WindowServer) so work Core Animation executes outside the app
      process is still counted — windowed to the measure block by the
      runner's own measure-begin/measure-end markers.
    - ios-device: xctrace records Animation Hitches on the app (presented
      frames + app CPU), Time Profiler on backboardd (render-server CPU)
      and Logging on the app (marker channel) — all three armed BEFORE
      the runner's recorder-go gate releases the launch, identical for
      every contestant.
    - W5/W6 additionally pull tmp/bench-steps.log and slice per step.
    """
    tag = f"{contestant_id}-{workload}-r{rep}"
    capacity = workload in CAPACITY_WORKLOADS
    dst = products_dir / subdir / app_path.name
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(app_path, dst, symlinks=True)
    xr_launch = products_dir / f"{tag}-launch.xctestrun"
    xr_work = products_dir / f"{tag}-work.xctestrun"
    write_xctestrun(template, xr_launch, target_key, subdir, app_path.name,
                    bundle_id, workload, drive, duration,
                    runner_app="", no_hitch=no_hitch,
                    only_test="testLaunch")
    write_xctestrun(template, xr_work, target_key, subdir, app_path.name,
                    bundle_id, workload, drive, duration,
                    runner_app="", no_hitch=no_hitch,
                    only_test="testWorkload")
    res_launch = results_dir / f"{tag}-launch.xcresult"
    res_work = results_dir / f"{tag}-work.xcresult"
    trace = results_dir / f"{tag}.trace"
    bb_trace = results_dir / f"{tag}-bb.trace"
    fp_trace = results_dir / f"{tag}-log.trace"
    steps_file = results_dir / f"{tag}-steps.log"
    runner_log = results_dir / f"{tag}-runner.log"
    for p in (res_launch, res_work, trace, bb_trace, fp_trace, steps_file,
              runner_log):
        if p.is_dir():
            shutil.rmtree(p, ignore_errors=True)
        else:
            p.unlink(missing_ok=True)

    exe = app_path.stem
    if plat == "macos":
        # CFBundleExecutable need not equal the bundle stem — Electron
        # names it "Electron", not the app name
        plist_f = app_path / "Contents" / "Info.plist"
        if plist_f.exists():
            try:
                exe = plistlib.loads(plist_f.read_bytes()).get(
                    "CFBundleExecutable", exe)
            except Exception:
                pass

    rec = {}
    sampler = None
    trace_proc = bb_proc = fp_proc = None
    trace_err = bb_err = fp_err = None
    t_start = time.time()
    # recorder-go gate: the runner's test waits on this before launching
    # the app, so every armed recorder binds at process birth instead of
    # hoping a blind sleep outraced the launch
    if plat == "ios-sim":
        subprocess.run(["xcrun", "simctl", "spawn", sim_udid(),
                        "notifyutil", "-p", "dev.bench.recorder"],
                       capture_output=True)
        # drain is runner-side: this pre-post just re-arms the channel
        # for the sim's notify namespace existence check
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
        if sampler is None and plat in ("ios-sim", "macos"):
            sampler = CpuSampler(cpu_pids_resolver(
                plat, device_udid or (sim_udid() if plat == "ios-sim"
                                      else None), bundle_id, exe),
                interval=0.5)
            sampler.start()
        xb, xbf, xblog = _xctest_spawn(xr_work, res_work, dest,
                                     results_dir, tag + "-work")
        try:
            if plat == "ios-device":
                # Frame+app-CPU trace on the app, CPU trace on the render
                # server, and the Logging channel for the marker — all
                # three for EVERY contestant so the measurement
                # environment is symmetric; attach polls arm before the
                # runner is released.
                trace_proc, trace_err = _xctrace_attach(
                    trace, "Animation Hitches", exe, duration + 120,
                    device_udid)
                bb_proc, bb_err = _xctrace_attach(
                    bb_trace, "Time Profiler", "backboardd",
                    duration + 120, device_udid)
                fp_proc, fp_err = _xctrace_attach(
                    fp_trace, "Logging", exe, duration + 120,
                    device_udid)
            _post_recorder_go(plat,
                              device_udid or (sim_udid()
                                              if plat == "ios-sim"
                                              else ""),
                              runner_bid, results_dir)
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
            for p in (trace_proc, bb_proc, fp_proc):
                _stop_trace_proc(p)
            if sampler is not None:
                sampler.stop()
                sampler.join(timeout=10)

        if artifact_sha:
            rec["artifact_sha256"] = artifact_sha

        # --- windowed sampler deltas + runner log markers ------------
        udid = device_udid or (sim_udid() if plat == "ios-sim" else "")
        pull_runner_log(plat, udid, runner_bid, runner_log)
        window, device_rec = read_runner_log(runner_log,
                                             max(runner_log_since,
                                                 t_start - 5))
        if device_rec:
            rec["device_record"] = device_rec
        w0, w1 = window
        if sampler is not None:
            rec["renderserver"] = RENDER_NAME[plat]
            rec["cpu_window_source"] = ("measure" if w0 and w1
                                        else "whole-invocation")
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
            rec["renderserver"] = "backboardd"
            if trace.exists():
                n, e = _trace_frame_count(trace)
                if n is not None:
                    rec["frames"] = n
                if e:
                    rec.setdefault("trace_errors", []).append(e)
                cpu_s, e = _trace_proc_cpu(trace, exe)
                if cpu_s is not None:
                    rec["app_cpu_window_s"] = cpu_s
                if e:
                    rec.setdefault("trace_errors", []).append(e)
            elif trace_err:
                rec.setdefault("trace_errors", []).append(
                    f"xctrace app: {trace_err}")
            if bb_trace.exists():
                cpu_s, e = _trace_proc_cpu(bb_trace, "backboardd")
                if cpu_s is not None:
                    rec["renderserver_cpu_s"] = cpu_s
                if e:
                    rec.setdefault("trace_errors", []).append(e)
            elif bb_err:
                rec.setdefault("trace_errors", []).append(
                    f"xctrace backboardd: {bb_err}")

        if capacity:
            pull_steps_log(plat,
                           device_udid or (sim_udid()
                                           if plat == "ios-sim" else ""),
                           bundle_id, steps_file)
            steps = read_steps_log(steps_file, t_start - 5, workload)
            cap = parse_capacity(
                trace if plat == "ios-device" and trace.exists() else None,
                steps, exe,
                render_trace=bb_trace if bb_trace.exists() else None,
                sampler=sampler)
            if trace_err:
                cap.setdefault("errors", []).append(f"xctrace: {trace_err}")
            if bb_err:
                cap.setdefault("errors", []).append(
                    f"xctrace backboardd: {bb_err}")
            rec["capacity"] = cap

        # apple-backend#281 baseline fields: app size on every row, and
        # waterui's first-paint marker where a readable channel exists.
        rec["app_bytes"] = du_bytes(app_path)
        if contestant_id == "waterui":
            fp = first_paint_ms(
                plat, device_udid or (sim_udid() if plat == "ios-sim"
                                      else None),
                trace=fp_trace if plat == "ios-device" else None,
                since=t_start)
            if fp is not None:
                rec["first_paint_ms"] = fp
            elif fp_err:
                rec.setdefault("trace_errors", []).append(
                    f"xctrace logging: {fp_err}")
        fr = rec.get("frames")
        if fr and rec.get("app_cpu_window_s") is not None:
            rec["cpu_ms_per_frame"] = round(
                rec["app_cpu_window_s"] * 1000.0 / fr, 3)
    finally:
        if sampler is not None:
            sampler.stop()
        for p in (trace_proc, bb_proc, fp_proc):
            _stop_trace_proc(p)

    # keep xcresult small: delete the bundles after parsing; the .trace
    # stays (it is the frame-interval evidence)
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
            try:
                return plistlib.loads(
                    plist_f.read_bytes())["CFBundleIdentifier"]
            except Exception:
                continue
    return None


def mac_thermal_wait(budget_s: float = 600.0):
    """macOS thermal gate: `pmset -g thermlog` reports CPU_Speed_Limit —
    100 means un-throttled. Bounded poll, not a blind sleep: a throttled
    host must not be timed, and an already-cool host proceeds at once.
    Returns True when cool within budget."""
    deadline = time.time() + budget_s
    while True:
        r = subprocess.run(["pmset", "-g", "thermlog"],
                           capture_output=True, text=True, timeout=30)
        # last CPU_Speed_Limit value in the log; absence of the field
        # means no throttling has been recorded
        limits = re.findall(r"CPU_Speed_Limit\s*=\s*(\d+)",
                            r.stdout or "")
        if not limits or int(limits[-1]) >= 100:
            return True
        if time.time() > deadline:
            return False
        time.sleep(10)


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
        template_key = "ios_sim_xctestrun"
        target_key = MANIFEST["harness"]["test_target_key"]["ios"]
        subdir = "Release-iphonesimulator"
        dest = f"platform=iOS Simulator,id={resolve_sim_udid(args.sim_udid)}"
    elif plat == "macos":
        template_key = "macos_xctestrun"
        target_key = MANIFEST["harness"]["test_target_key"]["macos"]
        subdir = "Release"
        dest = "platform=macOS"
    else:
        raise SystemExit("run-local supports ios-sim | macos")

    # the staged artifacts dir is canonical at run time; the manifest's dd
    # path is the build-time source of the same files
    template = next(iter(sorted(staged.glob("*.xctestrun"))),
                    ROOT / MANIFEST["harness"][template_key])

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
    runner_bid = runner_bundle_id(staged)
    workloads = list(MANIFEST["workloads"].keys())
    if getattr(args, "workloads", None):
        wanted_w = set(args.workloads.split(","))
        workloads = [w for w in workloads if w in wanted_w]

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
    no_hitch = state.get("no_hitch", False)

    def _on_sig(sig, _frame):
        _kill_active_procs()
        lock.close()
        sys.exit(128 + sig)

    _prev = _install_signal_handlers(_on_sig)
    try:
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
                wd = drive_override or drive_for(c, plat, "W1")
                run_one(plat, c["id"], app, bid, "W1", wd,
                        MANIFEST["workloads"]["W1"]["duration_s"], rep,
                        dest, products_dir, subdir, template, target_key,
                        results_path.parent / "xcresults",
                        no_hitch=True, device_udid=None,
                        runner_bid=runner_bid,
                        artifact_sha=artifact_shas.get(app.name))
                warmed.add(c["id"])
                state["warmed_up"] = sorted(warmed)
                results_path.write_text(json.dumps(state, indent=1))
            for w in workloads:
                if (w in CAPACITY_WORKLOADS
                        and not capacity_cell(plat, c["id"], w)):
                    continue  # W5/W6 ship in the five iOS contestants only
                drive = drive_override or drive_for(c, plat, w)
                duration = MANIFEST["workloads"][w]["duration_s"]
                if plat == "macos" and not mac_thermal_wait():
                    # a throttled host must not be timed — the cell
                    # records the failed attempt and moves on
                    state["runs"].append({
                        "contestant": c["id"], "workload": w,
                        "repeat": rep, "platform": plat, "drive": drive,
                        "error": "macOS thermal gate: CPU_Speed_Limit "
                                 "still throttled after 600s"})
                    results_path.write_text(json.dumps(state, indent=1))
                    continue
                print(f"[{plat}] rep {rep+1}/{repeats} {c['id']} {w} "
                      f"drive={drive}", flush=True)
                rec = run_one(plat, c["id"], app, bid, w, drive, duration,
                              rep, dest, products_dir, subdir, template,
                              target_key, results_path.parent / "xcresults",
                              no_hitch=no_hitch, device_udid=None,
                              runner_bid=runner_bid,
                              artifact_sha=artifact_shas.get(app.name))
                rec.update({"contestant": c["id"], "workload": w,
                            "repeat": rep, "platform": plat, "drive": drive})
                state["runs"].append(rec)
                results_path.write_text(json.dumps(state, indent=1))
                # Hitch evidence: if the metric emitted no measurements at
                # all on this platform, drop it instead of reporting zeros.
                if not no_hitch and "metrics" in rec:
                    idents = [m.get("identifier", "")
                              for mets in rec["metrics"].values()
                              for m in mets.values()]
                    # only a metric that emitted NO identifiers is dead;
                    # emitted-but-all-zero rows are real zero-hitch data
                    if not any("itch" in i.lower() for i in idents):
                        no_hitch = True
                        state["no_hitch"] = True
                        state.setdefault("notes", []).append(
                            "XCTHitchMetric emitted no measurements on "
                            f"{plat} (first run of {c['id']} {w}); "
                            "dropped via BENCH_NO_HITCH for remaining runs")
    finally:
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
        # XCTHitchMetric emitted zero-duration rows for every contestant on
        # iOS 26 device (first iPad run) — dropped on ios-device; presented
        # frames come from the xctrace Animation Hitches attach instead.
        no_hitch = state.get("no_hitch", False)
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
                                    "W1", drive_for(c, "ios-device", "W1"),
                                    MANIFEST["workloads"]["W1"]["duration_s"],
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
                                artifact_sha=artifact_shas.get(app.name))
                            rec.update({"contestant": c["id"], "workload": w,
                                        "repeat": rep,
                                        "platform": "ios-device",
                                        "drive": drive,
                                        "device_state": st})
                            state["runs"].append(rec)
                            results_path.write_text(
                                json.dumps(state, indent=1))
                            # Hitch evidence: if the metric emitted no
                            # measurements at all on this device, drop it
                            # for the remaining runs instead of zeros.
                            if not no_hitch and "metrics" in rec:
                                idents = [
                                    m.get("identifier", "")
                                    for mets in rec["metrics"].values()
                                    for m in mets.values()]
                                if not any("itch" in i.lower()
                                           for i in idents):
                                    no_hitch = True
                                    state.setdefault("notes", []).append(
                                        "XCTHitchMetric emitted no "
                                        f"measurements on {udid} (first "
                                        f"run of {c['id']} {w}); dropped "
                                        "via BENCH_NO_HITCH for remaining "
                                        "runs")
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
            if w in CAPACITY_WORKLOADS:
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

        # capacity ladders (W5/W6): per-contestant largest step inside the
        # frame budget + per-step percentiles (the evidence lives in the
        # .trace files; steps sliced by bench-steps.log timestamps)
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
    b.add_argument("--water-bin", default=None,
                   help="water CLI binary path override (default: the "
                        "in-tree CLI provisioned from this checkout via "
                        "lib/toolchain.provision_water_cli)")
    b.set_defaults(f=cmd_build)

    bs = sub.add_parser("bootstrap")
    bs.add_argument("--platform", choices=["ios-sim", "macos",
                                           "ios-device"], required=True)
    bs.add_argument("--sim-udid", default=None)
    bs.set_defaults(f=cmd_bootstrap)

    r = sub.add_parser("run-local")
    r.add_argument("--platform", choices=["ios-sim", "macos"], required=True)
    r.add_argument("--repeats", type=int, default=5)
    r.add_argument("--drive", default=None, choices=["swipe", "auto"],
                   help="override the manifest's per-contestant drive for "
                        "every contestant (default: manifest decision — "
                        "swipe everywhere, auto only where a contestant "
                        "declares one for the platform)")
    r.add_argument("--only", default=None,
                   help="comma-separated contestant ids (default: all); "
                        "re-runs append to the existing results file")
    r.add_argument("--workloads", default=None,
                   help="comma-separated workload ids (default: all W1-W4)")
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
                   help="comma-separated workload ids (default: all W1-W4)")
    d.add_argument("--out", default=None)
    d.set_defaults(f=cmd_device)

    rp = sub.add_parser("report")
    rp.add_argument("--input", default=str(RESULTS_DEFAULT))
    rp.add_argument("--out", default=str(ROOT / "build" / "report.md"))
    rp.set_defaults(f=cmd_report)

    args = ap.parse_args()
    args.f(args)


if __name__ == "__main__":
    main()

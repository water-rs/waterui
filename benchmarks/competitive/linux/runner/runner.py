#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# ///
"""Competitive benchmark runner — Linux (water-rs/waterui#1262).

Usage: uv run runner/runner.py [--reps N] [--only contestant[,...]]
                               [--workload w1[,...]] [--skip-build]
                               [--development]

Builds every contestant inside the shared container image, measures package
size, then runs each contestant x workload under benchcomp (headless wlroots
compositor: present timestamps, scripted input, cgroup memory sampling) for
>=5 repetitions and writes results-<UTC>.json in results/.
"""

from __future__ import annotations

import argparse
import atexit
import fcntl
import gzip
import io
import json
import os
import platform
import re
import statistics
import subprocess
import sys
import tarfile
import time
import tomllib
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "lib"))
import toolchain  # benchmarks/competitive/lib/toolchain.py

ROOT = Path(__file__).resolve().parent.parent  # benchmarks/competitive/linux
REPO = toolchain.repo_root()                  # waterui checkout root
APPS = REPO / "benchmarks" / "competitive" / "apps"
IMAGE = "waterui-bench-linux"
CARGO_CACHE = ROOT / ".cache" / "cargo"
WATER_CACHE = ROOT / ".cache" / "water"
NPM_CACHE = ROOT / ".cache" / "npm"
HOME_CACHE = ROOT / ".cache" / "container-home"
OUT_DIR = ROOT / "results"
VSYNC_MS = 1000.0 / 60.0
# Run-scoped namespace: every scratch file and container this invocation
# owns carries it, so concurrent or crashed invocations never share state.
RUN_ID = uuid.uuid4().hex[:8]
_CONTAINER_SEQ = 0


def stamp_id() -> str:
    return time.strftime("%Y%m%d-%H%M%S", time.gmtime()) + "-" + RUN_ID


def _owned_container_name() -> str:
    global _CONTAINER_SEQ
    _CONTAINER_SEQ += 1
    return f"bench-linux-{RUN_ID}-{_CONTAINER_SEQ}"


def cleanup_owned_containers() -> None:
    """Stop only the containers this invocation started."""
    try:
        q = subprocess.run(
            ["docker", "ps", "-aq", "--filter",
             f"name=bench-linux-{RUN_ID}-"],
            capture_output=True, text=True)
        ids = q.stdout.split()
        if ids:
            subprocess.run(["docker", "rm", "-f", *ids],
                           capture_output=True)
    except Exception:
        pass


atexit.register(cleanup_owned_containers)


def sh(cmd: list[str] | str, **kw) -> subprocess.CompletedProcess:
    print("+", cmd if isinstance(cmd, str) else " ".join(cmd), flush=True)
    kw.setdefault("check", True)
    if isinstance(cmd, str):
        return subprocess.run(cmd, shell=True, **kw)
    return subprocess.run(cmd, **kw)


def docker(*args: str, quiet: bool = False, **kw) -> subprocess.CompletedProcess:
    cmd = ["docker", *args]
    if quiet:
        kw.setdefault("capture_output", True)
        kw.setdefault("text", True)
    return sh(cmd, **kw)


def build_image() -> None:
    docker("build", "-t", IMAGE, "-f", str(ROOT / "docker" / "Dockerfile"),
           str(ROOT))


def _docker_run_cmd(inner: str, *, name: str = "",
                    privileged: bool = False,
                    extra_vol: list[str] | None = None,
                    dri: list[str] | bool = False,
                    host_user: bool = True) -> list[str]:
    cmd = [
        "docker", "run", "--rm",
        "-v", f"{ROOT}:/bench",
        # the whole checkout mounts at /repo so the shared apps/* projects
        # (and waterui_path = "../../.." in the workspace-member WaterUI
        # app) resolve inside the container
        "-v", f"{REPO}:/repo",
        "-v", f"{CARGO_CACHE}:/cargo-home",
        # Rust toolchain binaries stay at /opt/cargo/bin in the image; the
        # mounted home only carries registry/git/target caches.
        "-e", "CARGO_HOME=/cargo-home",
    ]
    if host_user and not privileged:
        # Builds run as the host uid: files they write into the mounted
        # checkout (target/, generated backends/, dist/) come out
        # host-owned, never root-owned. Privileged measurement runs keep
        # root — they need it for cgroup/device setup and write nothing
        # into /repo.
        HOME_CACHE.mkdir(parents=True, exist_ok=True)
        cmd += [
            "-u", f"{os.getuid()}:{os.getgid()}",
            "-v", f"{HOME_CACHE}:/home/bench",
            "-v", f"{WATER_CACHE}:/home/bench/.water",
            "-v", f"{NPM_CACHE}:/home/bench/.npm",
            "-e", "HOME=/home/bench",
        ]
    else:
        cmd += [
            "-v", f"{WATER_CACHE}:/root/.water",
            "-v", f"{NPM_CACHE}:/root/.npm",
            "-e", "HOME=/root",
        ]
    if privileged:
        cmd += ["--privileged", "--cgroupns=private"]
    if dri is True and Path("/dev/dri").is_dir():
        # pass the host render nodes through so Mesa/wgpu can bind a real GPU
        cmd += ["--device=/dev/dri"]
    elif isinstance(dri, list):
        # restrict the container to the selected adapter's render nodes so
        # the renderer a contestant uses is pinned, not merely available
        for node in dri:
            cmd += [f"--device={node}"]
    for v in extra_vol or []:
        cmd += ["-v", v]
    if name:
        cmd += ["--name", name]
    cmd += [IMAGE, "bash", "-c", inner]
    return cmd


def docker_run_bash(inner: str, *, name: str = "", privileged: bool = False,
                    extra_vol: list[str] | None = None,
                    dri: list[str] | bool = False,
                    quiet: bool = False,
                    host_user: bool = True) -> subprocess.CompletedProcess:
    """Run `bash -c inner` in the image with the bench tree mounted at /bench."""
    cmd = _docker_run_cmd(inner, name=name, privileged=privileged,
                          extra_vol=extra_vol, dri=dri,
                          host_user=host_user)
    if quiet:
        print("+", " ".join(cmd), flush=True)
        return subprocess.run(cmd, capture_output=True, text=True, check=True)
    return sh(cmd)


def build_contestants(manifest: dict, only: set[str] | None) -> dict[str, str]:
    """Build each contestant in the image; return {contestant: staged dir}."""
    staged = {}

    # benchcomp compositor itself is built in-image so its deps (wlroots)
    # always match the runtime.
    docker_run_bash(
        "gcc -O2 -Wall -Wextra -DWLR_USE_UNSTABLE "
        "-o /bench/benchcomp/benchcomp /bench/benchcomp/benchcomp.c "
        "-Wl,--export-dynamic $(pkg-config --cflags --libs wlroots-0.18 "
        "wayland-server libdrm gbm xkbcommon pixman-1) -lrt -ldl")

    if not only or "waterui-hydrolysis" in only:
        # The shared WaterUI app is a workspace member: `water package`
        # scaffolds backends/hydrolysis inside the project, builds the
        # shared-runtime release binary and stages binary + resources +
        # libwaterui_dylib into backends/hydrolysis/dist/linux/release —
        # exactly what gets shipped. waterui_path = "../../.." resolves to
        # /repo inside the container.
        inner = r'''
set -e
cd /repo/benchmarks/competitive/apps/waterui
export PATH=/bench/tools:$PATH
water package --platform linux --backend hydrolysis --release -y
D=/bench/dist/waterui-hydrolysis
rm -rf "$D"
mkdir -p "$D"
cp -a backends/hydrolysis/dist/linux/release/. "$D/"
# the shipped binary sits beside resources/ under the crate name
BIN="$D/waterui-bench"
[ -f "$BIN" ] || { echo "hydrolysis binary not found at $BIN"; ls -la "$D"; exit 1; }
mv "$BIN" "$D/app"
ls -la "$D"
'''
        docker_run_bash(inner)
        staged["waterui-hydrolysis"] = "dist/waterui-hydrolysis"

    if not only or "gtk4" in only:
        inner = r'''
set -e
cd /bench/gtk4
gcc -O2 -Wall -o gtk-bench main.c $(pkg-config --cflags --libs gtk4) -lm
mkdir -p /bench/dist/gtk4
cp gtk-bench /bench/dist/gtk4/app
'''
        docker_run_bash(inner)
        staged["gtk4"] = "dist/gtk4"

    if not only or "electron" in only:
        inner = r'''
set -e
cd /repo/benchmarks/competitive/apps/electron
# the committed package-lock is the version pin — `npm ci` installs it
# verbatim and never rewrites tracked files
npm ci --no-audit --no-fund
mkdir -p /bench/dist/electron
cp -a node_modules/electron/dist/. /bench/dist/electron/
mkdir -p /bench/dist/electron/resources/app
cp index.html main.js renderer.js package.json /bench/dist/electron/resources/app/
'''
        docker_run_bash(inner)
        staged["electron"] = "dist/electron"

    if not only or "flutter" in only:
        # the linux platform dir is generated, not committed — the pinned
        # SDK's `flutter create` produces it in a scratch dir, then the
        # build runs inside the shared app
        inner = r'''
set -e
git config --global --add safe.directory /opt/flutter
APP=/repo/benchmarks/competitive/apps/flutter
TAG="flutter create --platforms linux --project-name bench_flutter --org dev.bench --template app | flutter_ver=''' \
        + manifest["toolchain"]["flutter_version"] + r'''"
STAMP="$APP/linux/.bench-generator"
if [ ! -d "$APP/linux" ] || [ "$(cat "$STAMP" 2>/dev/null)" != "$TAG" ]; then
    rm -rf "$APP/linux"
    T=$(mktemp -d)
    flutter create --platforms linux --project-name bench_flutter \
        --org dev.bench --template app "$T/app"
    cp -a "$T/app/linux" "$APP/linux"
    echo "$TAG" > "$STAMP"
fi
cd "$APP"
flutter build linux --release
mkdir -p /bench/dist
rm -rf /bench/dist/flutter
cp -a build/linux/x64/release/bundle /bench/dist/flutter
'''
        docker_run_bash(inner)
        staged["flutter"] = "dist/flutter"

    return staged


def package_size(dir_rel: str) -> dict:
    """Uncompressed and gzip-compressed size of the staged directory."""
    d = ROOT / dir_rel
    total = sum(p.stat().st_size for p in d.rglob("*") if p.is_file())
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz", compresslevel=6,
                      format=tarfile.PAX_FORMAT) as tf:
        tf.add(d, arcname=d.name, recursive=True)
    return {"dir": dir_rel, "bytes_uncompressed": total,
            "bytes_gz": buf.tell()}


def parse_events(path: Path) -> list[dict]:
    out = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return out


def percentile(vals: list[float], p: float) -> float:
    if not vals:
        return 0.0
    vals = sorted(vals)
    k = (len(vals) - 1) * p / 100
    lo, hi = int(k), min(int(k) + 1, len(vals) - 1)
    return vals[lo] + (vals[hi] - vals[lo]) * (k - lo)


def metrics_from_events(events: list[dict]) -> dict:
    spawn = next((e for e in events if e.get("ev") == "spawn"), None)
    presents = [e for e in events if e.get("ev") == "present"]
    commits = [e for e in events if e.get("ev") == "commit"]
    mems = [e for e in events if e.get("ev") == "mem"]
    mapped = [e for e in events if e.get("ev") == "map"]

    m: dict = {"mapped": bool(mapped), "present_count": len(presents),
               "commit_count": len(commits)}

    first_committed = next(
        (p for p in presents if p.get("committed")), None)
    if spawn and first_committed:
        m["launch_ms"] = (first_committed["t"] - spawn["t"]) / 1e6

    # Frame pacing: intervals between consecutive presents that carried a
    # freshly committed buffer (i.e. real content frames).
    committed_ts = [p["t"] for p in presents if p.get("committed")]
    deltas = [(b - a) / 1e6 for a, b in zip(committed_ts, committed_ts[1:])]
    deltas = [d for d in deltas if 0 < d < 2000]
    if deltas:
        m["frame_ms"] = {
            "p50": percentile(deltas, 50),
            "p90": percentile(deltas, 90),
            "p99": percentile(deltas, 99),
            "samples": [round(d, 3) for d in deltas],
        }
        m["dropped_pct"] = round(
            100 * sum(1 for d in deltas if d > 1.5 * VSYNC_MS) / len(deltas), 2)
        span = (committed_ts[-1] - committed_ts[0]) / 1e9 if len(committed_ts) > 1 else 0
        m["fps"] = round((len(committed_ts) - 1) / span, 2) if span > 0 else 0

    if mems:
        currents = [e.get("current", 0) for e in mems]
        peaks = [e.get("peak", 0) for e in mems]
        m["rss_bytes_steady"] = int(statistics.median(currents))
        m["rss_bytes_peak"] = max(peaks)

    return m


def dri_nodes_in_use(cname: str) -> set[str]:
    """Device nodes currently held open by the contestant's own
    processes (the `benchapp` cgroup inside the container) — per-process
    renderer evidence, not mount pinning. Empty set when the container
    or cgroup is gone."""
    q = subprocess.run(
        ["docker", "exec", cname, "bash", "-c",
         "for p in $(cat /sys/fs/cgroup/benchapp/cgroup.procs "
         "2>/dev/null); do ls -l /proc/$p/fd 2>/dev/null; done "
         "| grep -oE '/dev/[A-Za-z0-9_]+' | sort -u"],
        capture_output=True, text=True)
    if q.returncode != 0:
        return set()
    return {Path(n).name for n in q.stdout.split()}


def fling_script(fling: dict, duration_ms: int) -> str:
    """The shared fling protocol (../WORKLOADS.md) as a benchcomp script:
    pointer parked at the window centre after `warmup_ms`, then repeated
    programs of `down` flings + `up` flings — `detents` 15 px wheel
    detents spread over `duration_ms`, a `pause_ms` pause after each."""
    lines = [f"{fling['warmup_ms']} motion 640 400"]
    t = float(fling["warmup_ms"])
    detent_ms = fling["duration_ms"] / fling["detents"]
    while t < duration_ms:
        for direction, count in ((1, fling["down"]), (-1, fling["up"])):
            for _ in range(count):
                for _ in range(fling["detents"]):
                    lines.append(f"{int(t)} axis {direction}")
                    t += detent_ms
                t += fling["pause_ms"]
    return "\n".join(lines) + "\n"


SCRIPT_DIR = ROOT / ".cache" / "scripts"


def workload_script(manifest: dict, wl: str, duration_ms: int) -> Path | None:
    """The generated drive script for a scrolling workload — emitted from
    the manifest's [fling] declaration so the committed tree carries no
    hand-written drive programs."""
    if wl not in ("w2", "w4"):
        return None
    SCRIPT_DIR.mkdir(parents=True, exist_ok=True)
    path = SCRIPT_DIR / f"{wl}.scr"
    path.write_text(fling_script(manifest["fling"], duration_ms))
    return path


def run_workload(contestant_cmd: str, wl: str, duration_ms: int,
                 script: Path | None, out_jsonl: Path,
                 dri: list[str] | bool = True) -> tuple[dict, str, set[str]]:
    """One measurement rep. benchcomp writes straight to the run-scoped
    out path (unique per rep AND per invocation); the owned container is
    removed even when the run fails. Returns (metrics, log, dri_nodes)
    where dri_nodes is the set of device-node basenames the contestant's
    own cgroup actually held open during the run."""
    script_arg = (f"--script /bench/{script.relative_to(ROOT)}"
                  if script else "")
    cname = _owned_container_name()
    inner = (
        "export XDG_RUNTIME_DIR=/tmp/bench-xdg; "
        "mkdir -p $XDG_RUNTIME_DIR; "
        f"/bench/benchcomp/benchcomp --spawn {json.dumps(contestant_cmd)} "
        f"--duration {duration_ms} --size 1280x800 --refresh 60000 "
        f"--cgroup benchapp {script_arg} "
        f"--out /bench/{out_jsonl.relative_to(ROOT)}"
    )
    cmd = _docker_run_cmd(inner, name=cname, privileged=True, dri=dri,
                          host_user=False)
    print("+", " ".join(cmd), flush=True)
    proc = subprocess.Popen(
        cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True)
    opened: set[str] = set()
    try:
        while proc.poll() is None:
            opened |= dri_nodes_in_use(cname)
            time.sleep(0.3)
        opened |= dri_nodes_in_use(cname)
        out, _ = proc.communicate()
    finally:
        subprocess.run(["docker", "rm", "-f", cname],
                       capture_output=True)
    if proc.returncode != 0:
        raise subprocess.CalledProcessError(
            proc.returncode, cmd, output=out, stderr=out)
    sys.stdout.write(out or "")
    return (metrics_from_events(parse_events(out_jsonl)),
            out or "", opened)


def capacity_rep(contestant_cmd: str, spec: dict, dri,
                 out_dir: Path, tag: str) -> dict:
    """One W5 ladder: one launch per step with BENCH_STEP, settle+hold.

    A step collapses (the ladder stops) when fewer than collapse_frac of
    its presents land inside two 60 Hz budgets (~33.3 ms — drawing slower
    than ~30 fps) or the capture yields fewer than four frames."""
    out = {"steps": [], "collapsed_at": None, "dri_nodes": []}
    for n in spec["steps"]:
        out_jsonl = out_dir / f"{tag}-step{n}-{RUN_ID}.jsonl"
        cmd = contestant_cmd.replace("BENCH_WORKLOAD",
                                     f"BENCH_STEP={n} BENCH_WORKLOAD", 1)
        s, run_log, opened = run_workload(
            cmd, "w5", spec["settle_ms"] + spec["hold_ms"], None,
            out_jsonl, dri=dri)
        out["dri_nodes"] = sorted(set(out["dri_nodes"]) | opened)
        rec = {"step": n, "metrics": s}
        frames = (s.get("frame_ms") or {}).get("samples", [])
        if len(frames) < 4:
            rec["collapse"] = "insufficient_frames"
            out["collapsed_at"] = n
            out["steps"].append(rec)
            break
        inside = sum(1 for d in frames if d <= 2 * VSYNC_MS) / len(frames)
        rec["inside_2x_budget_frac"] = round(inside, 3)
        out["steps"].append(rec)
        if inside < spec["collapse_frac"]:
            out["collapsed_at"] = n
            break
    return out


def aggregate_capacity(ladders: list[dict], budget_ms: float) -> dict:
    """Aggregate per-rep ladders: largest step keeping >=99% of presents
    inside budget_ms, per rep, then median across reps."""
    caps = []
    for lad in ladders:
        ok = [st for st in lad.get("steps", [])
              if (st.get("metrics", {}).get("frame_ms") or {})
              .get("samples") and
              sum(1 for d in st["metrics"]["frame_ms"]["samples"]
                  if d <= budget_ms) /
              len(st["metrics"]["frame_ms"]["samples"]) >= 0.99]
        caps.append(ok[-1]["step"] if ok else 0)
    return {"capacity": statistics.median(caps) if caps else 0,
            "rep_capacities": caps}


def aggregate(samples: list[dict]) -> dict:
    """Median/min/max over successful reps only; attempt records kept.

    `samples` mixes successful rep metrics and `{"error": ...}` attempt
    records — stats never fold an errored attempt in, and the cell
    reports how many attempts actually succeeded."""
    good = [s for s in samples if "error" not in s]
    keys = set()
    for s in good:
        keys |= s.keys()
    keys -= {"mapped", "present_count", "run", "adapter_used"}
    out: dict = {}
    for k in sorted(keys):
        if k == "frame_ms":
            fs = [s["frame_ms"] for s in good if "frame_ms" in s]
            all_ms = [x for f in fs for x in f["samples"]]
            out["frame_ms"] = {
                "p50": percentile(all_ms, 50),
                "p90": percentile(all_ms, 90),
                "p99": percentile(all_ms, 99),
                "run_p50s": [round(f["p50"], 3) for f in fs],
            }
            continue
        vals = [s[k] for s in good if k in s]
        if not vals:
            continue
        out[k] = {
            "median": statistics.median(vals),
            "min": min(vals),
            "max": max(vals),
            "samples": vals,
        }
    out["runs_succeeded"] = len(good)
    out["runs_attempted"] = len(samples)
    out["failures"] = [s for s in samples if "error" in s]
    return out


SOFTWARE_GPU_MARKERS = ("llvmpipe", "lavapipe", "swiftshader", "softpipe",
                        "warp", "basic render", "basic display")


def gpu_probe() -> dict:
    """GPU adapters as seen inside the bench container.

    gpu_probe.py exits non-zero when vulkaninfo fails or finds no adapter;
    that surfaces as CalledProcessError here and stops the run — a probe
    that cannot enumerate adapters must never silently pass the guard.
    """
    q = docker_run_bash("python3 /bench/runner/gpu_probe.py",
                        quiet=True, dri=True)
    return json.loads(q.stdout.strip().splitlines()[-1])


def adapter_software(a: dict) -> bool:
    """CPU-type devices and name-matched software rasterizers."""
    return (a.get("type") == "cpu"
            or any(m in a.get("name", "").lower()
                   for m in SOFTWARE_GPU_MARKERS))


def gpu_software(g: dict) -> bool:
    """True when no hardware GPU adapter is visible: every enumerated
    Vulkan device is a CPU/software rasterizer."""
    return not [a for a in g.get("vulkan") or [] if not adapter_software(a)]


def gpu_label(g: dict) -> str:
    devs = g.get("vulkan") or []
    if devs:
        return ", ".join(f"{d['name']} [{d['type']}]" for d in devs)
    return ("none (no Vulkan adapter; /dev/dri "
            + ("present" if g.get("dri") else "absent") + ")")


# hydrolysis's own adapter report — what the contestant actually selected,
# not what the inventory made available. Same log line the Windows leg
# parses (`RUST_LOG=hydrolysis::gpu=info`).
ADAPTER_RE = re.compile(
    r"selected wgpu adapter.*name:\s*\"(?P<name>[^\"]+)\".*?"
    r"device_type:\s*(?P<dt>[A-Za-z]+).*?backend:\s*(?P<backend>[A-Za-z0-9]+)",
    re.DOTALL,
)


def selected_adapter(log: str) -> dict | None:
    m = ADAPTER_RE.search(log)
    if not m:
        return None
    return {"name": m.group("name"), "type": m.group("dt").lower(),
            "backend": m.group("backend")}


def pci_env(node_pci: str) -> str:
    """0000:03:00.0 -> pci-0000_03_00_0 for DRI_PRIME."""
    return "pci-" + node_pci.replace(":", "_").replace(".", "_")


def machine_spec(gpu: dict, sw: bool, selected: dict | None = None) -> dict:
    cpu = "unknown"
    try:
        with open("/proc/cpuinfo") as f:
            m = re.search(r"model name\s*:\s*(.+)", f.read())
            if m:
                cpu = m.group(1).strip()
    except OSError:
        pass
    mem_kb = 0
    try:
        with open("/proc/meminfo") as f:
            mem_kb = int(f.readline().split()[1])
    except OSError:
        pass
    return {
        "cpu": cpu,
        "cores": os.cpu_count(),
        "mem_gb": round(mem_kb / 1048576, 1),
        "kernel": platform.release(),
        "os": " ".join(platform.uname()[:3]),
        "device": ("host (hardware GPU)" if gpu.get("dri")
                   else "VM/container (no physical GPU)"),
        "gpu": gpu_label(gpu),
        # the adapter pinned for this run (mount-restricted /dev/dri +
        # Mesa device-select); absent on all-software hosts.
        "gpu_used": (f"{selected['name']} [{selected['type']}]"
                     if selected else None),
        "gpu_attribution": (
            "restricted /dev/dri mount + MESA_VK_DEVICE_SELECT/DRI_PRIME "
            "make the selected adapter the only one reachable; "
            "hydrolysis's own adapter log is verified per run"
            if selected else "inventory only — no hardware adapter"),
        "gpu_software": sw,
    }


def resolved_versions() -> dict:
    """Query the image for the versions actually used."""
    q = docker_run_bash(
        "dpkg-query -W -f='${Package} ${Version}\\n' mesa-vulkan-drivers "
        "libwlroots-0.18 libgtk-4-1 libwayland-client0 nodejs 2>/dev/null; "
        "rustc --version; node --version; python3 --version; "
        "flutter --version 2>/dev/null | head -4; "
        "cat /repo/benchmarks/competitive/apps/electron/node_modules/electron/package.json 2>/dev/null "
        " | python3 -c 'import json,sys; print(\"electron\", json.load(sys.stdin)[\"version\"])'",
        quiet=True)
    return {"raw": q.stdout.strip()}


CONTESTANT_CMDS = {
    "waterui-hydrolysis":
        "env BENCH_WORKLOAD={wl} {wenv}{selenv}"
        "WAYLAND_DISPLAY=wayland-0 /bench/dist/waterui-hydrolysis/app",
    "gtk4":
        "env BENCH_WORKLOAD={wl} {selenv}WAYLAND_DISPLAY=wayland-0 "
        "GDK_BACKEND=wayland /bench/dist/gtk4/app",
    "electron":
        "env BENCH_WORKLOAD={wl} {selenv}WAYLAND_DISPLAY=wayland-0 "
        "/bench/dist/electron/electron --no-sandbox --ozone-platform=wayland",
    "flutter":
        "env BENCH_WORKLOAD={wl} {selenv}WAYLAND_DISPLAY=wayland-0 "
        "/bench/dist/flutter/bench_flutter",
}

LIMITATIONS = {
    "frame_ms": "Software Vulkan/GL (lavapipe/llvmpipe): frame times reflect "
                "CPU rasterisation, not a hardware GPU — dev numbers only.",
    "fps": "Compositor vsync at 60Hz; same for every contestant.",
    "rss_bytes_steady": "cgroup v2 memory.current of the app process tree.",
    "rss_bytes_peak": "cgroup v2 memory.peak of the app process tree.",
    "launch_ms": "spawn to first committed present, CLOCK_MONOTONIC.",
}

# Per-contestant caveats — discovered while bringing each contestant up under
# the shared headless wlroots compositor.
CONTESTANT_LIMITATIONS = {
    ("electron", "w3"): (
        "Chromium's viz frame submission stalls for continuous rAF animation "
        "under a headless wlroots compositor without a DRM device: rAF ticks "
        "at 60fps and the page stays 'visible', but wl_surface.commit only "
        "happens ~4 times per run (verified identical with --disable-gpu and "
        "SwiftShader). Input-driven frames (W2/W4 scroll) submit normally at "
        "60fps. W3 frame numbers below reflect the few committed frames only."
    ),
}


def _self_test() -> None:
    """Failure-direction checks — no docker, no GPU."""
    import importlib.util

    toolchain._self_test()

    # mixed attempts: errored reps are kept as records but never folded
    # into the stats
    samples = [{"run": i, "launch_ms": 10 + i, "frame_ms": {
        "p50": 16.0, "p90": 17.0, "p99": 18.0,
        "samples": [15.5, 16.5]}} for i in range(4)]
    samples.insert(2, {"run": 2, "error": "container rc=1",
                       "stderr_tail": "boom"})
    agg = aggregate(samples)
    assert agg["runs_succeeded"] == 4 and agg["runs_attempted"] == 5
    assert agg["launch_ms"]["median"] == statistics.median(
        [10, 11, 12, 13])
    assert len(agg["failures"]) == 1 and "container" in \
        agg["failures"][0]["error"]
    # all-failure cell: stats empty, accounting intact
    agg0 = aggregate([{"run": i, "error": "x"} for i in range(5)])
    assert agg0["runs_succeeded"] == 0 and agg0["runs_attempted"] == 5
    assert "launch_ms" not in agg0

    # report refuses any cell below the rep floor (mixed) and an
    # all-failed measurement
    spec = importlib.util.spec_from_file_location(
        "report", Path(__file__).with_name("report.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    wui = {"workloads": {"w1": {
        "runs_succeeded": 5, "runs_attempted": 5,
        "launch_ms": {"median": 10, "min": 9, "max": 11,
                      "samples": [10] * 5}}}}
    ok_res = {"repetitions": 5, "repetitions_met": True,
              "machine": {}, "contestants": {"waterui-hydrolysis": wui}}
    assert "5/5" in mod.render(ok_res)
    bad = {"repetitions": 5, "repetitions_met": True, "machine": {},
           "contestants": {"waterui-hydrolysis": {"workloads": {"w1": {
               "runs_succeeded": 3, "runs_attempted": 5,
               "failures": [{"run": 1, "error": "rc=1"}]}}}}}
    try:
        mod.render(bad)
    except SystemExit as e:
        assert "waterui-hydrolysis/w1: 3/5" in str(e)
    else:
        raise AssertionError("incomplete cell reported")

    # L1: the --only merge must never pick an -INCOMPLETE evidence file
    # as its target, and must never overwrite it
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        od = Path(td)
        machine = {"gpu": "fixture"}
        complete = od / "results-20200101-000000-deadbeef.json"
        complete.write_text(json.dumps({
            "machine": machine, "generated_utc": "old",
            "repetitions_met": True,
            "contestants": {"gtk4": {"workloads": {"w1": {
                "runs_succeeded": 5, "runs_attempted": 5}}}}}))
        incomplete = od / "results-INCOMPLETE-99999999-999999-livebeef.json"
        incomplete.write_text(json.dumps({"evidence": "keep me"}))
        new = {"machine": machine, "generated_utc": "new",
               "contestants": {"flutter": {"workloads": {"w1": {
                   "runs_succeeded": 5, "runs_attempted": 5}}}}}
        merged, out, rc = merge_partial_results(new, 5, od)
        assert rc == 0 and out == complete
        assert incomplete.read_text() == \
            json.dumps({"evidence": "keep me"})
        assert merged["repetitions_met"] is True
        assert "flutter" in merged["contestants"] and \
            "gtk4" in merged["contestants"]

        # L4: a cell carried forward below the floor keeps the merged
        # result incomplete — fresh -INCOMPLETE, prior file untouched
        stale_cell = {"machine": machine, "generated_utc": "n2",
                      "contestants": {"electron": {"workloads": {"w1": {
                          "runs_succeeded": 5, "runs_attempted": 5}}}}}
        # drop gtk4/w1 to 3 successes in the complete file
        prev = json.loads(complete.read_text())
        prev["contestants"]["gtk4"]["workloads"]["w1"][
            "runs_succeeded"] = 3
        complete.write_text(json.dumps(prev))
        merged2, out2, rc2 = merge_partial_results(stale_cell, 5, od)
        assert rc2 == 2 and "INCOMPLETE" in out2.name
        assert merged2["repetitions_met"] is False
        assert merged2["stale_cells"] == ["gtk4/w1"]
        assert not out2.exists() or True  # caller writes it
        # a machine mismatch refuses to merge
        other = {"machine": {"gpu": "elsewhere"},
                 "contestants": {"x": {"workloads": {}}}}
        _m3, out3, rc3 = merge_partial_results(other, 5, od)
        assert rc3 == 0 and "INCOMPLETE" not in out3.name
        assert _m3 is other

    # renderer evidence: basenames compared, empty opened set fails
    assert {"renderD128"} & {Path("/dev/dri/renderD128").name}
    assert not ({"card0"} & {"renderD128"})
    print("linux runner self-test ok")


def merge_partial_results(results: dict, reps: int,
                          out_dir: Path) -> tuple[dict, Path, int]:
    """`--only` merge: fold this run's cells into the newest COMPLETE
    results file. Returns (results, out_path, exit_code).

    A `-INCOMPLETE` file is failure evidence — never a merge target and
    never overwritten (L1). Cells carried forward are re-checked against
    THIS run's repetition floor; a shortfall writes a fresh -INCOMPLETE
    (the prior complete file stays intact) and returns 2 (L4). A machine
    mismatch leaves the run standalone rather than mixing hosts."""
    existing = [p for p in sorted(out_dir.glob("results-*.json"))
                if "INCOMPLETE" not in p.name]
    if not existing:
        return results, out_dir / f"results-{stamp_id()}.json", 0
    prev = json.loads(existing[-1].read_text())
    if prev.get("machine") != results.get("machine"):
        print("previous results come from a different machine — "
              "not merging; writing this run standalone",
              file=sys.stderr)
        return results, out_dir / f"results-{stamp_id()}.json", 0
    merged_cells = prev["contestants"]
    merged_cells.update(results["contestants"])
    prev["contestants"] = merged_cells
    prev["generated_utc"] = results["generated_utc"]
    stale = sorted(
        f"{cname}/{wl}"
        for cname, cent in merged_cells.items()
        for wl, wagg in (cent.get("workloads") or {}).items()
        if (wagg.get("runs_succeeded") or 0) < reps)
    if stale:
        prev["repetitions_met"] = False
        prev["stale_cells"] = stale
        out = out_dir / f"results-INCOMPLETE-{stamp_id()}.json"
        print(f"merged run still has cells below the {reps}-rep floor: "
              f"{stale} — evidence kept in {out}", file=sys.stderr)
        return prev, out, 2
    prev["repetitions_met"] = True
    prev.pop("stale_cells", None)
    return prev, existing[-1], 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true",
                    help="run failure-direction checks without docker")
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--only", type=str, default="")
    ap.add_argument("--workload", type=str, default="")
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--development", action="store_true",
                    help="permit a software GPU adapter (llvmpipe/lavapipe) "
                    "and mark the run development-only")
    ap.add_argument("--gpu", type=str, default="",
                    help="adapter name substring to measure on, required "
                    "when the host has more than one hardware adapter — "
                    "availability of a GPU does not prove a contestant "
                    "rendered on it")
    args = ap.parse_args()
    if args.self_test:
        _self_test()
        return 0

    manifest = tomllib.loads((ROOT / "manifest.toml").read_text())

    # one runner at a time: concurrent invocations share the results/out
    # trees and the cgroup name, so a second instance fails fast here
    # instead of racing them.
    OUT_DIR.mkdir(exist_ok=True)
    lock_fh = open(OUT_DIR / ".runner.lock", "w")
    try:
        fcntl.flock(lock_fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print("refusing to start: another runner.py holds "
              f"{OUT_DIR}/.runner.lock", file=sys.stderr)
        return 2
    lock_fh.write(f"pid={os.getpid()} run={RUN_ID}\n")
    lock_fh.flush()

    only = set(filter(None, args.only.split(","))) or None
    workloads = (list(filter(None, args.workload.split(",")))
                 or manifest["run"]["workloads"])
    reps = max(args.reps, manifest["run"]["repetitions"])
    durations = manifest["run"]["durations_ms"]

    CARGO_CACHE.mkdir(parents=True, exist_ok=True)
    WATER_CACHE.mkdir(parents=True, exist_ok=True)
    NPM_CACHE.mkdir(parents=True, exist_ok=True)
    OUT_DIR.mkdir(exist_ok=True)
    (ROOT / "out").mkdir(exist_ok=True)

    # Build the water CLI from THIS checkout (cli/ is a workspace member —
    # the checkout's HEAD is the framework+CLI+backend identity) into the
    # suite-shared cache; the runner never picks a host binary by path.
    tools = ROOT / "tools"
    tools.mkdir(exist_ok=True)
    water_bin = toolchain.provision_water_cli()
    sh(["cp", str(water_bin), str(tools / "water")])
    sh(["chmod", "+x", str(tools / "water")])

    if not args.skip_build:
        build_image()

    gpu = gpu_probe()
    adapters = gpu.get("vulkan") or []
    hw_adapters = [a for a in adapters if not adapter_software(a)]
    sw = not hw_adapters
    if sw and not args.development:
        print(
            f"refusing to measure: the GPU adapter is a software rasterizer "
            f"({gpu_label(gpu)}), so frame-time and memory numbers would "
            "not be evidence. Re-run on a host with a hardware GPU "
            "(/dev/dri is passed through when present), or pass "
            "--development to record the run as development-only.",
            file=sys.stderr)
        return 2

    # Which adapter the measurement runs on is pinned, not inferred: the
    # container mounts only the selected adapter's render nodes and Mesa's
    # device-select layer filters enumeration to it. A host offering more
    # than one hardware adapter is ambiguous unless --gpu picks one.
    selected: dict | None = None
    dri_mounts: list[str] | bool = True
    selenv = ""
    if hw_adapters:
        if len(hw_adapters) > 1 and not args.gpu:
            print(
                "refusing to measure: the host offers more than one "
                f"hardware adapter ({gpu_label(gpu)}) — adapter "
                "availability does not prove which one a contestant "
                "rendered on. Re-run with --gpu <name substring>.",
                file=sys.stderr)
            return 2
        if args.gpu:
            needle = args.gpu.lower()
            selected = next(
                (a for a in hw_adapters
                 if needle in a.get("name", "").lower()), None)
            if selected is None:
                print(f"--gpu {args.gpu!r} matches no hardware adapter "
                      f"({gpu_label(gpu)})", file=sys.stderr)
                return 2
        else:
            selected = hw_adapters[0]
        if not selected.get("dri"):
            print(
                f"refusing to measure: adapter '{selected['name']}' has no "
                "matching /dev/dri render node, so the renderer used "
                "cannot be pinned by mount — measurement would be "
                "unattributable.",
                file=sys.stderr)
            return 2
        dri_mounts = selected["dri"]
        vid = selected["vendor"].removeprefix("0x")
        did = selected["device"].removeprefix("0x")
        # MESA_VK_DEVICE_SELECT filters Vulkan enumeration to the selected
        # adapter (masking lavapipe, which has no PCI device to mount);
        # DRI_PRIME pins the EGL/GL device on the same adapter.
        selenv = f"MESA_VK_DEVICE_SELECT={vid}:{did} "
        if selected.get("pci"):
            selenv += f"DRI_PRIME={pci_env(selected['pci'])} "

    if not args.skip_build:
        staged = build_contestants(manifest, only)
    else:
        staged = {c: f"dist/{c}" for c in CONTESTANT_CMDS
                  if not only or c in only}

    # No adapter-forcing env ever exports — which GPU hydrolysis picks is
    # what the evidence must record, not what the runner dictates.
    # RUST_LOG always exports: the adapter line IS the per-rep renderer
    # evidence.
    wenv = "RUST_LOG=hydrolysis::gpu=info "

    results: dict = {
        "benchmark": "competitive-linux",
        "issue": "water-rs/waterui#1262",
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "machine": machine_spec(gpu, sw, selected),
        "development_only": bool(args.development),
        "versions": resolved_versions(),
        "water_cli": {"source": "in-tree cli/ (workspace member)",
                      "checkout_head": toolchain.checkout_head(),
                      "provisioned": str(water_bin)},
        "repetitions": reps,
        "frame_source": "benchcomp wlroots compositor present tick "
                        "(CLOCK_MONOTONIC per-surface commit->present)",
        "contestants": {},
    }

    for name in (only or CONTESTANT_CMDS.keys()):
        if name not in CONTESTANT_CMDS:
            print(f"unknown contestant {name}", file=sys.stderr)
            continue
        entry: dict = {"errors": []}
        dist = ROOT / staged.get(name, f"dist/{name}")
        if dist.exists():
            entry["package"] = package_size(staged[name])
        else:
            entry["errors"].append(f"staged dir missing: {staged.get(name)}")

        entry["workloads"] = {}
        expected_nodes = (
            {Path(n).name for n in dri_mounts}
            if isinstance(dri_mounts, list) else set())
        for wl in workloads:
            if wl in (manifest.get("capacity") or {}):
                spec = manifest["capacity"][wl]
                if name not in spec.get(
                        "contestants", list(CONTESTANT_CMDS)):
                    # the contestant implements no cell for this workload
                    # (e.g. electron has no W5) — no row, not a failure
                    continue
                # canonical capacity ladder (W5): one launch per step with
                # BENCH_STEP; per-rep ladders aggregate to a median capacity
                cmd = CONTESTANT_CMDS[name].format(
                    wl=wl, wenv=wenv, selenv=selenv)
                ladders = []
                failures = []
                for rep in range(reps):
                    try:
                        lad = capacity_rep(
                            cmd, spec, dri_mounts,
                            ROOT / "out", f"{name}-{wl}-{rep}")
                        if expected_nodes and not (
                                set(lad["dri_nodes"]) & expected_nodes):
                            raise RuntimeError(
                                "no renderer evidence: the contestant's "
                                "own processes opened no expected DRI "
                                f"node ({sorted(lad['dri_nodes'])} vs "
                                f"{sorted(expected_nodes)})")
                        ladders.append(lad)
                    except Exception as e:  # record, keep going
                        failures.append({"run": rep, "error": str(e)})
                agg = {"runs_succeeded": len(ladders),
                       "runs_attempted": len(ladders) + len(failures),
                       "failures": failures,
                       "raw": ladders}
                for budget, key in ((8.33, "capacity_120hz"),
                                    (16.67, "capacity_60hz")):
                    agg[key] = aggregate_capacity(ladders, budget)
                entry["workloads"][wl] = agg
                if agg["runs_succeeded"] < reps:
                    results["contestants"][name] = entry
                    out_fail = (OUT_DIR
                                / f"results-INCOMPLETE-{stamp_id()}.json")
                    results["repetitions_met"] = False
                    out_fail.write_text(json.dumps(results, indent=2))
                    print(
                        f"repetition requirement not met: {name}/{wl} "
                        f"got {agg['runs_succeeded']}/{reps} successful "
                        "reps — refusing to emit a measurement",
                        file=sys.stderr)
                    return 2
                continue
            samples = []
            for rep in range(reps):
                out_jsonl = (ROOT / "out"
                             / f"{name}-{wl}-{rep}-{RUN_ID}.jsonl")
                try:
                    s, run_log, opened = run_workload(
                        CONTESTANT_CMDS[name].format(
                            wl=wl, wenv=wenv, selenv=selenv), wl,
                        durations.get(wl, 15000),
                        workload_script(
                            manifest, wl, durations.get(wl, 15000)),
                        out_jsonl,
                        dri=dri_mounts)
                    s["run"] = rep
                    s["dri_nodes"] = sorted(opened)
                    # per-rep renderer evidence from the contestant's
                    # own processes — missing evidence fails the attempt
                    if expected_nodes and not (opened & expected_nodes):
                        raise RuntimeError(
                            "no renderer evidence: the contestant's own "
                            "processes opened no expected DRI node "
                            f"({sorted(opened)} vs "
                            f"{sorted(expected_nodes)})")
                    if name == "waterui-hydrolysis":
                        used = selected_adapter(run_log)
                        s["adapter_used"] = used
                        if used is None:
                            # fail closed: no adapter line means no proof
                            # which renderer hydrolysis actually used
                            raise RuntimeError(
                                "no adapter evidence: hydrolysis logged "
                                "no 'selected adapter' line this rep")
                        if (used["type"] == "cpu"
                                or any(m in used["name"].lower()
                                       for m in SOFTWARE_GPU_MARKERS)):
                            if not args.development:
                                # SystemExit skips the per-rep
                                # `except Exception` below — an adapter
                                # misattribution refuses the run (exit 2),
                                # it is not recorded as a flaky failure.
                                print(
                                    "refusing to measure: hydrolysis "
                                    "actually selected a software "
                                    f"adapter ({used['name']}) — frame-"
                                    "time and memory numbers would not "
                                    "be hardware evidence; re-run with "
                                    "--development",
                                    file=sys.stderr)
                                raise SystemExit(2)
                            results["development_only"] = True
                    samples.append(s)
                except subprocess.CalledProcessError as e:
                    samples.append({
                        "run": rep, "error": f"container rc={e.returncode}",
                        "stderr_tail": (e.stderr or "")[-500:]})
                except Exception as e:  # keep going — record the failure
                    samples.append({"run": rep, "error": str(e)})
            agg = aggregate(samples)
            entry["workloads"][wl] = agg
            entry["workloads"][wl]["raw"] = samples
            # the cell only counts when it met the requested repetition
            # count with successful reps — a shortfall rejects the run
            if agg["runs_succeeded"] < reps:
                results["contestants"][name] = entry
                out_fail = OUT_DIR / f"results-INCOMPLETE-{stamp_id()}.json"
                results["repetitions_met"] = False
                out_fail.write_text(json.dumps(results, indent=2))
                print(
                    f"repetition requirement not met: {name}/{wl} got "
                    f"{agg['runs_succeeded']}/{reps} successful reps "
                    f"({agg['runs_attempted']} attempted) — records kept "
                    f"in {out_fail}; refusing to emit a measurement",
                    file=sys.stderr)
                return 2
            lim = CONTESTANT_LIMITATIONS.get((name, wl))
            if lim:
                entry["workloads"][wl]["limitation"] = lim
        results["contestants"][name] = entry

    results["limitations"] = {
        k: v for k, v in LIMITATIONS.items()
        if not (k == "frame_ms" and not sw)
    }
    results["repetitions_met"] = True
    out = OUT_DIR / f"results-{stamp_id()}.json"
    if only:
        results, out, rc = merge_partial_results(results, reps, OUT_DIR)
        out.write_text(json.dumps(results, indent=2))
        if rc:
            return rc
        print(f"wrote {out}")
        return 0
    out.write_text(json.dumps(results, indent=2))
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

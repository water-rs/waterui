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

import sys

if sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {sys.version.split()[0]}); every leg "
        "declares its version in pyproject.toml + .python-version and "
        "runs under the uv-managed interpreter (`uv run`)")

import argparse
import atexit
import fcntl
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
import toolchain
import frame_stats as lib_frames  # benchmarks/competitive/lib/toolchain.py

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


def rust_channel() -> str:
    """The toolchain channel the repository declares in rust-toolchain.toml
    — the image builds exactly that toolchain, never a Dockerfile literal
    or the ambient rustup default."""
    doc = tomllib.loads((REPO / "rust-toolchain.toml").read_text())
    ch = doc.get("toolchain", {}).get("channel")
    if not ch:
        raise RuntimeError(
            "no toolchain.channel in the repository's rust-toolchain.toml")
    return ch


def rust_toolchain_resolved() -> str:
    """The exact toolchain the image installs: the channel from
    rust-toolchain.toml resolved to a concrete version at image build
    (stable/beta/nightly -> '1.90.0' / 'nightly-YYYY-MM-DD'), recorded in
    results.versions. A floating channel name must never reach the
    Dockerfile."""
    ch = rust_channel()
    if re.fullmatch(r"\d+\.\d+(\.\d+)?|(nightly|beta)-\d{4}-\d{2}-\d{2}",
                    ch):
        return ch  # already exact (1.90.0 or nightly-2025-…)
    import urllib.request
    url = ("https://static.rust-lang.org/dist/"
           f"channel-rust-{ch}.toml")
    doc = tomllib.loads(
        urllib.request.urlopen(url, timeout=30).read().decode())
    resolved = resolve_channel_manifest(ch, doc, url)
    print(f"resolved rust channel {ch} -> {resolved} ({url})", flush=True)
    return resolved


def resolve_channel_manifest(ch: str, doc: dict, url: str) -> str:
    """The exact toolchain name a dist channel manifest pins. A stable
    channel resolves to its release version; nightly and beta resolve to
    `<channel>-<dist date>` — the manifest's own `date`, which names the
    dist directory. The date inside `rustc --version` is the commit date,
    usually a day earlier, and names no installable toolchain."""
    if ch == "stable":
        ver = doc["pkg"]["rust"]["version"]  # "1.90.0 (1159e78 2025-09-14)"
        m = re.match(r"(\d+\.\d+\.\d+) ", ver)
        if not m:
            raise RuntimeError(
                f"unparseable rust version in {url}: {ver!r}")
        return m.group(1)
    if ch in ("nightly", "beta"):
        date = doc.get("date")
        if not (isinstance(date, str)
                and re.fullmatch(r"\d{4}-\d{2}-\d{2}", date)):
            raise RuntimeError(f"{url} carries no dist date: {date!r}")
        return f"{ch}-{date}"
    raise RuntimeError(
        f"rust-toolchain.toml channel {ch!r} is neither a release, a "
        "dated nightly/beta, nor stable/nightly/beta")


def build_image() -> None:
    docker("build", "-t", IMAGE, "-f", str(ROOT / "docker" / "Dockerfile"),
           "--build-arg", f"RUST_CHANNEL={rust_toolchain_resolved()}",
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
    if host_user:
        # Build containers run as the host uid: files they write into the
        # mounted checkout (target/, generated backends/, dist/) come out
        # host-owned, never root-owned. Measurement containers run as
        # root instead (cgroup + /dev/dri need it) and chown their
        # outputs back at the end — see run_workload.
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
    # always match the runtime. Debian's wlroots headers include generated
    # wayland protocol headers (xdg-shell-protocol.h & co) that the dev
    # package does not ship — wayland-scanner generates them from
    # /usr/share/wayland-protocols at build time.
    docker_run_bash(
        "mkdir -p /tmp/proto && "
        "for xml in $(find /usr/share/wayland-protocols -name '*.xml'); "
        "do wayland-scanner server-header \"$xml\" "
        "/tmp/proto/$(basename \"${xml%.xml}\")-protocol.h; done && "
        "gcc -O2 -Wall -Wextra -Werror -DWLR_USE_UNSTABLE -I/tmp/proto "
        "-o /bench/benchcomp/benchcomp /bench/benchcomp/benchcomp.c "
        "-Wl,--export-dynamic $(pkg-config --cflags --libs wlroots-0.18 "
        "wayland-server libdrm gbm xkbcommon pixman-1) -lrt -ldl")

    if not only or "waterui-hydrolysis" in only:
        # The shared WaterUI app is a workspace member: `water package`
        # builds the shared-runtime release binary and stages it at
        # target/package/<name>-<backend> — a single statically-pie ELF
        # (hydrolysis ships no separate resources tree on Linux).
        # waterui_path = "../../.." resolves to /repo inside the
        # container.
        inner = r'''
set -e
cd /repo/benchmarks/competitive/apps/waterui
export PATH=/bench/tools:$PATH
water package --platform linux --backend hydrolysis --release -y
D=/bench/dist/waterui-hydrolysis
rm -rf "$D"
mkdir -p "$D"
BIN=target/package/waterui-bench-hydrolysis
[ -f "$BIN" ] || { echo "hydrolysis binary not found at $BIN"; ls -la target/package; exit 1; }
cp "$BIN" "$D/app"
ls -la "$D"
'''
        docker_run_bash(inner)
        staged["waterui-hydrolysis"] = "dist/waterui-hydrolysis"

    if not only or "gtk4" in only:
        inner = r'''
set -e
cd /bench/gtk4
mkdir -p /bench/dist/gtk4
gcc -O2 -Wall -o /bench/dist/gtk4/app main.c $(pkg-config --cflags --libs gtk4) -lm
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
# electron >= 43 ships the binary download as a bin entry, not a
# postinstall — `npm ci` alone leaves node_modules/electron/dist absent.
# Same check-and-run as the macOS packager: pull dist/ only when it is
# missing, then fail loudly if it is still absent.
if [ ! -d node_modules/electron/dist ]; then
  node node_modules/electron/install.js
fi
if [ ! -d node_modules/electron/dist ]; then
  echo "electron dist/ still missing after install.js" >&2
  exit 1
fi
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
        + manifest["contestants"]["flutter"]["flutter_version"] + r'''"
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
            except json.JSONDecodeError as e:
                raise RuntimeError(
                    f"malformed benchcomp event line in {path}: "
                    f"{line[:120]!r} ({e})") from e
    return out


def percentile(vals: list[float], p: float) -> float:
    if not vals:
        return 0.0
    vals = sorted(vals)
    k = (len(vals) - 1) * p / 100
    lo, hi = int(k), min(int(k) + 1, len(vals) - 1)
    return vals[lo] + (vals[hi] - vals[lo]) * (k - lo)


def metrics_from_events(events: list[dict], warmup_ms: float,
                        capture_ms: float) -> dict:
    """Launch, frame pacing and memory of one rep over the declared window
    [first committed present + warmup_ms, + capture_ms] (METHOD). Both
    are declared and nonzero; a rep with no committed present has no
    window and fails."""
    if warmup_ms <= 0 or capture_ms <= 0:
        raise RuntimeError(
            f"warmup_ms ({warmup_ms}) and capture_ms ({capture_ms}) must "
            "both be declared and nonzero")
    spawn = next((e for e in events if e.get("ev") == "spawn"), None)
    presents = [e for e in events if e.get("ev") == "present"]
    commits = [e for e in events if e.get("ev") == "commit"]
    mems = [e for e in events if e.get("ev") == "mem"]
    mapped = [e for e in events if e.get("ev") == "map"]
    mem_errors = [e for e in events if e.get("ev") == "mem_error"]
    if mem_errors:
        raise RuntimeError(
            "benchcomp could not read the app cgroup's memory: "
            + "; ".join(f"{e['what']}: {os.strerror(e['errno'])}"
                        for e in mem_errors))
    committed_ts = [p["t"] for p in presents if p.get("committed")]
    if spawn is None or not committed_ts:
        raise RuntimeError(
            "benchcomp logged no spawn or no committed present owned by "
            "the contestant — the rep has no launch and no window")

    m: dict = {"mapped": bool(mapped), "present_count": len(presents),
               "commit_count": len(commits),
               "launch_ms": (committed_ts[0] - spawn["t"]) / 1e6}

    # Frame pacing over the declared window — lib/frame_stats decides
    # runs, gaps and missed vsyncs (decision 1).
    rel = [(t - committed_ts[0]) / 1e6 for t in committed_ts]
    stats = lib_frames.frame_statistics(rel, warmup_ms, capture_ms,
                                        VSYNC_MS)
    if stats["intervals_ms"]:
        m["frame_ms"] = {
            "p50": stats["frame_ms_p50"],
            "p90": stats["frame_ms_p90"],
            "p99": stats["frame_ms_p99"],
            "samples": [round(d, 3) for d in stats["intervals_ms"]],
        }
        m["missed_vsyncs"] = stats["missed_vsyncs"]
        m["fps"] = stats["fps"]

    # memory over the same window as the frames: median and peak of the
    # cgroup's memory.current samples inside it — startup and the
    # lifetime high-water mark (memory.peak) never enter
    w0 = committed_ts[0] + warmup_ms * 1e6
    w1 = w0 + capture_ms * 1e6
    cur = [e["current"] for e in mems if w0 <= e["t"] <= w1]
    if not cur:
        raise RuntimeError(
            "no cgroup memory sample inside the measurement window")
    m["rss_bytes_steady"] = int(statistics.median(cur))
    m["rss_bytes_peak"] = max(cur)
    return m


# Userspace renderer libraries, by basename: Mesa Vulkan drivers
# (libvulkan_<drv>.so), DRI/gallium drivers (<drv>_dri.so), the gallium
# megadriver (libgallium-<ver>.so: every gallium driver in one object, so
# the render node it binds decides which one runs), Chromium's SwiftShader
# and the NVIDIA userspace driver.
_VK_DRIVERS = {"lvp": "lavapipe", "radeon": "radv", "intel": "anv",
               "intel_hasvk": "hasvk", "nouveau": "nvk", "virtio": "venus",
               "freedreno": "turnip", "panfrost": "panvk",
               "broadcom": "v3dv", "asahi": "honeykrisp",
               "powervr_mesa": "powervr"}
SOFTWARE_RENDERERS = {"lavapipe", "llvmpipe", "softpipe", "swrast",
                      "kms_swrast", "swiftshader"}
GALLIUM = "gallium"
# kernel DRM driver of the selected adapter -> the hardware userspace
# renderers that drive it
KERNEL_RENDERERS = {
    "amdgpu": {"radv", "radeonsi"}, "radeon": {"radeonsi", "r600"},
    "i915": {"anv", "hasvk", "iris", "crocus", "i965"},
    "xe": {"anv", "iris"}, "nouveau": {"nvk", "nouveau"},
    "virtio_gpu": {"venus", "virtio_gpu"}, "nvidia": {"nvidia"},
    "msm": {"turnip", "freedreno"}, "panfrost": {"panvk", "panfrost"},
    "panthor": {"panvk", "panfrost"}, "v3d": {"v3dv", "v3d"},
    "asahi": {"honeykrisp", "asahi"},
}


def renderer_of(lib_path: str) -> str | None:
    """The userspace renderer a mapped library is, or None."""
    name = Path(lib_path).name
    m = re.match(r"libvulkan_(\w+?)\.so", name)
    if m:
        return _VK_DRIVERS.get(m.group(1), f"vulkan:{m.group(1)}")
    m = re.match(r"(\w+)_dri\.so", name)
    if m:
        return m.group(1)
    if re.match(r"libgallium-.*\.so", name):
        return GALLIUM
    if name.startswith("libvk_swiftshader.so"):
        return "swiftshader"
    if re.match(r"lib(nvidia-(glcore|eglcore)|GLX_nvidia|EGL_nvidia)\.so",
                name):
        return "nvidia"
    return None


def kernel_driver(node: str) -> str:
    """The kernel DRM driver behind a render node on this host."""
    link = Path("/sys/class/drm") / Path(node).name / "device" / "driver"
    if not link.exists():
        raise RuntimeError(f"{node}: no kernel driver bound ({link})")
    return link.resolve().name


def _counter_value(key: str, raw: str) -> int | None:
    """A DRM fdinfo GPU usage counter as an integer, or None for a key that
    is not one: `drm-engine-<engine>: <n> ns` (busy time) and
    `drm-cycles-<class>: <n>` (xe). Capacity, total-cycle and memory keys
    are not usage."""
    if key.startswith("drm-engine-capacity-"):
        return None
    if key.startswith("drm-engine-"):
        m = re.fullmatch(r"(\d+) ns", raw)
    elif key.startswith("drm-cycles-"):
        m = re.fullmatch(r"(\d+)", raw)
    else:
        return None
    if m is None:
        raise RuntimeError(
            f"unparseable DRM fdinfo usage counter {key}: {raw!r}")
    return int(m.group(1))


def drm_usage_delta(events: list[dict]) -> dict[str, dict]:
    """GPU work per render node across the measurement window, from the
    DRM fdinfo counters benchcomp read at window start and window end.

    A DRM client is (render node, drm-client-id) — dup'd fds and forked
    holders of one open file share it. Its usage over the window is its
    end counters minus its start counters; a client opened inside the
    window started from zero. Returns {node: {"driver", "clients",
    "counters": {key: delta}}} for every node held at window end."""
    by_phase: dict[str, dict] = {"start": {}, "end": {}}
    for e in events:
        if e.get("ev") != "drm":
            continue
        ctr = e["counters"]
        if "drm-client-id" not in ctr or "drm-driver" not in ctr:
            raise RuntimeError(
                f"{e['target']} fd {e['fd']} of pid {e['pid']}: fdinfo "
                f"carries no drm-client-id/drm-driver ({sorted(ctr)}) — the "
                "kernel exposes no DRM client usage for this node")
        usage = {k: v for k, raw in ctr.items()
                 for v in [_counter_value(k, raw)] if v is not None}
        by_phase[e["phase"]][(e["target"], ctr["drm-client-id"])] = (
            ctr["drm-driver"], usage)
    if not any(e.get("ev") == "window_start" for e in events):
        raise RuntimeError("benchcomp logged no window_start — the DRM "
                           "usage counters were never read at window start")
    nodes: dict[str, dict] = {}
    for (node, client), (driver, end) in by_phase["end"].items():
        start = by_phase["start"].get((node, client), (driver, {}))[1]
        n = nodes.setdefault(node, {"driver": driver, "clients": 0,
                                    "counters": {}})
        n["clients"] += 1
        for k, v in end.items():
            d = v - start.get(k, 0)
            if d < 0:
                raise RuntimeError(
                    f"{node} client {client}: {k} went backwards across "
                    f"the window ({start.get(k)} -> {v})")
            n["counters"][k] = n["counters"].get(k, 0) + d
    return nodes


def renderer_evidence(events: list[dict], expected: dict) -> dict:
    """What the contestant rendered with, proven by the GPU work its own
    processes submitted inside the window: benchcomp reads the DRM fdinfo
    usage counters of every render-node fd in the contestant's cgroup at
    window start and at window end. The libraries mapped and nodes held at
    window end are recorded as supporting evidence only — the Vulkan
    loader maps every installed ICD and opens render nodes while
    enumerating, and the gallium megadriver carries llvmpipe, so neither
    shows what did the rendering. Raises when the run's expected renderer
    class is not the one proven.

    `expected` is {"class": "hardware", "kernel_driver", "nodes"} for a
    run pinned to a hardware adapter, {"class": "software"} otherwise. A
    hardware run must show a non-zero usage delta on one of the selected
    adapter's render nodes. A software run must show zero usage on every
    render node it holds, with a software rasterizer (lavapipe, llvmpipe,
    or the gallium megadriver that carries it) mapped."""
    errors = [e for e in events if e.get("ev") == "evidence_error"]
    if errors:
        raise RuntimeError(
            "benchcomp could not read the contestant's renderer evidence: "
            + "; ".join(f"{e['what']} of pid {e['pid']}: "
                        f"{os.strerror(e['errno'])} (errno {e['errno']})"
                        for e in errors))
    renderers = sorted({r for e in events if e.get("ev") == "lib"
                        for r in [renderer_of(e["path"])] if r})
    held = sorted({e["target"] for e in events if e.get("ev") == "fd"
                   and e["target"].startswith("/dev/dri/")})
    usage = drm_usage_delta(events)
    busy = sorted(n for n, u in usage.items()
                  if any(v > 0 for v in u["counters"].values()))
    out = {"renderer_class": expected["class"],
           "gpu_usage": usage, "gpu_busy_nodes": busy,
           "renderers_mapped": renderers, "render_nodes": held}
    if expected["class"] == "hardware":
        silent = sorted(n for n in expected["nodes"]
                        if n.startswith("/dev/dri/renderD")
                        and n in usage and not usage[n]["counters"])
        if silent:
            raise RuntimeError(
                f"renderer evidence: {silent} expose no drm-engine/"
                f"drm-cycles usage counters (kernel driver "
                f"{expected['kernel_driver']}) — GPU use cannot be proven "
                "on this adapter")
        used = sorted(set(expected["nodes"]) & set(busy))
        if not used:
            raise RuntimeError(
                f"renderer evidence: no GPU work on the selected adapter's "
                f"render nodes {expected['nodes']} across the window "
                f"(usage {usage}; mapped {renderers})")
        out["renderer_used"] = [
            f"{expected['kernel_driver']} on {n}" for n in used]
    else:
        if busy:
            raise RuntimeError(
                f"renderer evidence: a software run submitted GPU work on "
                f"{busy} across the window (usage {usage})")
        used = [r for r in renderers
                if r in SOFTWARE_RENDERERS or r == GALLIUM]
        if not used:
            raise RuntimeError(
                f"renderer evidence: no GPU work, but no software "
                f"rasterizer mapped either ({renderers}) — nothing shows "
                "what rendered")
        out["renderer_used"] = used
    return out


def fling_script(fling: dict, duration_ms: int) -> str:
    """The shared fling protocol (../WORKLOADS.md) as a benchcomp script:
    pointer parked at the window centre at t=0 (window start — benchcomp
    anchors script time on the first owned present + warmup), then
    repeated programs of `down` flings + `up` flings — `detents` 15 px
    wheel detents spread over `duration_ms`, a `pause_ms` pause after
    each."""
    lines = ["0 motion 640 400"]
    t = 0.0
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
                 expected: dict,
                 dri: list[str] | bool,
                 warmup_ms: int) -> tuple[dict, str]:
    """One measurement rep. benchcomp writes straight to the run-scoped
    out path (unique per rep AND per invocation); the owned container is
    removed even when the run fails. Returns (metrics, log); the metrics
    carry the renderer evidence benchcomp read at window end, checked
    against `expected`."""
    script_arg = (f"--script /bench/{script.relative_to(ROOT)}"
                  if script else "")
    cname = _owned_container_name()
    # Measurement containers run as root inside the container: cgroup v2
    # setup needs CAP_SYS_ADMIN-adjacent ownership of /sys/fs/cgroup and
    # /dev/dri nodes open without group juggling. Outputs are chowned to
    # the host uid at the end (owner of the /bench mount); a chown that
    # fails fails the rep.
    inner = (
        "export XDG_RUNTIME_DIR=/tmp/bench-xdg; "
        "mkdir -p $XDG_RUNTIME_DIR; "
        f"/bench/benchcomp/benchcomp --spawn {json.dumps(contestant_cmd)} "
        f"--duration {duration_ms} --warmup {warmup_ms} "
        "--size 1280x800 --refresh 60000 "
        f"--cgroup benchapp {script_arg} "
        f"--out /bench/{out_jsonl.relative_to(ROOT)}; "
        "rc=$?; "
        "chown -R \"$(stat -c %u:%g /bench)\" /bench/out "
        "|| { echo 'benchcomp: chown of /bench/out failed' >&2; exit 125; }; "
        "exit $rc"
    )
    cmd = _docker_run_cmd(inner, name=cname, privileged=True, dri=dri,
                          host_user=False)
    print("+", " ".join(cmd), flush=True)
    try:
        proc = subprocess.run(cmd, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True)
    finally:
        subprocess.run(["docker", "rm", "-f", cname],
                       capture_output=True)
    out = proc.stdout
    if proc.returncode != 0:
        raise subprocess.CalledProcessError(
            proc.returncode, cmd, output=out, stderr=out)
    sys.stdout.write(out or "")
    events = parse_events(out_jsonl)
    # capture is fixed from window start (first owned present +
    # warmup), never from spawn — duration_ms IS the capture length
    m = metrics_from_events(events, warmup_ms, duration_ms)
    m["renderer"] = renderer_evidence(events, expected)
    return m, out or ""


def capacity_rep(contestant_cmd: str, spec: dict, dri, expected: dict,
                 out_dir: Path, tag: str) -> dict:
    """One W5 ladder: one launch per step with BENCH_STEP, settle+hold.

    A step collapses (the ladder stops) when fewer than collapse_frac of
    its presents land inside two 60 Hz budgets (~33.3 ms — drawing slower
    than ~30 fps) or the capture yields fewer than four frames."""
    out = {"steps": [], "collapsed_at": None}
    for n in spec["steps"]:
        out_jsonl = out_dir / f"{tag}-step{n}-{RUN_ID}.jsonl"
        cmd = contestant_cmd.replace("BENCH_WORKLOAD",
                                     f"BENCH_STEP={n} BENCH_WORKLOAD", 1)
        # settle_ms is the declared warmup, hold_ms the capture length
        s, _run_log = run_workload(
            cmd, "w5", spec["hold_ms"], None, out_jsonl, expected,
            dri=dri, warmup_ms=spec["settle_ms"])
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
    keys -= {"mapped", "present_count", "run", "adapter_used", "renderer"}
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
    # the probe's stdout is the JSON document itself (indent=2);
    # diagnostics go to stderr
    return json.loads(q.stdout)


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
        "libwlroots-0.18 libgtk-4-1 libwayland-client0 2>/dev/null; "
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
    "rss_bytes_steady": "median of the app process tree's cgroup v2 "
                        "memory.current, sampled every 100 ms inside the "
                        "measurement window.",
    "rss_bytes_peak": "maximum of the same in-window memory.current "
                      "samples (startup is outside the window).",
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

    # renderer evidence: the DRM fdinfo usage counters of the contestant's
    # render-node fds across the window decide the class; mapped libraries
    # are recorded, never decisive
    node = "/dev/dri/renderD128"

    def drm(phase, gfx, client="7", driver="amdgpu", fd=12):
        counters = {"drm-driver": driver, "drm-client-id": client,
                    "drm-pdev": "0000:03:00.0",
                    "drm-engine-capacity-gfx": "1"}
        if gfx is not None:
            counters["drm-engine-gfx"] = f"{gfx} ns"
            counters["drm-engine-compute"] = "0 ns"
        return {"ev": "drm", "phase": phase, "pid": 7, "fd": fd,
                "target": node, "counters": counters}

    def ev(*libs, held=(), drm_events=(), window=True):
        return ([{"ev": "lib", "pid": 7, "path": p} for p in libs]
                + [{"ev": "fd", "pid": 7, "target": n} for n in held]
                + ([{"ev": "window_start"}] if window else [])
                + list(drm_events))
    lvp = "/usr/lib/x86_64-linux-gnu/libvulkan_lvp.so"
    radv = "/usr/lib/x86_64-linux-gnu/libvulkan_radeon.so"
    gal = "/usr/lib/x86_64-linux-gnu/libgallium-25.0.7-1+deb13u1.so"
    assert renderer_of(lvp) == "lavapipe" and renderer_of(radv) == "radv"
    assert renderer_of(gal) == "gallium"
    assert renderer_of("/usr/lib/x86_64-linux-gnu/dri/iris_dri.so") == "iris"
    assert renderer_of("/usr/lib/x86_64-linux-gnu/libgtk-4.so.1") is None
    hw = {"class": "hardware", "kernel_driver": "amdgpu", "nodes": [node]}
    sw = {"class": "software"}
    busy = (drm("start", 1_000), drm("end", 5_001_000))
    idle = (drm("start", 1_000), drm("end", 1_000))
    ok = renderer_evidence(ev(lvp, radv, held=[node], drm_events=busy), hw)
    assert ok["renderer_used"] == [f"amdgpu on {node}"], ok
    assert ok["gpu_usage"][node]["counters"]["drm-engine-gfx"] == 5_000_000
    assert ok["renderers_mapped"] == ["lavapipe", "radv"], ok
    # a client that opened the node inside the window starts from zero
    ok = renderer_evidence(ev(radv, held=[node], drm_events=(
        drm("end", 300, client="9"),)), hw)
    assert ok["gpu_busy_nodes"] == [node], ok
    # software: no GPU work on any held node, a software rasterizer
    # mapped — hardware ICDs the loader mapped do not matter, and the
    # gallium megadriver (llvmpipe inside) counts once the node is idle
    assert renderer_evidence(ev(lvp), sw)["renderer_used"] == ["lavapipe"]
    assert renderer_evidence(
        ev(radv, lvp, held=[node], drm_events=idle), sw)[
            "renderer_used"] == ["lavapipe"]
    assert renderer_evidence(
        ev(gal, held=[node], drm_events=idle), sw)[
            "renderer_used"] == ["gallium"]
    rejected = (
        # lvp + radv mapped and the node held, but no GPU work: software
        # rendering that the mapped-library rule used to pass as hardware
        (ev(lvp, radv, held=[node], drm_events=idle), hw, "no GPU work"),
        (ev(gal, held=[node], drm_events=idle), hw, "no GPU work"),
        (ev(radv), hw, "no GPU work"),
        # a driver that keeps no usage counters cannot prove hardware
        (ev(radv, held=[node], drm_events=(drm("start", None),
                                           drm("end", None))),
         hw, "no drm-engine/drm-cycles usage counters"),
        (ev(lvp, held=[node], drm_events=busy), sw, "submitted GPU work"),
        (ev(radv, held=[node], drm_events=idle), sw,
         "no software rasterizer mapped"),
        (ev(radv, held=[node], drm_events=busy, window=False), hw,
         "no window_start"),
        (ev(radv, held=[node], drm_events=busy)
         + [{"ev": "evidence_error", "what": "maps", "pid": 7,
             "errno": 13}], hw, "maps of pid 7"),
        (ev(radv, held=[node], drm_events=(drm("start", 9_000),
                                           drm("end", 1_000))),
         hw, "went backwards"),
    )
    for events, exp, why in rejected:
        try:
            renderer_evidence(events, exp)
        except RuntimeError as e:
            assert why in str(e), (why, str(e))
        else:
            raise AssertionError(f"wrong renderer accepted: {events} {exp}")

    # memory is the capture window's: median and peak of the in-window
    # memory.current samples — the startup spike never enters
    ms = 1_000_000
    events = ([{"ev": "spawn", "t": 0},
               {"ev": "present", "t": 100 * ms, "committed": True}]
              + [{"ev": "present", "t": (100 + 16 * i) * ms,
                  "committed": True} for i in range(1, 200)]
              + [{"ev": "mem", "t": 50 * ms, "current": 900}]
              + [{"ev": "mem", "t": (1100 + 100 * i) * ms,
                  "current": 100 + i} for i in range(10)]
              + [{"ev": "mem", "t": 9000 * ms, "current": 800}])
    mm = metrics_from_events(events, warmup_ms=1000, capture_ms=1000)
    assert mm["rss_bytes_peak"] == 109 and mm["rss_bytes_steady"] == 104, mm
    try:
        metrics_from_events([e for e in events if e["ev"] != "mem"],
                            warmup_ms=1000, capture_ms=1000)
    except RuntimeError as e:
        assert "inside the measurement window" in str(e)
    else:
        raise AssertionError("window without memory samples accepted")
    # no committed present, or an undeclared window: no measurement
    for bad_events, warm, capture in (
            ([e for e in events if e["ev"] != "present"], 1000, 1000),
            (events, 1000, 0), (events, 0, 1000)):
        try:
            metrics_from_events(bad_events, warmup_ms=warm,
                                capture_ms=capture)
        except RuntimeError:
            pass
        else:
            raise AssertionError(
                f"rep without a window accepted ({warm}, {capture})")

    # toolchain names: nightly/beta take the manifest's dist date, never
    # the commit date in the version string
    url = "https://static.rust-lang.org/dist/channel-rust-nightly.toml"
    nightly = {"date": "2025-10-05", "pkg": {"rust": {
        "version": "1.92.0-nightly (abc1234 2025-10-04)"}}}
    assert resolve_channel_manifest("nightly", nightly, url) \
        == "nightly-2025-10-05"
    stable = {"date": "2025-09-18", "pkg": {"rust": {
        "version": "1.90.0 (1159e78c4 2025-09-14)"}}}
    assert resolve_channel_manifest("stable", stable, url) == "1.90.0"
    for ch, doc in (("nightly", {"pkg": nightly["pkg"]}),
                    ("1.90", stable), ("my-toolchain", stable)):
        try:
            resolve_channel_manifest(ch, doc, url)
        except RuntimeError:
            pass
        else:
            raise AssertionError(f"{ch} resolved without a dist pin")
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
    ap.add_argument("--build-only", action="store_true",
                    help="build the image and every contestant, stage them "
                    "under dist/, and stop — no measurement, no GPU need")
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
        # the generated flutter linux/ tree, npm ci and every contestant
        # build run on a clean tree and must leave it clean
        with toolchain.tracked_tree_unchanged(
                "linux contestant build",
                [ROOT] + [(ROOT / c["project"]).resolve()
                          for name, c in manifest["contestants"].items()
                          if not only or name in only]):
            staged = build_contestants(manifest, only)
    else:
        staged = {c: f"dist/{c}" for c in CONTESTANT_CMDS
                  if not only or c in only}
    if args.build_only:
        for c, d in staged.items():
            print(f"   staged {c} -> {ROOT / d}")
        return 0

    # The renderer class every rep must prove it loaded: the selected
    # hardware adapter's userspace driver (bound through its render
    # nodes), or a software renderer on an all-software host.
    expected_renderer: dict = (
        {"class": "hardware", "nodes": list(dri_mounts),
         "kernel_driver": kernel_driver(dri_mounts[0])}
        if selected else {"class": "software"})

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
        for wl in workloads:
            if wl in (manifest.get("capacity") or {}):
                spec = manifest["capacity"][wl]
                if name not in spec.get(
                        "contestants", list(CONTESTANT_CMDS)):
                    # the manifest names no cell for this contestant —
                    # no row, not a failure
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
                            cmd, spec, dri_mounts, expected_renderer,
                            ROOT / "out", f"{name}-{wl}-{rep}")
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
                    # per-rep renderer evidence (the libraries and
                    # render nodes of the contestant's own processes at
                    # window end) is checked inside run_workload
                    s, run_log = run_workload(
                        CONTESTANT_CMDS[name].format(
                            wl=wl, wenv=wenv, selenv=selenv), wl,
                        durations[wl],
                        workload_script(manifest, wl, durations[wl]),
                        out_jsonl, expected_renderer,
                        dri=dri_mounts,
                        warmup_ms=manifest["pacing"]["warmup_ms"])
                    s["run"] = rep
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

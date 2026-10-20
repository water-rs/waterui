#!/usr/bin/env python3
"""Interleaved A/B timing of two `cherenkov-bench` Android builds on a rooted
Pixel-class device.

  pixel_compare.py run     --base B --head H --scene DIR... --out O
                           [--governor | --gpu-counters]
  pixel_compare.py run     --base B --head H --scene DIR... --out O
                           --engine E --workers 1 --cpus LIST --cluster-khz POLICY=KHZ...
                           [--devfreq-khz NAME=KHZ...]
  pixel_compare.py run     --base B --head H --scene DIR... --out O
                           --engine E --workers N --cpus LIST
  pixel_compare.py analyze O
  pixel_compare.py quiet-limit O...

`run` pushes both binaries and the scene directories, then measures every
scene in rounds of base/head pairs: `base head` then `head base`, alternating,
so neither arm always runs first. Each invocation is one `measure` process:
`--rate 120 --energy --cpu 7`, the warmup and measured frame counts given, and
`--native W×H` (width `--width`, the scene's own aspect) for scenes under a
`perf/` directory. Every completed report is kept and nothing is dropped;
only the quiet gate below reruns an invocation.

A GPU's busy time at a given workload scales with its clock, and the default
GPU governor picks the clock from recent utilization: a fresh process, a
120 Hz pacing gap or a short scene leaves it at whatever operating point the
previous seconds settled on. Identical binaries then differ by the clocks
they happened to get, not by their cost (#2613). So `run` measures code cost
with both the GPU and the measuring CPU held at one operating point:
`scaling_min_freq = scaling_max_freq = --gpu-khz / --cpu-khz` for each
workload. The device-side supervisor selects the vendor's CPU performance
profiles, releases CPU skin-temperature votes and selects the performance
governor. It restores every setting when the workload exits, times out or is
interrupted. Physical CPU/GPU temperatures are checked against their first
passive hardware trip; the battery is checked against `--battery-c`.
Crossing a limit terminates the workload. Thermal status is recorded without
waiting for it to clear. Emergency hardware protection remains enabled.
`--governor` leaves performance policies unchanged and measures the delivered
frame budget instead of fixed-clock cost.

A frame whose GPU time is a fraction of a millisecond is latency-bound on the
Pixel's Mali: added work is absorbed or changes the GPU's power state, so its
GPU time is not a cost measure at 2% (#2775). `--gpu-counters` passes
`measure --gpu-counters`, which records each frame's Mali shader-core active
cycles, and `analyze` then compares those cycles instead of GPU time. Each
process still settles at its own cycle level, a few percent apart for
identical binaries, while frames within it agree, so such a scene needs many
short invocations: `--rounds 32 --frames 300` resolves the 96 px corpus
scenes.

`--engine cherenkov-cpu` (or the reference CPU engines `vello-cpu` and
`skia-cpu`) measures a CPU rasterizer instead: `measure --engine E --cpu
LIST` with every bench thread confined to the CPU set `--cpus`, the engine at
`--workers` workers, each scene at its own size, and no pacing or energy
window, so frames run back to back. The worker count reaches the engine
through its own control: cherenkov-cpu's raster pool, and rayon's global pool
beside it, through `RAYON_NUM_THREADS`; vello-cpu's dispatcher through
`VELLO_CPU_THREADS` (one worker is its single-threaded dispatcher, N its
multithreaded one with N workers); skia-cpu is single-threaded and runs at
one worker only. The GPU is left to its governor.

The worker count selects what a CPU-engine collection decides (#2988). At one
worker it compares cost: every CPU frequency policy holding a CPU of the set
is held at its `--cluster-khz` clock for the workload, original policy values
are restored after each invocation, and `analyze` decides the change by the
rule below. At more workers the scheduler places the workers on different
clusters from run to run, which moves identical builds by more than the
limit, so the collection checks the frame budget instead: the clocks stay
with their governors, as with `--governor`, and `analyze` reports each arm's
frame p99 against `--budget-ms` without deciding a change.

For cherenkov-cpu, "N workers" means N raster workers confined to `--cpus`,
while the frame's serial work runs on CPU 7 alone: `measure --render-cpu 7`
moves the engine's render thread (lowering, glyph resolution and the frame
handoff) and the measuring thread (encode) there once the engine has built
its pool. With one mask over a big.LITTLE set those serial phases would land
on a big or a little core from run to run (#2988). `--cpus` must hold CPU 7;
at one worker (`--cpus 7`) everything shares it. Each report lists every
thread's CPU mask and its CPU time over the measured window, and a run whose
render thread, raster workers or other active threads are not where they
were put is invalid.

The memory, L3 and interconnect clocks (`mif`, `dsu`, `bci`: the devfreq
devices `*.devfreq_NAME`) follow their own governors. Every run records
their residency over the workload (`interconnect_residency_ms` in the run
log), and in a comparison `--devfreq-khz NAME=KHZ` holds one at a clock
(`min_freq = max_freq`), restored afterwards; a held clock's residency
outside that clock invalidates the run. None is held by default: what moves
a one-worker run is the memory controller's low-power state, which these
clocks do not reach. A one-worker frame misses its caches only sparsely, the
controller powers down in the gaps, and how often depends on what else
reaches DRAM at the time, so identical one-worker runs moved by a percent
and more, within a process over seconds, and outside activity made them
faster (#2988). A one-worker comparison therefore runs the base arm's
`memory-keepalive` on CPU 0 beside every invocation of either arm: one read
of a 64 MiB buffer every microsecond keeps the controller active. The
supervisor starts the workload on its `ready` signal and fails the run when
it exits early; as the supervisor's child it never counts as another
process.

Controls, checked for every invocation and recorded with it:

- before: screen off, physical sensors below their limits, and the requested
  device settings applied with readback evidence;
- during: physical temperature supervision, GPU clock residency, CPU and
  thread placement, and the requested worker count;
- after: pinned policy ranges, CPU frequency and interconnect clock residency
  over the workload, followed by restoration of every original property and
  sysfs value.

The supervisor checks the temperatures every 250 ms beside the frames. A
check reads the sensors with shell builtins and waits on a FIFO with a read
timeout, so it forks nothing and appends to no file; the readings are
written once the workload has exited.

Other processes take CPU time from a run's frames, most of all when the
workers hold every CPU. The device supervisor therefore snapshots every
process's cumulative CPU time (`/proc/PID/stat`) and the uptime just before
it starts the workload and just after it reaps it, so the snapshots cost the
frames nothing. Each run records the other processes' CPU per second of that
window and the processes that used the most (`quiet` in `runs.jsonl`). With
`--quiet-limit MS_PER_S`, a run above the limit is rerun as `RUN_ID-rN`: the
loud run and its report stay on disk, are logged as rejected and reported by
`analyze`, which decides on the accepted runs. A slot still loud after
`--quiet-attempts` runs stops the collection. `quiet-limit O...` derives the
limit from collections of quiet runs: their median plus three robust
standard deviations (1.4826 × the median absolute deviation).

A failed control marks the invocation invalid and stops the collection:
`analyze` reports a scene with an invalid invocation as `invalid`, never as
a pass, and the raw report stays on disk.

`analyze` reads a collection and, per scene and per metric of the plan, takes
each invocation's p50 of the metric over its measured frames, the median of
those per arm, and their change. The metrics are GPU milliseconds (or
shader-core active cycles with `--gpu-counters`) for the GPU engine; for a
CPU engine the frame (encode and submit together, in milliseconds: an engine
may rasterize in either, as vello-cpu dispatches strips while it records),
and for cherenkov-cpu also its raster pass (`shade`, the backend's phase). A
p50 over a finite frame count has a sampling error, and a fresh process can
land at a different cost, so the change alone does not say whether the
collection can tell a 2% change from none. `analyze` therefore puts a 95%
interval around the change by a hierarchical bootstrap (invocations, then the
frames of each). A metric whose whole interval lies above `--limit-percent`
(2%) `fails` whatever the interval's width: the collection has shown the
change exceeds the limit at that confidence. Any other metric is decided only
when its interval is no wider than the limit (half-width at most half of
`--limit-percent`), so a true change of zero reads above the limit with
probability far below a percent; it then `fails` above the limit and passes
otherwise. A wider interval is `unresolved`, never a pass. The limit bounds a
cost increase, so an interval wholly below the negative limit — the head
measurably cheaper — is no failure, and like any other it passes only at that
width and is `unresolved` beyond it. The scene's verdict is its first
metric's (GPU time, cycles or the frame). Each metric also carries the change
of the median invocation p99 and its interval, which decide nothing. More
`--frames` shrink the sampling error of each p50 and more `--rounds` average
over more processes; full-size scenes resolve on GPU time at the defaults,
and scenes of a fraction of a millisecond on shader-core cycles. In governor
mode, and for a CPU engine at more than one worker, `analyze` judges only
the first metric (GPU time or the frame): it reports each arm's worst and
median invocation p99 against `--budget-ms`, is `over budget` when the
head's worst exceeds it, and makes no cost decision.

For cherenkov-cpu `analyze` also reports, per arm, how much of `submit` the
backend's phases (`lower`, `glyph`, `shade`) leave unaccounted: the render
thread's message handoff and the frame's work outside the three phases.

`analyze` needs from a collection only `plan.json` (`scenes`, `rounds`,
`frames`, `warmup`, `metrics`, `engine`, `clocks`), `control.json`
(`completed`), `runs.jsonl` (each row's `rejected` and `quiet`) and the
reports, so a collector for another
device can write the same layout and be judged by the same rule.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
import re
import shlex
import shutil
import signal
import statistics
import subprocess
import sys
import time
import typing
from pathlib import Path
from typing import Any


def phase(sample: dict[str, Any], name: str) -> float:
    """Seconds of the frame's phase `name`; a frame that does not report it
    exactly once is an error."""
    matches = [p["seconds"] for p in sample["phases"] if p["name"] == name]
    if len(matches) != 1:
        raise SystemExit(f"a frame reports phase {name!r} {len(matches)} times")
    return matches[0]


# What `analyze` compares per frame: GPU milliseconds from timestamps, the
# shader cores' active cycles from the Mali counters (`--gpu-counters`), a CPU
# engine's frame milliseconds, or the CPU backend's raster pass.
METRICS = {
    "gpu-time": ("ms", lambda sample: sample["gpu_seconds"] * 1000),
    "shader-core-cycles": ("cycles",
                           lambda sample: sample["gpu_counters"]["shader_core_active_cycles"]),
    "frame": ("ms", lambda sample: (sample["encode_seconds"] + sample["submit_seconds"]) * 1000),
    "shade": ("ms", lambda sample: phase(sample, "shade") * 1000),
}
# The CPU backend's phases, in render order; they run inside `submit`.
CPU_PHASES = ("lower", "glyph", "shade")


class CpuEngine(typing.NamedTuple):
    """A CPU rasterizer the bench measures: the environment that sets its
    worker count, the `device.adapter` a report states for that count, the
    metrics `analyze` decides it on, its most workers, and whether it runs
    its frame's serial work on a render thread of its own."""
    environment: typing.Callable[[int], dict[str, str]]
    adapter: typing.Callable[[int], str]
    metrics: tuple[str, ...]
    max_workers: int | None
    render_thread: bool


CPU_ENGINES = {
    "cherenkov-cpu": CpuEngine(lambda n: {"RAYON_NUM_THREADS": str(n)},
                               lambda n: f"cherenkov-cpu ({n} threads)", ("frame", "shade"), None,
                               True),
    "vello-cpu": CpuEngine(lambda n: {"VELLO_CPU_THREADS": "0" if n == 1 else str(n)},
                           lambda n: "vello-cpu (single-threaded dispatcher)" if n == 1
                           else f"vello-cpu ({n} threads)", ("frame",), None, False),
    "skia-cpu": CpuEngine(lambda n: {}, lambda n: "Skia raster surface", ("frame",), 1, False),
}
# The kernel's names (15 bytes) for cherenkov's render thread and raster
# workers (`cherenkov-render`, `cherenkov-raster-N`).
RENDER_THREAD = "cherenkov-rende"
RASTER_THREAD = "cherenkov-raste"
GPU_ENGINE = "cherenkov"

# A collection's identity: every field `analyze` relies on is in the plan.
PLAN = "plan.json"
CONTROL = "control.json"
RUNS = "runs.jsonl"
RATE_HZ = 120
BOOTSTRAP_RESAMPLES = 2000
# How far above the quiet runs' median, in robust standard deviations, the
# other processes' CPU of a run may lie before it is rerun.
QUIET_SIGMAS = 3
# The GPU engine's measuring CPU, and a CPU engine's render CPU.
CPU = 7
# Where a one-worker comparison runs `memory-keepalive`, and its reads per
# second: enough that the memory controller never idles long enough to
# power down beside the workload.
KEEPALIVE_CPU = 0
KEEPALIVE_RATE = 1_000_000
# Memory, L3 and interconnect clocks (devfreq devices `*.devfreq_NAME`),
# whose residency every run records and `--devfreq-khz` holds.
INTERCONNECT = ("mif", "dsu", "bci")
DEVFREQ = "/sys/class/devfreq"
# CoolingDevice types of `android.os.CoolingDevice`: CPU and GPU.
COOLING_TYPES = (2, 3)
CPUFREQ = "/sys/devices/system/cpu/cpufreq"


class ControlError(Exception):
    """A device control did not hold; the collection stops."""


class Device:
    """`adb -s SERIAL` with root shell helpers and deadlines on every call."""

    def __init__(self, serial: str) -> None:
        self.serial = serial

    def adb(self, *args: str, timeout: float = 60) -> str:
        result = subprocess.run(["adb", "-s", self.serial, *args], text=True,
                                capture_output=True, timeout=timeout, check=False)
        if result.returncode != 0:
            raise RuntimeError(f"adb {' '.join(args)}: exit {result.returncode}\n"
                               f"{result.stdout}{result.stderr}")
        return result.stdout

    def root(self, command: str, timeout: float = 60) -> str:
        return self.adb("shell", "su -c " + shlex.quote(command), timeout=timeout)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree_sha256(directory: Path) -> dict[str, str]:
    return {str(p.relative_to(directory)): sha256(p)
            for p in sorted(directory.rglob("*")) if p.is_file()}


def save(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def cpu_list(text: str) -> list[int]:
    """The CPUs of a `measure --cpu` list such as `0-3,7`, sorted."""
    cpus: set[int] = set()
    for part in text.split(","):
        low, _, high = part.partition("-")
        cpus.update(range(int(low), int(high or low) + 1))
    return sorted(cpus)


def cpu_cell(workers: int) -> str:
    """What a CPU-engine collection at `workers` workers decides: the cost
    change at one worker, the frame budget at more (#2988)."""
    return "comparison" if workers == 1 else "budget"


class Process(typing.NamedTuple):
    """One line of `/proc/PID/stat`: its parent, its name, the CPU ticks it
    spent itself and the ticks its reaped children spent."""
    pid: int
    ppid: int
    comm: str
    own: int
    children: int


def processes(text: str) -> dict[tuple[int, int], Process]:
    """Every process of a `cat /proc/[0-9]*/stat`, keyed by pid and start
    time so that a reused pid is a different process."""
    out = {}
    for line in text.splitlines():
        head, _, rest = line.rpartition(")")
        pid, _, comm = head.partition(" (")
        fields = rest.split()
        utime, stime, cutime, cstime = map(int, fields[11:15])
        out[int(pid), int(fields[19])] = Process(int(pid), int(fields[1]), comm,
                                                 utime + stime, cutime + cstime)
    return out


def uptime(text: str) -> float:
    """Uptime seconds from a `cat /proc/uptime /proc/stat`; the per-CPU
    counters after it are kept as raw evidence only."""
    return float(text.split(maxsplit=1)[0])


def outside_cpu(directory: Path, tick_hz: int) -> dict[str, Any]:
    """What ran beside the workload over its supervised window, from the
    supervisor's before and after snapshots.

    The workload starts after the first snapshot and is reaped by the
    supervisor before the second, so its CPU time is in neither: it lands in
    the supervisor's reaped-children counter, which is reported as the
    workload's and never counted as other. The supervisor's own time (its
    temperature polling) is counted as other: it runs beside every frame.
    Its other children (`sleep`, the snapshot `cat`) are excluded.

    Other processes' CPU is the change of their own and reaped-children time;
    a process born in the window counts whole. A process that exits in the
    window disappears, and its whole lifetime then reaches an ancestor's
    reaped-children time, so its time before the window is taken back there.
    The gate's quantity is that total per second of the window."""
    supervisor = int((directory / "supervisor-pid").read_text())
    before, after = ({key: proc for key, proc in
                      processes((directory / f"processes-{moment}").read_text()).items()
                      if proc.ppid != supervisor}
                     for moment in ("before", "after"))
    window = (uptime((directory / "system-after").read_text())
              - uptime((directory / "system-before").read_text()))
    # A process that exited in the window carries its time before the window
    # into the reaped-children time of its nearest ancestor still alive. A
    # parent that does not wait for its children (kthreadd, a SIGCHLD-ignoring
    # daemon) receives nothing, so that ancestor's children delta bounds what
    # is taken back from it, and an exit's time inside the window is then
    # unknown and counts as none.
    alive = {proc.pid: key for key, proc in after.items() if key in before}
    by_pid = {proc.pid: proc for proc in before.values()}
    earlier: dict[tuple[int, int], int] = {}
    for key, proc in before.items():
        if key in after:
            continue
        parent = proc.ppid
        while parent not in alive and parent in by_pid:
            parent = by_pid[parent].ppid
        if parent in alive:
            earlier[alive[parent]] = earlier.get(alive[parent], 0) + proc.own + proc.children
    other: dict[tuple[int, int], int] = {}
    workload = 0
    for key, proc in after.items():
        if key not in before:
            other[key] = proc.own + proc.children
            continue
        own = proc.own - before[key].own
        children = proc.children - before[key].children
        if proc.pid == supervisor:
            workload, other[key] = children, own
        else:
            other[key] = own + max(0, children - earlier.get(key, 0))
    names = {key: proc.comm for key, proc in (before | after).items()}
    total = sum(other.values()) / tick_hz
    return {
        "window_seconds": window,
        "other_cpu_seconds": total,
        "other_ms_per_s": total * 1000 / window,
        "workload_cpu_seconds": workload / tick_hz,
        "top": [[names[key], key[0], ticks / tick_hz] for key, ticks in
                sorted(other.items(), key=lambda item: -item[1])[:8] if ticks > 0],
    }


class Clocks:
    """Frequency policies read and pinned through sysfs, by name: `gpu` is
    the Mali device behind `/sys/class/misc/mali0`, `policyN` a CPU
    frequency policy, and a name of `INTERCONNECT` the devfreq device
    `DEVFREQ/*.devfreq_NAME`."""

    def __init__(self, device: Device, names: list[str]) -> None:
        self.device = device
        self.paths = {name: self.locate(device, name) for name in names}

    @staticmethod
    def locate(device: Device, name: str) -> str:
        if name == "gpu":
            return device.root("readlink -f /sys/class/misc/mali0/device").strip()
        if name in INTERCONNECT:
            found = device.root(f"ls -d {DEVFREQ}/*.devfreq_{name}").split()
            if len(found) != 1:
                raise ControlError(f"devfreq {name}: {found}")
            return device.root("readlink -f " + shlex.quote(found[0])).strip()
        return f"{CPUFREQ}/{name}"

    @staticmethod
    def nodes(name: str) -> tuple[str, str, str, str]:
        """A clock's lower and upper bound, current frequency and available
        frequencies, as node names in its directory."""
        if name in INTERCONNECT:
            return "min_freq", "max_freq", "cur_freq", "available_frequencies"
        if name == "gpu":
            return "scaling_min_freq", "scaling_max_freq", "cur_freq", "available_frequencies"
        return ("scaling_min_freq", "scaling_max_freq", "scaling_cur_freq",
                "scaling_available_frequencies")

    @staticmethod
    def describe(device: Device) -> dict[str, dict[str, Any]]:
        """Every CPU frequency policy: its CPUs, its hardware ceiling and its
        available clocks."""
        out = {}
        for name in device.root(f"ls {CPUFREQ}").split():
            if not name.startswith("policy"):
                continue
            path = f"{CPUFREQ}/{name}"
            cpus, ceiling, available = device.root(
                f"cat {path}/related_cpus {path}/cpuinfo_max_freq "
                f"{path}/scaling_available_frequencies").split("\n")[:3]
            out[name] = {"cpus": [int(c) for c in cpus.split()], "cpuinfo_max": int(ceiling),
                         "available": [int(v) for v in available.split()]}
        return out

    def available(self, name: str) -> list[int]:
        node = self.nodes(name)[3]
        return [int(v) for v in self.device.root(f"cat {self.paths[name]}/{node}").split()]

    def state(self) -> dict[str, dict[str, int]]:
        state = {}
        for name, path in self.paths.items():
            low, high, current, _ = self.nodes(name)
            values = self.device.root(f"cat {path}/{low} {path}/{high} {path}/{current}").split()
            state[name] = dict(zip(("min", "max", "current"), map(int, values)))
        return state

    @staticmethod
    def residency_file(name: str) -> str:
        """The time-in-state table of a CPU policy (ticks) or an
        interconnect clock (milliseconds), relative to its directory."""
        return "time_in_state" if name in INTERCONNECT else "stats/time_in_state"


def residency_table(text: str, name: str) -> dict[int, int]:
    """A time-in-state table by kHz. A CPU policy's rows are `KHZ TICKS`; a
    devfreq table prints `%8lu%10lu`, so its columns touch once a time
    reaches ten digits and are split by width."""
    rows = text.splitlines()
    if name in INTERCONNECT:
        return {int(row[:8]): int(row[8:]) for row in rows}
    return {int(khz): int(ticks) for khz, ticks in (row.split() for row in rows)}



def thermal(device: Device) -> dict[str, Any]:
    raw = device.adb("shell", "dumpsys thermalservice")
    status = int(re.search(r"Thermal Status:\s*(\d+)", raw)[1])
    current = raw.split("Current temperatures from HAL:", 1)[1].split(
        "Current cooling devices from HAL:", 1)[0]
    battery = float(re.search(r"mValue=([-\d.]+), mType=2, mName=battery", current)[1])
    cooling_text = raw.split("Current cooling devices from HAL:", 1)[1].split(
        "Temperature static thresholds", 1)[0]
    cooling = {name: int(value) for value, kind, name in re.findall(
        r"CoolingDevice\{mValue=(\d+), mType=(\d+), mName=([^}]+)\}", cooling_text)
        if int(kind) in COOLING_TYPES}
    if not cooling:
        raise ControlError("thermalservice reports no CPU or GPU cooling device")
    return {"status": status, "battery_c": battery, "cooling": cooling}


def screen_off(device: Device) -> str:
    device.adb("shell", "input keyevent KEYCODE_SLEEP")
    return re.search(r"mWakefulness=(\w+)", device.adb("shell", "dumpsys power"))[1]


class PerformanceControls:
    """Device-native policy controls and physical sensor limits for one workload."""

    def __init__(self, device: Device, clocks: Clocks, pinned: dict[str, int] | None,
                 battery_c: float) -> None:
        self.arguments: list[str] = []
        self.sensors: list[dict[str, Any]] = []
        # Every interconnect clock's residency is recorded, held or not.
        for name in INTERCONNECT:
            self.arguments += ["--residency", name,
                               clocks.paths[name] + "/" + clocks.residency_file(name)]
        power = json.loads(device.root("cat /vendor/etc/powerhint.json"))
        nodes = {node["Name"]: node for node in power["Nodes"]}
        if pinned:
            for action in power["Actions"]:
                node = nodes.get(action.get("Node"))
                if (action["PowerHint"] == "GAME" and node is not None
                        and node.get("Type") == "Property"
                        and node["Path"].startswith("vendor.thermal.")
                        and node["Path"].endswith(".profile")):
                    self.arguments += ["--property", node["Path"], action["Value"]]
            for node in nodes.values():
                if node["Name"].endswith("_TSKIN_BYPASS"):
                    assert "1" in node["Values"]
                    self.arguments += ["--node", node["Path"], "1"]
            for name, khz in pinned.items():
                path = clocks.paths[name]
                if name in INTERCONNECT:
                    self.arguments += ["--devfreq", name, path, str(khz)]
                    continue
                if name != "gpu":
                    governors = device.root("cat " + shlex.quote(path + "/scaling_available_governors")).split()
                    if "performance" not in governors:
                        raise ControlError(f"{name} has no performance governor")
                    self.arguments += ["--node", path + "/scaling_governor", "performance",
                                       "--residency", name,
                                       path + "/" + clocks.residency_file(name)]
                self.arguments += ["--clock", name, path, str(khz)]

        config_name = device.root("getprop vendor.thermal.config").strip()
        sensors = self.load_sensors(device, config_name)
        for sensor in sensors.values():
            if sensor.get("VirtualSensor") or sensor.get("Type") not in ("CPU", "GPU", "BATTERY"):
                continue
            name = sensor["Name"]
            zone = device.root("readlink -f " + shlex.quote("/dev/thermal/tz-by-name/" + name)).strip()
            if not zone:
                raise ControlError(f"no physical sensor path for {name}")
            if sensor["Type"] == "BATTERY":
                limit = round(battery_c * 1000)
            else:
                types = device.root("ls " + shlex.quote(zone) + "/trip_point_*_type").split()
                limits = []
                for path in types:
                    if device.root("cat " + shlex.quote(path)).strip() == "passive":
                        limits.append(int(device.root("cat " + shlex.quote(path.removesuffix("_type") + "_temp")).strip()))
                if not limits:
                    raise ControlError(f"{name} has no physical throttling threshold")
                limit = min(limits)
            reading = {"name": name, "path": zone + "/temp", "limit_millicelsius": limit}
            self.sensors.append(reading)
            self.arguments += ["--sensor", name, reading["path"], str(limit)]
        if not self.sensors:
            raise ControlError("device exposes no physical temperature sensors")

    @classmethod
    def load_sensors(cls, device: Device, name: str) -> dict[str, dict[str, Any]]:
        config = json.loads(device.root("cat " + shlex.quote("/vendor/etc/" + name)))
        sensors: dict[str, dict[str, Any]] = {}
        for include in config.get("Include", []):
            sensors.update(cls.load_sensors(device, include))
        sensors.update({sensor["Name"]: sensor for sensor in config.get("Sensors", [])})
        return sensors

    @staticmethod
    def read_evidence(directory: Path) -> dict[str, Any]:
        """Read actual setup/restore values, never infer success from process exit alone."""
        controls = []
        exit_code = int((directory / "exit-code").read_text())
        count = int((directory / "registered-controls").read_text())
        for index in range(count):
            stem = directory / f"control-{index}"
            fields = {suffix: stem.with_suffix("." + suffix).read_text().strip()
                      for suffix in ("kind", "key", "original", "requested", "restored")}
            applied = stem.with_suffix(".applied")
            fields["applied"] = applied.read_text().strip() if applied.exists() else None
            if fields["original"] != fields["restored"]:
                raise ControlError(f"device setting not restored: {fields['key']}")
            if fields["kind"] == "clock":
                fields.update({suffix: stem.with_suffix("." + suffix).read_text().strip()
                               for suffix in ("name", "original-max", "restored-max")})
                maximum = stem.with_suffix(".applied-max")
                fields["applied-max"] = maximum.read_text().strip() if maximum.exists() else None
                if fields["original-max"] != fields["restored-max"]:
                    raise ControlError(f"clock ceiling not restored: {fields['key']}")
            if exit_code == 0 and (fields["applied"] != fields["requested"] or
                    (fields["kind"] == "clock" and fields["applied-max"] != fields["requested"])):
                raise ControlError(f"device setting not applied: {fields['key']}")
            controls.append(fields)
        return {"exit": exit_code, "settings": controls}


class Session:
    """One collection: the plan, the device, the controls and the run log."""

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.out: Path = args.out
        self.device = Device(args.serial)
        self.cpu_engine = CPU_ENGINES.get(args.engine)
        self.render_cpus = None
        self.remote = f"/data/local/tmp/pixel-compare-{self.out.name}"
        # What runs beside the workload under the supervisor.
        self.beside: list[str] = []
        if self.cpu_engine is None:
            self.cpus = [CPU]
            self.pinned = None if args.governor else {"gpu": args.gpu_khz,
                                                      self.policy_of(CPU): args.cpu_khz}
        else:
            self.cpus = cpu_list(args.cpus)
            if cpu_cell(args.workers) == "comparison":
                self.pinned = dict(args.cluster_khz)
                if KEEPALIVE_CPU in self.cpus:
                    raise SystemExit(f"--cpus must leave CPU {KEEPALIVE_CPU} to the memory "
                                     "keep-alive")
                self.beside = ["--keepalive", f"{self.remote}/bench-base", str(KEEPALIVE_CPU),
                               str(KEEPALIVE_RATE)]
            else:
                self.pinned = None
            if self.cpu_engine.render_thread:
                if CPU not in self.cpus:
                    raise SystemExit(f"--cpus must hold the render thread's CPU {CPU}")
                self.render_cpus = [CPU]
        if self.pinned is not None:
            self.pinned |= dict(args.devfreq_khz)
        names = list(self.pinned or {"gpu": 0, self.policy_of(CPU): 0})
        self.clocks = Clocks(self.device, names + [n for n in INTERCONNECT if n not in names])
        self.performance = PerformanceControls(self.device, self.clocks, self.pinned, args.battery_c)
        self.device_runs: dict[str, dict[str, Any]] = {}
        self.tick_hz = int(self.device.adb("shell", "getconf CLK_TCK"))

    def policy_of(self, cpu: int) -> str:
        return self.device.root(f"readlink -f /sys/devices/system/cpu/cpu{cpu}/cpufreq").strip(
            ).rsplit("/", 1)[1]

    def snapshot(self) -> dict[str, Any]:
        wakefulness = screen_off(self.device)
        if wakefulness == "Awake":
            raise ControlError("screen remained awake")
        return {"wakefulness": wakefulness, "thermal": thermal(self.device),
                "clocks": self.clocks.state()}

    def plan(self) -> list[dict[str, Any]]:
        args = self.args
        self.out.mkdir(parents=True)
        arms = {"base": args.base, "head": args.head}
        scenes = []
        for directory in args.scene:
            perf = directory.parent.name == "perf"
            scenes.append({"name": directory.name, "path": str(directory), "perf": perf,
                           "sha256": tree_sha256(directory)})
        if len({s["name"] for s in scenes}) != len(scenes):
            raise SystemExit("scene names must be unique")
        policies = None
        if self.cpu_engine is not None:
            policies = Clocks.describe(self.device)
            holding = {name for name, policy in policies.items()
                       if set(policy["cpus"]) & set(self.cpus)}
            if self.pinned is not None and set(self.pinned) - set(INTERCONNECT) != holding:
                raise SystemExit(f"--cluster-khz must name exactly the policies holding CPUs "
                                 f"{self.cpus}: {sorted(holding)}")
        if self.pinned:
            for name, khz in self.pinned.items():
                if khz not in self.clocks.available(name):
                    raise SystemExit(f"{name} {khz} kHz is not an available frequency")
        if self.cpu_engine is None:
            metrics = ["shader-core-cycles" if args.gpu_counters else "gpu-time"]
        elif self.pinned is None:
            # A frame-budget check judges the frame alone.
            metrics = list(self.cpu_engine.metrics[:1])
        else:
            metrics = list(self.cpu_engine.metrics)
        plan = {
            "arms": {side: {"path": str(path), "sha256": sha256(path)} for side, path in arms.items()},
            "scenes": scenes, "rounds": args.rounds, "frames": args.frames,
            "warmup": args.warmup, "width": args.width, "rate_hz": RATE_HZ,
            "engine": args.engine, "workers": args.workers, "cpus": self.cpus,
            "render_cpus": self.render_cpus,
            "memory_keepalive": ({"cpu": KEEPALIVE_CPU, "reads_per_s": KEEPALIVE_RATE,
                                  "bench": "base"} if self.beside else None),
            "metrics": metrics, "policies": policies,
            "clocks": "governor" if self.pinned is None else self.pinned,
            "battery_c": args.battery_c, "serial": args.serial,
            "tick_hz": self.tick_hz, "quiet_limit_ms_per_s": args.quiet_limit,
            "quiet_attempts": args.quiet_attempts,
            "script_sha256": sha256(Path(__file__)),
            "supervisor_sha256": sha256(Path(__file__).with_name("pixel_run.sh")),
            "physical_temperature_limits": self.performance.sensors,
            "device_controls": self.performance.arguments,
        }
        save(self.out / PLAN, plan)
        if self.device.root("id -u").strip() != "0":
            raise SystemExit("the device's su does not grant root")
        return scenes

    def push(self, scenes: list[dict[str, Any]]) -> None:
        """Pushes both arms and every scene into the collection's remote
        directory, which the shell user owns so `adb push` can write it."""
        plan = json.loads((self.out / PLAN).read_text())
        supervisor = self.remote + "/pixel_run.sh"
        self.device.adb("push", str(Path(__file__).with_name("pixel_run.sh")), supervisor, timeout=30)
        if self.device.root("sha256sum " + shlex.quote(supervisor)).split()[0] != plan["supervisor_sha256"]:
            raise ControlError("uploaded device supervisor hash differs from plan")
        arms = {side: Path(arm["path"]) for side, arm in plan["arms"].items()}
        for side, path in arms.items():
            target = f"{self.remote}/bench-{side}"
            self.device.adb("push", str(path), target, timeout=180)
            self.device.root(f"chmod 755 {target}")
            if self.device.root(f"sha256sum {target}").split()[0] != plan["arms"][side]["sha256"]:
                raise SystemExit(f"pushed {side} binary does not match its hash")
        for scene in scenes:
            self.device.adb("push", scene["path"], f"{self.remote}/scenes/{scene['name']}",
                            timeout=180)

    def measure(self, scene: dict[str, Any], side: str, run_id: str) -> Path:
        args = self.args
        report = f"{self.remote}/{run_id}.json"
        command = [f"{self.remote}/bench-{side}", "measure", "--engine", args.engine,
                   "--scene", f"{self.remote}/scenes/{scene['name']}",
                   "--cpu", ",".join(map(str, self.cpus)),
                   "--warmup", str(args.warmup), "--frames", str(args.frames), "--out", report]
        environment: dict[str, str] = {}
        if self.cpu_engine is None:
            command += ["--rate", str(RATE_HZ), "--energy"]
            if args.gpu_counters:
                command.append("--gpu-counters")
            if scene["perf"]:
                spec = json.loads((Path(scene["path"]) / "scene.json").read_text())
                height = round(args.width * spec["height"] / spec["width"])
                command += ["--native", f"{args.width}x{height}"]
        else:
            environment = self.cpu_engine.environment(args.workers)
            if self.render_cpus is not None:
                command += ["--render-cpu", ",".join(map(str, self.render_cpus))]
        prefix = ["env", *(f"{k}={v}" for k, v in environment.items())] if environment else []
        remote_control = f"{self.remote}/{run_id}-control"
        supervised = ["sh", self.remote + "/pixel_run.sh", "--out", remote_control,
                      "--seconds", str(args.run_seconds), *self.performance.arguments,
                      *self.beside, "--", *prefix, *command]
        process = subprocess.Popen(
            ["adb", "-s", self.device.serial, "shell", "su -c " + shlex.quote(shlex.join(supervised))],
            text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            log, _ = process.communicate(timeout=args.run_seconds + 30)
        except BaseException:
            self.device.root(
                "if [ -f " + shlex.quote(remote_control + "/supervisor-pid") + "]; then "
                "read -r pid < " + shlex.quote(remote_control + "/supervisor-pid") +
                '; kill -TERM "$pid"; fi', timeout=10)
            log, _ = process.communicate(timeout=15)
            (self.out / f"{run_id}.log").write_text(log)
            raise
        finally:
            self.device.adb("pull", remote_control, str(self.out / f"{run_id}-control"), timeout=30)
        (self.out / f"{run_id}.log").write_text(log)
        evidence = PerformanceControls.read_evidence(self.out / f"{run_id}-control")
        self.device_runs[run_id] = evidence
        if process.returncode != 0 or evidence["exit"] != 0:
            raise ControlError(f"{run_id}: device supervisor exited {process.returncode}; see {run_id}.log")
        local = self.out / f"{run_id}.json"
        self.device.adb("pull", report, str(local), timeout=60)
        return local

    def check_report(self, local: Path) -> list[str]:
        """The in-window controls a report carries."""
        report = json.loads(local.read_text())
        if "unsupported" in report:
            return [f"the engine does not support {report['unsupported']}"]
        failures = []
        if report["conditions"]["screen_state"] not in ("off", "dozing", "asleep"):
            failures.append(f"screen {report['conditions']['screen_state']}")
        placement = report["placement"]
        if not (placement["controlled"] and placement["requested"] == self.cpus
                and placement["render"] == self.render_cpus):
            failures.append("placement not pinned")
        measuring = self.render_cpus or self.cpus
        if any(s["cpu_start"] not in measuring or s["cpu_end"] not in measuring
               for s in report["samples"]):
            failures.append(f"measuring thread left CPUs {measuring}")
        if self.render_cpus is not None:
            failures += thread_placement(placement["threads"], self.cpus, self.render_cpus)
        if self.cpu_engine is not None:
            adapter = self.cpu_engine.adapter(self.args.workers)
            if report["device"]["adapter"] != adapter:
                failures.append(f"the engine reports {report['device']['adapter']!r}, "
                                f"not {adapter!r}")
            return failures
        if self.args.gpu_counters and (report.get("gpu_counters") is None or any(
                s.get("gpu_counters") is None for s in report["samples"])):
            failures.append("report carries no per-frame GPU counters")
        residency = report["gpu_clock"]
        if residency is None:
            failures.append("report carries no GPU clock residency")
        elif self.pinned:
            other = {r["khz"]: r["milliseconds"] for r in residency["residency"]
                     if r["khz"] != self.pinned["gpu"] and r["milliseconds"] > 0}
            if other:
                failures.append(f"GPU left {self.pinned['gpu']} kHz during the window: {other}")
        return failures

    def run(self) -> None:
        scenes = self.plan()
        control: dict[str, Any] = {"completed": False, "invocations": self.device_runs}
        control["original"] = self.clocks.state()
        save(self.out / CONTROL, control)
        self.device.adb("shell", f"mkdir {self.remote} {self.remote}/scenes")
        try:
            self.push(scenes)
            for scene in scenes:
                for pair in range(self.args.rounds):
                    order = ("base", "head") if pair % 2 == 0 else ("head", "base")
                    for side in order:
                        self.quiet_invocation(scene, pair, side)
            control["completed"] = True
        finally:
            self.device.root(f"rm -r {self.remote}")
            control["restored"] = self.clocks.state()
            save(self.out / CONTROL, control)

    def quiet_invocation(self, scene: dict[str, Any], pair: int, side: str) -> None:
        """Measures one invocation until other processes stayed within
        `--quiet-limit` over its window. A run beyond it is kept, logged as
        rejected and measured again; a run still beyond it after
        `--quiet-attempts` stops the collection."""
        for attempt in range(self.args.quiet_attempts):
            run_id = f"{scene['name']}-p{pair}-{side}" + (f"-r{attempt}" if attempt else "")
            if not self.invocation(scene, pair, side, run_id, attempt):
                return
        raise ControlError(f"{scene['name']}-p{pair}-{side}: other processes exceeded "
                           f"{self.args.quiet_limit} ms/s in {self.args.quiet_attempts} attempts")

    def invocation(self, scene: dict[str, Any], pair: int, side: str, run_id: str,
                   attempt: int) -> bool:
        """Measures one invocation between its controls and logs it; a
        failed control stops the collection. Returns whether the quiet gate
        rejected the run."""
        before = self.snapshot()
        local = self.measure(scene, side, run_id)
        directory = self.out / f"{run_id}-control"
        ranges = {}
        for name in self.pinned or {}:
            values = [int(v) for v in (directory / f"{name}-after-range").read_text().split()]
            ranges[name] = dict(zip(("min", "max"), values))
        residency = {}
        for name in [n for n in self.pinned or {} if n != "gpu" and n not in INTERCONNECT] + list(INTERCONNECT):
            readings = [residency_table((directory / f"{name}-{moment}-residency").read_text(), name)
                        for moment in ("before", "after")]
            residency[name] = {khz: value - readings[0][khz] for khz, value in readings[1].items()}
        after = {"thermal": thermal(self.device), "clocks": ranges,
                 "restored_clocks": self.clocks.state(),
                 "cpu_residency_ticks": {name: {k: v for k, v in table.items() if v}
                                         for name, table in residency.items()
                                         if name not in INTERCONNECT},
                 "interconnect_residency_ms": {name: {k: v for k, v in residency[name].items() if v}
                                               for name in INTERCONNECT}}
        failures = self.check_report(local)
        if self.pinned:
            # What a pinned run controls is the clocks: a cooling device
            # that moved none leaves the run valid. Each range is checked
            # after the run, and each CPU policy's and interconnect clock's
            # residency over the whole process.
            for name, khz in self.pinned.items():
                state = after["clocks"][name]
                if state["min"] != khz or state["max"] != khz:
                    failures.append(f"{name} policy after {state}")
                other = {k: v for k, v in residency.get(name, {}).items() if k != khz and v}
                if other:
                    failures.append(f"{name} left {khz} kHz: {other}")
        quiet = outside_cpu(directory, self.tick_hz)
        limit = self.args.quiet_limit
        rejected = limit is not None and quiet["other_ms_per_s"] > limit
        record = {"scene": scene["name"], "pair": pair, "side": side, "run_id": run_id,
                  "attempt": attempt, "report_sha256": sha256(local), "before": before,
                  "after": after, "quiet": quiet, "rejected": rejected, "failures": failures}
        with (self.out / RUNS).open("a") as stream:
            stream.write(json.dumps(record) + "\n")
        state = "invalid: " + "; ".join(failures) if failures else "valid"
        print("RUN", run_id, state, f"other {quiet['other_ms_per_s']:.1f} ms/s",
              "rejected, rerun" if rejected else "", flush=True)
        if failures:
            raise ControlError(f"{run_id}: {'; '.join(failures)}")
        return rejected



def thread_placement(threads: list[dict[str, Any]], cpus: list[int],
                     render: list[int]) -> list[str]:
    """What is wrong with a cherenkov-cpu report's threads when its render
    thread was moved to `render`: that thread must be there, every raster
    worker on `cpus`, and every other thread that ran in the window within
    `cpus`."""
    failures = []
    names = [t["name"] for t in threads]
    if names.count(RENDER_THREAD) != 1:
        failures.append(f"{names.count(RENDER_THREAD)} render threads")
    for thread in threads:
        if thread["name"] == RENDER_THREAD:
            expected = render
        elif thread["name"] == RASTER_THREAD:
            expected = cpus
        elif thread["window_ns"] == 0 or set(thread["allowed"]) <= set(cpus):
            continue
        else:
            expected = None
        if thread["allowed"] != expected:
            failures.append(f"thread {thread['tid']} {thread['name']} on {thread['allowed']}")
    return failures


def quantile(values: list[float], q: float) -> float:
    """Nearest-rank quantile, the bench's own definition."""
    ordered = sorted(values)
    return ordered[math.ceil(q * len(ordered)) - 1]


def median_change(arms: dict[str, list[list[float]]], q: float = 0.5) -> float:
    """The change of the median invocation quantile `q` (the p50 unless
    given), head against base, in percent."""
    medians = {side: statistics.median(quantile(run, q) for run in runs)
               for side, runs in arms.items()}
    return (medians["head"] / medians["base"] - 1) * 100


def change_interval(arms: dict[str, list[list[float]]], rng: random.Random,
                    q: float = 0.5) -> tuple[float, float]:
    """The 95% interval of `median_change` by a hierarchical bootstrap:
    each resample draws an arm's invocations with replacement, then each
    drawn invocation's frames with replacement, so both the spread between
    processes and the sampling error of a quantile over a finite frame count
    widen it."""
    changes = sorted(
        median_change({side: [rng.choices(run, k=len(run))
                              for run in rng.choices(runs, k=len(runs))]
                       for side, runs in arms.items()}, q)
        for _ in range(BOOTSTRAP_RESAMPLES))
    tail = round(BOOTSTRAP_RESAMPLES * 0.025)
    return changes[tail], changes[-tail - 1]


def decide(change: float, low: float, high: float, limit_percent: float) -> str:
    """The pinned-mode verdict for a change and its 95% interval, all in
    percent. An interval wholly above the limit fails at any width; any other
    is decided only at a half-width of at most half the limit."""
    if low > limit_percent:
        return "fail"
    if (high - low) / 2 > limit_percent / 2:
        return "unresolved"
    return "fail" if change > limit_percent else "pass"


def phase_gap(reports: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    """Per arm, what the CPU backend's phases leave of each frame's
    `submit`, in milliseconds and as a share of that submit: its p50, p99
    and maximum over every measured frame."""
    out = {}
    for side, runs in reports.items():
        gaps, shares = [], []
        for report in runs:
            for sample in report["samples"]:
                submit = sample["submit_seconds"]
                gap = submit - sum(phase(sample, name) for name in CPU_PHASES)
                gaps.append(gap * 1000)
                shares.append(gap / submit * 100)
        out[side] = {"ms": [quantile(gaps, 0.5), quantile(gaps, 0.99), max(gaps)],
                     "percent_of_submit": [quantile(shares, 0.5), quantile(shares, 0.99),
                                           max(shares)]}
    return out


def analyze(out: Path, limit_percent: float, budget_ms: float) -> dict[str, Any]:
    plan = json.loads((out / PLAN).read_text())
    control = json.loads((out / CONTROL).read_text())
    runs = [json.loads(line) for line in (out / RUNS).read_text().splitlines()]
    pinned = plan["clocks"] != "governor"
    result: dict[str, Any] = {"plan": plan, "completed": control["completed"], "scenes": {}}
    for scene in plan["scenes"]:
        name = scene["name"]
        rows = [r for r in runs if r["scene"] == name and not r["rejected"]]
        entry: dict[str, Any] = {
            "runs": len(rows),
            "rejected_runs": [{"run_id": r["run_id"], "other_ms_per_s": r["quiet"]["other_ms_per_s"],
                               "top": r["quiet"]["top"]}
                              for r in runs if r["scene"] == name and r["rejected"]],
            "other_ms_per_s": {r["run_id"]: r["quiet"]["other_ms_per_s"] for r in rows}}
        expected = 2 * plan["rounds"]
        invalid = [r["run_id"] for r in rows if r["failures"]]
        if invalid or len(rows) != expected:
            entry["verdict"] = "invalid"
            entry["invalid_runs"] = invalid
            entry["missing_runs"] = expected - len(rows)
            result["scenes"][name] = entry
            continue
        reports: dict[str, list[dict[str, Any]]] = {"base": [], "head": []}
        for row in sorted(rows, key=lambda r: r["pair"]):
            path = out / f"{row['run_id']}.json"
            if sha256(path) != row["report_sha256"]:
                raise SystemExit(f"{row['run_id']}.json changed after collection")
            report = json.loads(path.read_text())
            if len(report["samples"]) != plan["frames"] or report["warmup_frames"] != plan["warmup"]:
                raise SystemExit(f"{row['run_id']}: frame counts differ from the plan")
            reports[row["side"]].append(report)
        entry["metrics"] = {}
        for metric in plan["metrics"]:
            unit, value = METRICS[metric]
            frames = {side: [[value(s) for s in report["samples"]] for report in side_reports]
                      for side, side_reports in reports.items()}
            per_invocation = {side: {"p50": [quantile(run, 0.5) for run in runs_],
                                     "p99": [quantile(run, 0.99) for run in runs_]}
                              for side, runs_ in frames.items()}
            judged = {"unit": unit, "per_invocation": per_invocation,
                      "worst_p99": {side: max(v["p99"]) for side, v in per_invocation.items()}}
            if pinned:
                change = median_change(frames)
                low, high = change_interval(frames, random.Random(f"{name}/{metric}"))
                p99_low, p99_high = change_interval(
                    frames, random.Random(f"{name}/{metric}/p99"), 0.99)
                judged |= {"change_percent": change, "interval_percent": [low, high],
                           "p99_change_percent": median_change(frames, 0.99),
                           "p99_interval_percent": [p99_low, p99_high],
                           "verdict": decide(change, low, high, limit_percent)}
            else:
                judged |= {"budget_ms": budget_ms,
                           "median_p99": {side: statistics.median(v["p99"])
                                          for side, v in per_invocation.items()},
                           "verdict": "within budget" if judged["worst_p99"]["head"] <= budget_ms
                           else "over budget"}
            entry["metrics"][metric] = judged
        entry["verdict"] = entry["metrics"][plan["metrics"][0]]["verdict"]
        if plan["engine"] == "cherenkov-cpu":
            entry["phase_gap"] = phase_gap(reports)
        result["scenes"][name] = entry
    return result


def quiet_limit(collections: list[Path]) -> dict[str, Any]:
    """The `--quiet-limit` the runs of `collections` support: the median of
    their other-process CPU per second plus `QUIET_SIGMAS` robust standard
    deviations (1.4826 × the median absolute deviation), so a run of the
    background these runs saw passes and a burst beyond it does not. Each
    run is re-read from its raw snapshots."""
    rates = {}
    for out in collections:
        plan = json.loads((out / PLAN).read_text())
        for line in (out / RUNS).read_text().splitlines():
            row = json.loads(line)
            if row["failures"]:
                continue
            rates[f"{out.name}/{row['run_id']}"] = outside_cpu(
                out / f"{row['run_id']}-control", plan["tick_hz"])["other_ms_per_s"]
    median = statistics.median(rates.values())
    sigma = 1.4826 * statistics.median(abs(r - median) for r in rates.values())
    return {"runs": rates, "median_ms_per_s": median, "robust_sigma_ms_per_s": sigma,
            "limit_ms_per_s": median + QUIET_SIGMAS * sigma}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    run.add_argument("--base", type=Path, required=True)
    run.add_argument("--head", type=Path, required=True)
    run.add_argument("--scene", type=Path, action="append", required=True)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument("--serial", required=True)
    run.add_argument("--engine", choices=[GPU_ENGINE, *CPU_ENGINES], default=GPU_ENGINE)
    run.add_argument("--workers", type=int)
    run.add_argument("--cpus")
    run.add_argument("--cluster-khz", action="append", default=[],
                     type=lambda text: (text.split("=", 1)[0], int(text.split("=", 1)[1])),
                     metavar="POLICY=KHZ")
    run.add_argument("--devfreq-khz", action="append", default=[],
                     type=lambda text: (text.split("=", 1)[0], int(text.split("=", 1)[1])),
                     metavar="NAME=KHZ")
    run.add_argument("--governor", action="store_true")
    run.add_argument("--gpu-counters", action="store_true")
    run.add_argument("--gpu-khz", type=int, default=649000)
    run.add_argument("--cpu-khz", type=int, default=2294000)
    run.add_argument("--rounds", type=int, default=4)
    run.add_argument("--frames", type=int, default=120)
    run.add_argument("--warmup", type=int, default=10)
    run.add_argument("--width", type=int, default=1280)
    run.add_argument("--run-seconds", type=int, default=300)
    run.add_argument("--battery-c", type=float, default=45.0)
    run.add_argument("--quiet-limit", type=float, metavar="MS_PER_S")
    run.add_argument("--quiet-attempts", type=int, default=3)
    limit = commands.add_parser("quiet-limit")
    limit.add_argument("collections", type=Path, nargs="+")
    report = commands.add_parser("analyze")
    report.add_argument("out", type=Path)
    report.add_argument("--limit-percent", type=float, default=2.0)
    report.add_argument("--budget-ms", type=float, default=1000 / RATE_HZ)
    args = parser.parse_args()
    if args.command == "run":
        if args.governor and args.gpu_counters:
            parser.error("--gpu-counters measures cost at pinned clocks; --governor measures "
                         "the frame budget")
        engine = CPU_ENGINES.get(args.engine)
        if engine is None:
            if args.workers is not None or args.cpus is not None or args.cluster_khz:
                parser.error("--workers, --cpus and --cluster-khz select a CPU engine's cells")
        else:
            if args.governor or args.gpu_counters:
                parser.error("a CPU engine's worker count selects its mode, without GPU counters")
            if args.workers is None or args.cpus is None:
                parser.error("a CPU engine needs --workers and --cpus")
            if args.workers < 1 or (engine.max_workers is not None
                                    and args.workers > engine.max_workers):
                parser.error(f"{args.engine} runs at 1 to {engine.max_workers or 'any'} workers")
            if cpu_cell(args.workers) == "comparison":
                if not args.cluster_khz:
                    parser.error("one worker compares cost at held clocks: name them with "
                                 "--cluster-khz")
            elif args.cluster_khz or args.devfreq_khz:
                parser.error("more workers check the frame budget under the default governors; "
                             "--cluster-khz and --devfreq-khz hold clocks")
            if len(dict(args.cluster_khz)) != len(args.cluster_khz):
                parser.error("--cluster-khz names a policy twice")
        if args.governor and args.devfreq_khz:
            parser.error("--devfreq-khz holds clocks; --governor leaves them to their governors")
        if (len(dict(args.devfreq_khz)) != len(args.devfreq_khz)
                or not set(dict(args.devfreq_khz)) <= set(INTERCONNECT)):
            parser.error(f"--devfreq-khz names each of {', '.join(INTERCONNECT)} at most once")
        if args.rounds < 2 or args.rounds % 2:
            parser.error("--rounds must be even, so each arm runs first equally often")
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(124))
        if shutil.which("adb") is None:
            parser.error("adb is not on PATH")
        Session(args).run()
    elif args.command == "quiet-limit":
        derived = quiet_limit(args.collections)
        for run_id, rate in sorted(derived["runs"].items(), key=lambda item: item[1]):
            print(f"{run_id}: {rate:.1f} ms/s")
        print(f"median {derived['median_ms_per_s']:.1f} ms/s, robust sigma "
              f"{derived['robust_sigma_ms_per_s']:.1f} ms/s: --quiet-limit "
              f"{derived['limit_ms_per_s']:.0f}")
    else:
        result = analyze(args.out, args.limit_percent, args.budget_ms)
        save(args.out / "analysis.json", result)
        for name, entry in result["scenes"].items():
            for rejected in entry["rejected_runs"]:
                print(f"{name}: {rejected['run_id']} rejected and rerun, other processes "
                      f"{rejected['other_ms_per_s']:.1f} ms/s")
            if "metrics" not in entry:
                print(f"{name}: {entry['verdict']}")
                continue
            for metric, judged in entry["metrics"].items():
                if "budget_ms" in judged:
                    print(f"{name} {metric}: {judged['verdict']} (budget "
                          f"{judged['budget_ms']:.2f} ms; invocation p99 worst / median: " +
                          ", ".join(f"{side} {judged['worst_p99'][side]:.3f} / "
                                    f"{judged['median_p99'][side]:.3f} ms"
                                    for side in ("base", "head")) + ")")
                    continue
                low, high = judged["interval_percent"]
                p99_low, p99_high = judged["p99_interval_percent"]
                print(f"{name} {metric}: {judged['verdict']} ({judged['change_percent']:+.3f}%, "
                      f"95% interval {low:+.3f}% to {high:+.3f}%; p99 "
                      f"{judged['p99_change_percent']:+.3f}%, {p99_low:+.3f}% to {p99_high:+.3f}%)")
            if "phase_gap" in entry:
                for side, gap in entry["phase_gap"].items():
                    print(f"{name} {side} submit outside the phases: p50 {gap['ms'][0]:.3f} ms "
                          f"({gap['percent_of_submit'][0]:.2f}%), p99 {gap['ms'][1]:.3f} ms "
                          f"({gap['percent_of_submit'][1]:.2f}%), max {gap['ms'][2]:.3f} ms")


if __name__ == "__main__":
    main()

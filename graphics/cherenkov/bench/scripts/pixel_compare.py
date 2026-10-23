#!/usr/bin/env python3
"""Interleaved A/B timing of two `cherenkov-bench` Android builds on a rooted
Pixel-class device.

  pixel_compare.py run     --base B --head H --scene DIR... --out O
                           [--governor | --gpu-counters]
  pixel_compare.py run     --base B --head H --scene DIR... --out O
                           --engine E --workers N --cluster-khz POLICY=KHZ...
  pixel_compare.py analyze O

`run` pushes both binaries and the scene directories, then measures every
scene in rounds of base/head pairs: `base head` then `head base`, alternating,
so neither arm always runs first. Each GPU-engine invocation is one `measure`
process: `--rate 120 --energy --cpu 7`, the warmup and measured frame counts
given, and `--native W×H` (width `--width`, the scene's own aspect) for scenes
under a `perf/` directory. Every completed report is kept; nothing is retried
or dropped.

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
`skia-cpu`) measures a CPU rasterizer instead: `measure --engine E` with the
engine at `--workers` workers, each scene at its own size, and no pacing or
energy window, so frames run back to back and measure cost. No bench thread
is placed: a phone's big.LITTLE scheduler moves threads where it will, and
pinning or confining them measures a device no user has, so no CPU set or
affinity is applied. The worker count reaches the engine through its own
control: cherenkov-cpu's raster pool, and rayon's global pool beside it,
through `RAYON_NUM_THREADS`; vello-cpu's dispatcher through
`VELLO_CPU_THREADS` (one worker is its single-threaded dispatcher, N
its multithreaded one with N workers); skia-cpu is single-threaded and runs
at one worker only. The GPU is left to its governor. Every CPU frequency
policy on the device is held at its `--cluster-khz` clock for the workload
instead, so thermal throttling stays out of the run; the option must name
every policy. Original policy values are restored after each invocation.

Controls, checked for every invocation and recorded with it:

- before: screen off, physical sensors below their limits, and the requested
  device settings applied with readback evidence;
- during: physical temperature supervision, GPU clock residency, the GPU
  engine's measuring-thread placement, and a CPU engine's worker count;
- after: pinned policy ranges and CPU frequency residency over the workload,
  followed by restoration of every original property and sysfs value.

A failed control marks the invocation invalid and stops the collection:
`analyze` reports a scene with an invalid invocation as `invalid`, never as
a pass, and the raw report stays on disk.

`analyze` reads a collection and, per scene and per metric of the plan, takes
each invocation's p50 of the metric over its measured frames, the median of
those per arm, and their change. The metrics are GPU milliseconds (or
shader-core active cycles with `--gpu-counters`) for the GPU engine; for a
CPU engine the frame (encode and submit together, in milliseconds: an engine
may rasterize in either, as vello-cpu dispatches strips while it records),
and for cherenkov-cpu also its raster pass (`shade`, the backend's phase).

For the GPU engine, a p50 over a finite frame count has a sampling error, and
a fresh process can land at a different cost, so the change alone does not
say whether the collection can tell a 2% change from none. `analyze`
therefore puts a 95% interval around the change by a hierarchical bootstrap
(invocations, then the frames of each). A metric whose whole interval lies
above `--limit-percent` (2%) `fails` whatever the interval's width: the
collection has shown the change exceeds the limit at that confidence. Any
other metric is decided only when its interval is no wider than the limit
(half-width at most half of `--limit-percent`), so a true change of zero
reads above the limit with probability far below a percent; it then `fails`
above the limit and passes otherwise. A wider interval is `unresolved`,
never a pass. The limit bounds a cost increase, so an interval wholly below
the negative limit — the head measurably cheaper — is no failure, and like
any other it passes only at that width and is `unresolved` beyond it. The
scene's verdict is its first metric's (GPU time or cycles). Each metric also
carries the change of the median invocation p99 and its interval, which
decide nothing. More `--frames` shrink the sampling error of each p50 and
more `--rounds` average over more processes; full-size scenes resolve on GPU
time at the defaults, and scenes of a fraction of a millisecond on
shader-core cycles. In governor mode `analyze` reports each arm's worst
invocation GPU p99 against `--budget-ms` and makes no cost decision.

A CPU engine's collection is never decided: the phone's big.LITTLE
scheduling makes CPU timing unstable and a CPU-side change is proven by the
deterministic Callgrind gate, so `analyze` reports, per scene and metric,
each arm's median invocation p50 and p99 with their change, for judging
whether there is an obvious regression.

For cherenkov-cpu `analyze` also reports, per arm, how much of `submit` the
backend's phases (`lower`, `glyph`, `shade`) leave unaccounted: the render
thread's message handoff and the frame's work outside the three phases.

`analyze` needs from a collection only `plan.json` (`scenes`, `rounds`,
`frames`, `warmup`, `metrics`, `engine`, `clocks`), `control.json`
(`completed`), `runs.jsonl` and the reports, so a collector for another
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
    metrics `analyze` reports, and its most workers."""
    environment: typing.Callable[[int], dict[str, str]]
    adapter: typing.Callable[[int], str]
    metrics: tuple[str, ...]
    max_workers: int | None


CPU_ENGINES = {
    "cherenkov-cpu": CpuEngine(lambda n: {"RAYON_NUM_THREADS": str(n)},
                               lambda n: f"cherenkov-cpu ({n} threads)", ("frame", "shade"), None),
    "vello-cpu": CpuEngine(lambda n: {"VELLO_CPU_THREADS": "0" if n == 1 else str(n)},
                           lambda n: "vello-cpu (single-threaded dispatcher)" if n == 1
                           else f"vello-cpu ({n} threads)", ("frame",), None),
    "skia-cpu": CpuEngine(lambda n: {}, lambda n: "Skia raster surface", ("frame",), 1),
}
GPU_ENGINE = "cherenkov"

# A collection's identity: every field `analyze` relies on is in the plan.
PLAN = "plan.json"
CONTROL = "control.json"
RUNS = "runs.jsonl"
RATE_HZ = 120
BOOTSTRAP_RESAMPLES = 2000
# The GPU engine's measuring CPU.
CPU = 7
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


class Clocks:
    """Frequency policies read and pinned through sysfs, by name: `gpu` is
    the Mali device behind `/sys/class/misc/mali0`, `policyN` a CPU
    frequency policy."""

    def __init__(self, device: Device, names: list[str]) -> None:
        self.device = device
        self.paths = {name: device.root("readlink -f /sys/class/misc/mali0/device").strip()
                      if name == "gpu" else f"{CPUFREQ}/{name}" for name in names}

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
        node = "available_frequencies" if name == "gpu" else "scaling_available_frequencies"
        return [int(v) for v in self.device.root(f"cat {self.paths[name]}/{node}").split()]

    def state(self) -> dict[str, dict[str, int]]:
        state = {}
        for name, path in self.paths.items():
            current = "cur_freq" if name == "gpu" else "scaling_cur_freq"
            values = self.device.root(
                f"cat {path}/scaling_min_freq {path}/scaling_max_freq {path}/{current}").split()
            state[name] = dict(zip(("min", "max", "current"), map(int, values)))
        return state



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
                if name != "gpu":
                    governors = device.root("cat " + shlex.quote(path + "/scaling_available_governors")).split()
                    if "performance" not in governors:
                        raise ControlError(f"{name} has no performance governor")
                    self.arguments += ["--node", path + "/scaling_governor", "performance"]
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
        if self.cpu_engine is None:
            self.cpus: list[int] | None = [CPU]
            self.pinned = None if args.governor else {"gpu": args.gpu_khz,
                                                      self.policy_of(CPU): args.cpu_khz}
        else:
            self.cpus = None
            self.pinned = dict(args.cluster_khz)
        self.clocks = Clocks(self.device, list(self.pinned or {"gpu": 0, self.policy_of(CPU): 0}))
        self.remote = f"/data/local/tmp/pixel-compare-{self.out.name}"
        self.performance = PerformanceControls(self.device, self.clocks, self.pinned, args.battery_c)
        self.device_runs: dict[str, dict[str, Any]] = {}

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
            if set(self.pinned) != set(policies):
                raise SystemExit(f"--cluster-khz must name every CPU frequency policy on "
                                 f"the device: {sorted(policies)}")
        if self.pinned:
            for name, khz in self.pinned.items():
                if khz not in self.clocks.available(name):
                    raise SystemExit(f"{name} {khz} kHz is not an available frequency")
        if self.cpu_engine is None:
            metrics = ["shader-core-cycles" if args.gpu_counters else "gpu-time"]
        else:
            metrics = list(self.cpu_engine.metrics)
        plan = {
            "arms": {side: {"path": str(path), "sha256": sha256(path)} for side, path in arms.items()},
            "scenes": scenes, "rounds": args.rounds, "frames": args.frames,
            "warmup": args.warmup, "width": args.width, "rate_hz": RATE_HZ,
            "engine": args.engine, "workers": args.workers, "cpus": self.cpus,
            "metrics": metrics, "policies": policies,
            "clocks": "governor" if self.pinned is None else self.pinned,
            "battery_c": args.battery_c, "serial": args.serial,
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
                   "--warmup", str(args.warmup), "--frames", str(args.frames), "--out", report]
        environment: dict[str, str] = {}
        if self.cpu_engine is None:
            command += ["--cpu", str(CPU), "--rate", str(RATE_HZ), "--energy"]
            if args.gpu_counters:
                command.append("--gpu-counters")
            if scene["perf"]:
                spec = json.loads((Path(scene["path"]) / "scene.json").read_text())
                height = round(args.width * spec["height"] / spec["width"])
                command += ["--native", f"{args.width}x{height}"]
        else:
            environment = self.cpu_engine.environment(args.workers)
        prefix = ["env", *(f"{k}={v}" for k, v in environment.items())] if environment else []
        remote_control = f"{self.remote}/{run_id}-control"
        supervised = ["sh", self.remote + "/pixel_run.sh", "--out", remote_control,
                      "--seconds", str(args.run_seconds), *self.performance.arguments,
                      "--", *prefix, *command]
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
        if self.cpu_engine is not None:
            adapter = self.cpu_engine.adapter(self.args.workers)
            if report["device"]["adapter"] != adapter:
                failures.append(f"the engine reports {report['device']['adapter']!r}, "
                                f"not {adapter!r}")
            return failures
        if not (report["placement"]["controlled"] and report["placement"]["requested"] == self.cpus):
            failures.append("placement not pinned")
        if any(s["cpu_start"] not in self.cpus or s["cpu_end"] not in self.cpus
               for s in report["samples"]):
            failures.append(f"measuring thread left CPUs {self.cpus}")
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
                        self.invocation(scene, pair, side)
            control["completed"] = True
        finally:
            self.device.root(f"rm -r {self.remote}")
            control["restored"] = self.clocks.state()
            save(self.out / CONTROL, control)

    def invocation(self, scene: dict[str, Any], pair: int, side: str) -> None:
        """Measures one invocation between its controls and logs it; a
        failed control stops the collection."""
        run_id = f"{scene['name']}-p{pair}-{side}"
        before = self.snapshot()
        local = self.measure(scene, side, run_id)
        directory = self.out / f"{run_id}-control"
        residency = {}
        ranges = {}
        for name in self.pinned or {}:
            values = [int(v) for v in (directory / f"{name}-after-range").read_text().split()]
            ranges[name] = dict(zip(("min", "max"), values))
            if name != "gpu":
                readings = [{int(k): int(v) for k, v in
                             (line.split() for line in (directory / f"{name}-{moment}-residency").read_text().splitlines())}
                            for moment in ("before", "after")]
                residency[name] = {khz: ticks - readings[0][khz] for khz, ticks in readings[1].items()}
        after = {"thermal": thermal(self.device), "clocks": ranges,
                 "restored_clocks": self.clocks.state(),
                 "cpu_residency_ticks": {name: {k: v for k, v in table.items() if v}
                                         for name, table in residency.items()}}
        failures = self.check_report(local)
        if self.pinned:
            # What a pinned run controls is the clocks: a cooling device
            # that moved none leaves the run valid. Each range is checked
            # after the run, and each CPU policy's residency over the whole
            # process.
            for name, khz in self.pinned.items():
                state = after["clocks"][name]
                if state["min"] != khz or state["max"] != khz:
                    failures.append(f"{name} policy after {state}")
                other = {k: v for k, v in residency.get(name, {}).items() if k != khz and v}
                if other:
                    failures.append(f"{name} left {khz} kHz: {other}")
        record = {"scene": scene["name"], "pair": pair, "side": side, "run_id": run_id,
                  "report_sha256": sha256(local), "before": before, "after": after,
                  "failures": failures}
        with (self.out / RUNS).open("a") as stream:
            stream.write(json.dumps(record) + "\n")
        print("RUN", run_id, "invalid: " + "; ".join(failures) if failures else "valid",
              flush=True)
        if failures:
            raise ControlError(f"{run_id}: {'; '.join(failures)}")



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
    cpu_engine = plan["engine"] in CPU_ENGINES
    pinned = plan["clocks"] != "governor"
    result: dict[str, Any] = {"plan": plan, "completed": control["completed"], "scenes": {}}
    for scene in plan["scenes"]:
        name = scene["name"]
        rows = [r for r in runs if r["scene"] == name]
        entry: dict[str, Any] = {"runs": len(rows)}
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
                      "change_percent": median_change(frames),
                      "p99_change_percent": median_change(frames, 0.99)}
            if cpu_engine:
                judged["p50"] = {side: statistics.median(invocation["p50"])
                                 for side, invocation in per_invocation.items()}
                judged["p99"] = {side: statistics.median(invocation["p99"])
                                 for side, invocation in per_invocation.items()}
            else:
                change = judged["change_percent"]
                low, high = change_interval(frames, random.Random(f"{name}/{metric}"))
                p99_low, p99_high = change_interval(frames, random.Random(f"{name}/{metric}/p99"),
                                                    0.99)
                judged["interval_percent"] = [low, high]
                judged["p99_interval_percent"] = [p99_low, p99_high]
                judged["worst_p99"] = {side: max(v["p99"])
                                       for side, v in per_invocation.items()}
                if pinned:
                    judged["verdict"] = decide(change, low, high, limit_percent)
                else:
                    judged["verdict"] = "within budget" \
                        if judged["worst_p99"]["head"] <= budget_ms else "over budget"
            entry["metrics"][metric] = judged
        if not cpu_engine:
            entry["verdict"] = entry["metrics"][plan["metrics"][0]]["verdict"]
        if plan["engine"] == "cherenkov-cpu":
            entry["phase_gap"] = phase_gap(reports)
        result["scenes"][name] = entry
    return result


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
    run.add_argument("--cluster-khz", action="append", default=[],
                     type=lambda text: (text.split("=", 1)[0], int(text.split("=", 1)[1])),
                     metavar="POLICY=KHZ")
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
            if args.workers is not None or args.cluster_khz:
                parser.error("--workers and --cluster-khz are a CPU engine's controls")
        else:
            if args.governor or args.gpu_counters:
                parser.error("a CPU engine is measured at pinned CPU clocks, without GPU counters")
            if args.workers is None or not args.cluster_khz:
                parser.error("a CPU engine needs --workers and --cluster-khz")
            if args.workers < 1 or (engine.max_workers is not None
                                    and args.workers > engine.max_workers):
                parser.error(f"{args.engine} runs at 1 to {engine.max_workers or 'any'} workers")
            if len(dict(args.cluster_khz)) != len(args.cluster_khz):
                parser.error("--cluster-khz names a policy twice")
        if args.rounds < 2 or args.rounds % 2:
            parser.error("--rounds must be even, so each arm runs first equally often")
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(124))
        if shutil.which("adb") is None:
            parser.error("adb is not on PATH")
        Session(args).run()
    else:
        result = analyze(args.out, args.limit_percent, args.budget_ms)
        save(args.out / "analysis.json", result)
        cpu_engine = result["plan"]["engine"] in CPU_ENGINES
        for name, entry in result["scenes"].items():
            if "metrics" not in entry:
                print(f"{name}: {entry['verdict']}")
                continue
            for metric, judged in entry["metrics"].items():
                if cpu_engine:
                    print(f"{name} {metric}: base p50 {judged['p50']['base']:.3f} "
                          f"{judged['unit']} p99 {judged['p99']['base']:.3f}; head p50 "
                          f"{judged['p50']['head']:.3f} {judged['unit']} p99 "
                          f"{judged['p99']['head']:.3f}; {judged['change_percent']:+.3f}%, "
                          f"p99 {judged['p99_change_percent']:+.3f}%")
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

#!/usr/bin/env python3
"""Interleaved A/B timing of two `cherenkov-bench` Android builds on a rooted
Pixel-class device.

  pixel_compare.py run     --base B --head H --scene DIR... --out O [--governor]
  pixel_compare.py analyze O

`run` pushes both binaries and the scene directories, then measures every
scene in rounds of base/head pairs: `base head` then `head base`, alternating,
so neither arm always runs first. Each invocation is one `measure` process:
`--rate 120 --energy --cpu 7`, the warmup and measured frame counts given, and
`--native W×H` (width `--width`, the scene's own aspect) for scenes under a
`perf/` directory. Every completed report is kept; nothing is retried or
dropped.

A GPU's busy time at a given workload scales with its clock, and the default
GPU governor picks the clock from recent utilization: a fresh process, a
120 Hz pacing gap or a short scene leaves it at whatever operating point the
previous seconds settled on. Identical binaries then differ by the clocks
they happened to get, not by their cost (#2613). So `run` measures code cost
with both the GPU and the measuring CPU held at one operating point:
`scaling_min_freq = scaling_max_freq = --gpu-khz / --cpu-khz` for the whole
collection, the original limits restored afterwards. A thermal cap is never
overridden: when a requested clock lies outside the effective policy range,
the run waits for the device to cool and fails if it does not. `--governor`
leaves both clocks to their governors instead; that mode measures what the
device delivers (the 8.33 ms p99 frame budget), not cost.

Controls, checked for every invocation and recorded with it:

- before: screen off; thermal status below moderate; battery below
  `--battery-c`; every CPU and GPU cooling device at state 0; in pinned mode
  both policy ranges collapsed onto the requested clocks;
- during: the report's `gpu_clock` residency over the measured window (in
  pinned mode all of its powered time at `--gpu-khz`); the measuring thread
  on the pinned CPU without migrating;
- after: thermal status below moderate; in pinned mode both policy ranges
  still on the requested clocks and the measuring CPU's `time_in_state`
  residency over the whole process at `--cpu-khz` alone; under the governor
  every cooling device still at 0, since a cooling device narrows the range
  the governor picks from. A pinned run is not invalidated by a cooling
  device that moved neither clock.

A failed control marks the invocation invalid and stops the collection:
`analyze` reports a scene with an invalid invocation as `invalid`, never as
a pass, and the raw report stays on disk.

`analyze` reads a collection and, per scene, takes each invocation's GPU p50
over its measured frames, the median of those per arm, and their change. A
p50 over a finite frame count has a sampling error, and a fresh process can
land at a different cost, so the change alone does not say whether the
collection can tell a 2% change from none: a scene whose frame times spread
widely around a short GPU time needs far more frames than one whose frames
agree to a fraction of a percent. `analyze` therefore puts a 95% interval
around the change by a hierarchical bootstrap (invocations, then the frames
of each) and decides only a scene whose interval is no wider than the limit
(half-width at most half of `--limit-percent`): a true change of zero then
reads above the limit with probability far below a percent. Such a scene
`fails` above `--limit-percent` (2%) and passes otherwise; a wider interval
is `unresolved`, never a pass, and the scene is collected again with more
`--frames` or `--rounds`. In governor mode `analyze` reports each arm's
worst invocation GPU p99 against `--budget-ms` and makes no cost decision.
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
from pathlib import Path
from typing import Any

# A collection's identity: every field `analyze` relies on is in the plan.
PLAN = "plan.json"
CONTROL = "control.json"
RUNS = "runs.jsonl"
RATE_HZ = 120
BOOTSTRAP_RESAMPLES = 2000
CPU = 7
# Thermal status from `dumpsys thermalservice`: 2 is MODERATE.
THERMAL_STATUS_BELOW = 2
# CoolingDevice types of `android.os.CoolingDevice`: CPU and GPU.
COOLING_TYPES = (2, 3)


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
    """The GPU and measuring-CPU frequency policies, read and pinned through
    sysfs. The GPU is the Mali device behind `/sys/class/misc/mali0`."""

    def __init__(self, device: Device) -> None:
        self.device = device
        gpu = device.root("readlink -f /sys/class/misc/mali0/device").strip()
        self.paths = {"gpu": gpu, "cpu": f"/sys/devices/system/cpu/cpu{CPU}/cpufreq"}

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

    def cpu_residency(self) -> dict[int, int]:
        """The measuring CPU's cumulative time per clock (kHz to 10 ms
        ticks, cpufreq `stats/time_in_state`)."""
        text = self.device.root(f"cat {self.paths['cpu']}/stats/time_in_state")
        return {int(khz): int(ticks) for khz, ticks in (line.split() for line in text.splitlines())}

    def write(self, name: str, low: int, high: int) -> None:
        path = self.paths[name]
        # Raise the ceiling before the floor when moving up and the floor
        # before the ceiling when moving down, so min <= max holds at every
        # step; writing max, min, max covers both.
        self.device.root(f"echo {high} > {path}/scaling_max_freq; "
                         f"echo {low} > {path}/scaling_min_freq; "
                         f"echo {high} > {path}/scaling_max_freq")


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


class Session:
    """One collection: the plan, the device, the controls and the run log."""

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.out: Path = args.out
        self.device = Device(args.serial)
        self.clocks = Clocks(self.device)
        self.pinned = None if args.governor else {"gpu": args.gpu_khz, "cpu": args.cpu_khz}
        self.remote = f"/data/local/tmp/pixel-compare-{self.out.name}"

    def quiet(self, pinned: bool) -> dict[str, Any]:
        """Waits, bounded, until the device is cool and — when `pinned` —
        both policies sit on the requested clocks. Unpinned, a requested
        clock must lie inside the effective range, which a thermal cap
        narrows; this never lifts a cap."""
        deadline = time.clock_gettime(time.CLOCK_MONOTONIC) + self.args.cool_seconds
        while True:
            wakefulness = screen_off(self.device)
            sample = thermal(self.device)
            clocks = self.clocks.state()
            reasons = []
            if wakefulness == "Awake":
                reasons.append("screen awake")
            if sample["status"] >= THERMAL_STATUS_BELOW:
                reasons.append(f"thermal status {sample['status']}")
            if sample["battery_c"] >= self.args.battery_c:
                reasons.append(f"battery {sample['battery_c']} C")
            hot = {k: v for k, v in sample["cooling"].items() if v != 0}
            if hot:
                reasons.append(f"cooling {hot}")
            for name, khz in (self.pinned or {}).items():
                low, high = clocks[name]["min"], clocks[name]["max"]
                if pinned and (low, high) != (khz, khz):
                    reasons.append(f"{name} policy {low}-{high} not pinned at {khz}")
                if not pinned and not low <= khz <= high:
                    reasons.append(f"{name} {khz} outside the effective range {low}-{high}")
            if not reasons:
                return {"wakefulness": wakefulness, "thermal": sample, "clocks": clocks}
            if time.clock_gettime(time.CLOCK_MONOTONIC) > deadline:
                raise ControlError("device not quiet within "
                                   f"{self.args.cool_seconds} s: {'; '.join(reasons)}")
            print("waiting:", "; ".join(reasons), flush=True)
            time.sleep(5)

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
        if self.pinned:
            for name, khz in self.pinned.items():
                if khz not in self.clocks.available(name):
                    raise SystemExit(f"{name} {khz} kHz is not an available frequency")
        plan = {
            "arms": {side: {"path": str(path), "sha256": sha256(path)} for side, path in arms.items()},
            "scenes": scenes, "rounds": args.rounds, "frames": args.frames,
            "warmup": args.warmup, "width": args.width, "rate_hz": RATE_HZ, "cpu": CPU,
            "clocks": "governor" if args.governor else {"gpu_khz": args.gpu_khz,
                                                         "cpu_khz": args.cpu_khz},
            "battery_c": args.battery_c, "serial": args.serial,
            "script_sha256": sha256(Path(__file__)),
        }
        save(self.out / PLAN, plan)
        if self.device.root("id -u").strip() != "0":
            raise SystemExit("the device's su does not grant root")
        return scenes

    def push(self, scenes: list[dict[str, Any]]) -> None:
        """Pushes both arms and every scene into the collection's remote
        directory, which the shell user owns so `adb push` can write it."""
        plan = json.loads((self.out / PLAN).read_text())
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
        command = [f"{self.remote}/bench-{side}", "measure", "--engine", "cherenkov",
                   "--scene", f"{self.remote}/scenes/{scene['name']}",
                   "--rate", str(RATE_HZ), "--energy", "--cpu", str(CPU),
                   "--warmup", str(args.warmup), "--frames", str(args.frames), "--out", report]
        if scene["perf"]:
            spec = json.loads((Path(scene["path"]) / "scene.json").read_text())
            height = round(args.width * spec["height"] / spec["width"])
            command += ["--native", f"{args.width}x{height}"]
        log = self.device.root("timeout 300 " + shlex.join(command), timeout=330)
        (self.out / f"{run_id}.log").write_text(log)
        local = self.out / f"{run_id}.json"
        self.device.adb("pull", report, str(local), timeout=60)
        return local

    def check_report(self, local: Path) -> list[str]:
        """The in-window controls a report carries."""
        report = json.loads(local.read_text())
        failures = []
        if report["conditions"]["screen_state"] not in ("off", "dozing", "asleep"):
            failures.append(f"screen {report['conditions']['screen_state']}")
        if not (report["placement"]["controlled"] and report["placement"]["requested"] == [CPU]):
            failures.append("placement not pinned")
        if any(s["migrated"] or s["cpu_start"] != CPU or s["cpu_end"] != CPU
               for s in report["samples"]):
            failures.append("measuring thread left the pinned CPU")
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
        # The limits to restore are read from a cool device: a thermal cap
        # reads as a lowered `scaling_max_freq`, and writing that back would
        # leave the cap in place as a user limit.
        control: dict[str, Any] = {"completed": False}
        control["original_cooled"] = self.quiet(pinned=False)
        original = control["original_cooled"]["clocks"]
        control["original"] = original
        save(self.out / CONTROL, control)
        self.device.adb("shell", f"mkdir {self.remote} {self.remote}/scenes")
        try:
            self.push(scenes)
            if self.pinned:
                for name, khz in self.pinned.items():
                    self.clocks.write(name, khz, khz)
                control["pinned"] = self.clocks.state()
                save(self.out / CONTROL, control)
            for scene in scenes:
                for pair in range(self.args.rounds):
                    order = ("base", "head") if pair % 2 == 0 else ("head", "base")
                    for side in order:
                        run_id = f"{scene['name']}-p{pair}-{side}"
                        before = self.quiet(pinned=self.pinned is not None)
                        cpu_before = self.clocks.cpu_residency()
                        local = self.measure(scene, side, run_id)
                        cpu_ticks = {khz: ticks - cpu_before[khz] for khz, ticks
                                     in self.clocks.cpu_residency().items()}
                        after = {"thermal": thermal(self.device), "clocks": self.clocks.state(),
                                 "cpu_residency_ticks": {k: v for k, v in cpu_ticks.items() if v}}
                        failures = self.check_report(local)
                        if after["thermal"]["status"] >= THERMAL_STATUS_BELOW:
                            failures.append(f"thermal status {after['thermal']['status']} after")
                        if self.pinned:
                            # What a pinned run controls is the clocks: a
                            # cooling device that moved neither leaves the run
                            # valid. Its range is checked after the run, and
                            # the CPU's residency over the whole process.
                            for name, khz in self.pinned.items():
                                state = after["clocks"][name]
                                if state["min"] != khz or state["max"] != khz:
                                    failures.append(f"{name} policy after {state}")
                            other = {k: v for k, v in cpu_ticks.items()
                                     if k != self.pinned["cpu"] and v}
                            if other:
                                failures.append(f"CPU left {self.pinned['cpu']} kHz: {other}")
                        else:
                            # Under the governor a cooling device narrows the
                            # range the governor picks from.
                            hot = {k: v for k, v in after["thermal"]["cooling"].items() if v}
                            if hot:
                                failures.append(f"cooling after {hot}")
                        record = {"scene": scene["name"], "pair": pair, "side": side,
                                  "run_id": run_id, "report_sha256": sha256(local),
                                  "before": before, "after": after, "failures": failures}
                        with (self.out / RUNS).open("a") as stream:
                            stream.write(json.dumps(record) + "\n")
                        print("RUN", run_id, "invalid: " + "; ".join(failures) if failures
                              else "valid", flush=True)
                        if failures:
                            raise ControlError(f"{run_id}: {'; '.join(failures)}")
            control["completed"] = True
        finally:
            self.device.root(f"rm -r {self.remote}")
            if self.pinned:
                self.restore(original, control)

    def restore(self, original: dict[str, Any], control: dict[str, Any]) -> None:
        """Writes the original policy limits back, then verifies them once
        the device has cooled: a thermal cap reads as a lower
        `scaling_max_freq` until its cooling device returns to 0."""
        for name in self.clocks.paths:
            self.clocks.write(name, original[name]["min"], original[name]["max"])
        self.pinned = None
        control["restored_cooled"] = self.quiet(pinned=False)
        control["restored"] = self.clocks.state()
        save(self.out / CONTROL, control)
        for name in self.clocks.paths:
            for bound in ("min", "max"):
                if control["restored"][name][bound] != original[name][bound]:
                    raise RuntimeError(f"{name} {bound} not restored: {control['restored']}")


def quantile(values: list[float], q: float) -> float:
    """Nearest-rank quantile, the bench's own definition."""
    ordered = sorted(values)
    return ordered[math.ceil(q * len(ordered)) - 1]


def median_change(arms: dict[str, list[list[float]]]) -> float:
    """The change of the median invocation GPU p50, head against base, in
    percent."""
    medians = {side: statistics.median(quantile(run, 0.5) for run in runs)
               for side, runs in arms.items()}
    return (medians["head"] / medians["base"] - 1) * 100


def change_interval(arms: dict[str, list[list[float]]],
                    rng: random.Random) -> tuple[float, float]:
    """The 95% interval of `median_change` by a hierarchical bootstrap:
    each resample draws an arm's invocations with replacement, then each
    drawn invocation's frames with replacement, so both the spread between
    processes and the sampling error of a p50 over a finite frame count
    widen it."""
    changes = sorted(
        median_change({side: [rng.choices(run, k=len(run))
                              for run in rng.choices(runs, k=len(runs))]
                       for side, runs in arms.items()})
        for _ in range(BOOTSTRAP_RESAMPLES))
    tail = round(BOOTSTRAP_RESAMPLES * 0.025)
    return changes[tail], changes[-tail - 1]


def analyze(out: Path, limit_percent: float, budget_ms: float) -> dict[str, Any]:
    plan = json.loads((out / PLAN).read_text())
    control = json.loads((out / CONTROL).read_text())
    runs = [json.loads(line) for line in (out / RUNS).read_text().splitlines()]
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
        arms: dict[str, dict[str, list[float]]] = {"base": {"p50": [], "p99": []},
                                                    "head": {"p50": [], "p99": []}}
        frames: dict[str, list[list[float]]] = {"base": [], "head": []}
        for row in sorted(rows, key=lambda r: r["pair"]):
            report = json.loads((out / f"{row['run_id']}.json").read_text())
            if sha256(out / f"{row['run_id']}.json") != row["report_sha256"]:
                raise SystemExit(f"{row['run_id']}.json changed after collection")
            gpu = [s["gpu_seconds"] * 1000 for s in report["samples"]]
            if len(gpu) != plan["frames"] or report["warmup_frames"] != plan["warmup"]:
                raise SystemExit(f"{row['run_id']}: frame counts differ from the plan")
            frames[row["side"]].append(gpu)
            arms[row["side"]]["p50"].append(quantile(gpu, 0.5))
            arms[row["side"]]["p99"].append(quantile(gpu, 0.99))
        change = median_change(frames)
        low, high = change_interval(frames, random.Random(name))
        entry.update({"gpu_p50_ms": arms, "change_percent": change,
                      "interval_percent": [low, high],
                      "worst_p99_ms": {side: max(v["p99"]) for side, v in arms.items()}})
        if pinned:
            if (high - low) / 2 > limit_percent / 2:
                entry["verdict"] = "unresolved"
            else:
                entry["verdict"] = "fail" if change > limit_percent else "pass"
        else:
            entry["verdict"] = "within budget" if entry["worst_p99_ms"]["head"] <= budget_ms \
                else "over budget"
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
    run.add_argument("--governor", action="store_true")
    run.add_argument("--gpu-khz", type=int, default=649000)
    run.add_argument("--cpu-khz", type=int, default=2294000)
    run.add_argument("--rounds", type=int, default=4)
    run.add_argument("--frames", type=int, default=120)
    run.add_argument("--warmup", type=int, default=10)
    run.add_argument("--width", type=int, default=1280)
    run.add_argument("--battery-c", type=float, default=45.0)
    run.add_argument("--cool-seconds", type=float, default=600.0)
    report = commands.add_parser("analyze")
    report.add_argument("out", type=Path)
    report.add_argument("--limit-percent", type=float, default=2.0)
    report.add_argument("--budget-ms", type=float, default=1000 / RATE_HZ)
    args = parser.parse_args()
    if args.command == "run":
        if args.rounds < 2 or args.rounds % 2:
            parser.error("--rounds must be even, so each arm runs first equally often")
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(124))
        if shutil.which("adb") is None:
            parser.error("adb is not on PATH")
        Session(args).run()
    else:
        result = analyze(args.out, args.limit_percent, args.budget_ms)
        save(args.out / "analysis.json", result)
        for name, entry in result["scenes"].items():
            change = entry.get("change_percent")
            if change is None:
                print(f"{name}: {entry['verdict']}")
            else:
                low, high = entry["interval_percent"]
                print(f"{name}: {entry['verdict']} ({change:+.3f}%, 95% interval "
                      f"{low:+.3f}% to {high:+.3f}%)")


if __name__ == "__main__":
    main()

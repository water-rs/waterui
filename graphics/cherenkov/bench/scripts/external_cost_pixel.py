"""Pixel 9 Pro driver for `cherenkov-bench external-cost` (#168).

Run only under `pixel-adb --run`: its device lock covers the whole
experiment (one adb channel, `su -c` for the root-only ODPM rails).

Matrix: per size×transfer cell the two paths run interleaved —
e,c,e,c then the reversed control c,e,c,e — so drift cancels. The
battery/thermal gate is checked before every measured window, with the
screen off the whole time (the bench draws offscreen).

A run is accepted only when the remote report exists, pulls cleanly
and `verify` matches the cell. The remote path is removed before the
run, so a previous report cannot be pulled. Success is that file, not
a substring of the bench's stdout.

Raw reports land in `--out-dir` (default /tmp/external-cost-pixel),
never in git.
"""

import argparse
import datetime
import hashlib
import json
import pathlib
import re
import subprocess
import time
import uuid

from external_cost_matrix import (
    CELLS,
    FRAMES,
    ORDERS,
    RATE,
    WARMUP,
    args_for,
    order_name,
    verify,
)

BENCH = "/data/local/tmp/cherenkov-bench"
# The X4 prime core: `measure`'s documented pin for the Pixel.
CPU = "7"


def adb(*args):
    result = subprocess.run(
        ["adb", *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    if result.returncode:
        raise RuntimeError("adb {}: {}".format(args, result.stdout))
    return result.stdout


def shell(command):
    return adb("shell", command)


def thermal():
    service = shell("dumpsys thermalservice")
    status = int(re.search(r"Thermal Status:\s*(\d+)", service)[1])
    # The cached section may be hours old. The same dumpsys call also asks
    # the HAL for current readings, including the battery in degrees Celsius.
    current = service.split("Current temperatures from HAL:", 1)[1].split(
        "Current cooling devices from HAL:", 1
    )[0]
    battery = re.search(
        r"Temperature\{mValue=([\d.]+), mType=2, mName=battery,", current
    )
    if battery is None:
        raise RuntimeError("thermalservice returned no current battery temperature")
    return {"battery_c": float(battery[1]), "status": status}


def cool_enough(sample):
    return sample["battery_c"] < 45.0 and sample["status"] < 2


def cooled():
    shell("input keyevent KEYCODE_SLEEP")
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        sample = thermal()
        if cool_enough(sample):
            return sample
        print(
            "screen-off cooling: battery={} C, status={}".format(
                sample["battery_c"], sample["status"]
            ),
            flush=True,
        )
        time.sleep(5)
    raise RuntimeError("cooling timeout: battery must be below 45 C and status below 2")


def push(binary):
    subprocess.run(["adb", "push", binary, BENCH], check=True, capture_output=True)
    shell("chmod 755 {}".format(BENCH))


def remote_exists(path):
    result = subprocess.run(
        ["adb", "shell", "test", "-f", path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def run_cell(path, size, transfer, rep, out_dir, run_id):
    remote = "/data/local/tmp/ext-{}.json".format(run_id)
    shell("rm -f {}".format(remote))
    args = args_for(path, size, transfer, remote) + ["--energy", "--cpu", CPU]
    cmd = "su -c '{} {}'".format(BENCH, " ".join(args))
    print(
        "[{}/{} rep{}] path {} run {} ...".format(size, transfer, rep, path, run_id),
        flush=True,
    )
    out = shell(cmd)
    if not remote_exists(remote):
        raise RuntimeError("bench produced no report: {}".format(out))
    local = out_dir / "{}-{}-{}-{}.json".format(size, transfer, path, rep)
    subprocess.run(
        ["adb", "pull", remote, str(local)], check=True, capture_output=True
    )
    shell("rm -f {}".format(remote))
    report = json.loads(local.read_text())
    return verify(report, size, transfer, path)


def milliseconds(report, key, index):
    """`report[key][index]` in milliseconds, or None when the series is absent."""
    series = report.get(key)
    if not isinstance(series, list) or len(series) <= index or series[index] is None:
        return None
    return series[index] * 1e3


def summarize(report):
    energy = report.get("energy") or {}
    memory = report.get("memory") or {}
    steady = (memory.get("steady") or {}).get("process") or {}
    gpu = (((memory.get("steady") or {}).get("engine") or {}).get("value") or {})
    pacing = report.get("pacing") or {}
    conditions = report.get("conditions") or {}
    return {
        "gpu_ms_p50": milliseconds(report, "composite_seconds", 0),
        "gpu_ms_p99": milliseconds(report, "composite_seconds", 2),
        "total_ms_p50": milliseconds(report, "total_seconds", 0),
        "total_ms_p99": milliseconds(report, "total_seconds", 2),
        "encode_ms_p50": milliseconds(report, "encode_seconds", 0),
        "encode_ms_p99": milliseconds(report, "encode_seconds", 2),
        "submit_ms_p50": milliseconds(report, "submit_seconds", 0),
        "handoff_ms_p50": milliseconds(report, "handoff_seconds", 0),
        "joules_per_frame": energy.get("joules_per_frame"),
        "watts": energy.get("watts"),
        "steady_rss_bytes": steady.get("rss_bytes"),
        "steady_gpu_bytes": gpu.get("gpu_bytes"),
        "missed": pacing.get("missed_deadlines"),
        "thermal": conditions.get("thermal_status"),
        "dropped_frames": report.get("dropped_frames"),
        "bench_gpu_bytes": report.get("bench_gpu_bytes"),
        "import_form": report.get("import_form"),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, help="aarch64 release binary")
    parser.add_argument("--out-dir", default="/tmp/external-cost-pixel")
    parser.add_argument("--runs", type=int, default=4, help="runs per order (ABAB => 4)")
    args = parser.parse_args()
    out_dir = pathlib.Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    binary = pathlib.Path(args.binary)
    binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
    matrix_id = uuid.uuid4().hex[:12]

    push(str(binary))
    shell("input keyevent KEYCODE_SLEEP")

    results = []
    git_sha = None
    initial = cooled()
    print("initial thermal: {}".format(initial), flush=True)
    for size, transfer in CELLS:
        for order in ORDERS:
            for rep, path in enumerate(order[: args.runs]):
                sample = cooled()
                run_id = uuid.uuid4().hex[:12]
                report = run_cell(path, size, transfer, rep, out_dir, run_id)
                if git_sha is None:
                    git_sha = report["git_sha"]
                elif report["git_sha"] != git_sha:
                    raise RuntimeError(
                        "git_sha changed from {} to {}".format(git_sha, report["git_sha"])
                    )
                row = {
                    "cell": "{}/{}".format(size, transfer),
                    "path": path,
                    "order": order_name(order),
                    "rep": rep,
                    "run_id": run_id,
                    "battery_c": sample["battery_c"],
                    "status": sample["status"],
                    **summarize(report),
                }
                results.append(row)
                print(json.dumps(row), flush=True)
    summary = {
        "device": "pixel9pro",
        "run_id": matrix_id,
        "binary": str(binary),
        "binary_sha256": binary_sha256,
        "git_sha": git_sha,
        "frames": FRAMES,
        "warmup": WARMUP,
        "rate": RATE,
        "finished": datetime.datetime.now(datetime.UTC).isoformat(),
        "results": results,
    }
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print("summary -> {}".format(out_dir / "summary.json"))


if __name__ == "__main__":
    main()

"""Run only under pixel-adb --run: its device lock covers the entire experiment."""

import argparse
import datetime
import json
import pathlib
import re
import selectors
import subprocess
import time
from zoneinfo import ZoneInfo

PKG = "cool.lexo.cherenkov_planes_static"
ACTIVITY = PKG + "/android.app.NativeActivity"
TAG = "cherenkov-planes"


def adb(*args):
    result = subprocess.run(
        ["adb", *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"adb {args}: {result.stdout}")
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
            f"screen-off cooling: battery={sample['battery_c']} C, status={sample['status']}",
            flush=True,
        )
        time.sleep(5)
    raise RuntimeError("cooling timeout: battery must be below 45 C and status below 2")


def odpm():
    raw = shell(
        "su -c 'for d in /sys/bus/iio/devices/iio:device*/; do echo DEV:$d; cat $d/enabled_rails; cat $d/energy_value; done'"
    )
    rails = {}
    times = {}
    device = None
    for line in raw.splitlines():
        if line.startswith("DEV:"):
            device = line.removeprefix("DEV:").strip("/").split("/")[-1]
        if match := re.match(r"t=(\d+)", line):
            times[device] = int(match[1])
        if match := re.match(r"CH(\d+)\(T=\d+\)\[([^\]]+)\],\s*(\d+)", line):
            rails[device + ":" + match[2]] = int(match[3])
    if not rails:
        raise RuntimeError("ODPM provided no rails: " + raw)
    return {"rails_uws": rails, "times_ms": times, "raw": raw}


def heartbeat():
    raw = shell(f"logcat -d -v raw -s {TAG}:D")
    rows = [line for line in raw.splitlines() if " gpu_bytes=" in line]
    if not rows:
        raise RuntimeError("no recorded-content heartbeat: " + raw)
    return rows[-1]


def launch(scenario):
    shell(f"am force-stop {PKG}")
    shell("logcat -c")
    stream = subprocess.Popen(
        ["adb", "logcat", "-v", "raw", "-s", TAG + ":D"],
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    selector = selectors.DefaultSelector()
    selector.register(stream.stdout, selectors.EVENT_READ)
    start = time.monotonic()
    try:
        result = shell(f"am start -W -n {ACTIVITY} --es scenario {scenario}")
        if "Error" in result:
            raise RuntimeError(result)
        deadline = start + 30
        checked = start
        while time.monotonic() < deadline:
            if time.monotonic() >= checked + 1:
                thermal()
                checked = time.monotonic()
            if not selector.select(min(1, max(0, deadline - time.monotonic()))):
                continue
            line = stream.stdout.readline()
            if (
                " gpu_bytes=" in line
                and "decision=unseen" not in line
                and "decision=pending" not in line
            ):
                return {
                    "launch_seconds": time.monotonic() - start,
                    "heartbeat": line.strip(),
                }
        raise RuntimeError(
            "no settled verdict within 30 seconds: "
            + shell("logcat -d -b crash")
            + shell(f"logcat -d -s {TAG}:D")
        )
    finally:
        selector.close()
        stream.terminate()
        stream.wait()


def timed_window(seconds):
    deadline = time.monotonic() + seconds
    samples = []
    while time.monotonic() < deadline:
        samples.append(thermal())
        time.sleep(min(1, max(0, deadline - time.monotonic())))
    return samples


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario")
    parser.add_argument("--apk")
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if (
        re.fullmatch(r"(?:static|animated):[1-9]\d*:[1-9]\d*:\d+", args.scenario)
        is None
    ):
        parser.error("scenario must be static|animated:side:count:lifetime")
    args.out.mkdir(parents=True, exist_ok=True)
    print(
        "lock acquired",
        datetime.datetime.now(ZoneInfo("America/New_York")).isoformat(),
        flush=True,
    )
    shell("input keyevent KEYCODE_SLEEP")
    cooled()
    if args.apk:
        print(adb("install", "-r", args.apk), flush=True)
    saved = {
        key: shell("settings get system " + key).strip()
        for key in ["screen_brightness", "screen_brightness_mode", "screen_off_timeout"]
    }
    try:
        shell("settings put system screen_brightness_mode 0")
        shell("settings put system screen_brightness 0")
        shell("settings put system screen_off_timeout 600000")
        order = ["plane", "engine"] * 6 + ["engine", "plane", "engine", "plane"]
        if args.smoke:
            order = ["plane", "engine"]
        for index, mode in enumerate(order):
            initial = cooled()
            shell("input keyevent KEYCODE_WAKEUP")
            shell("wm dismiss-keyguard")
            launched = launch(args.scenario + ":" + mode)
            print(index, mode, launched, flush=True)
            prefix = args.out / f"{index:02d}-{mode}"
            prefix.with_suffix(".initial-thermal.json").write_text(
                json.dumps(initial, indent=2) + "\n"
            )
            if not args.smoke:
                settle = timed_window(5)
                while not cool_enough(thermal()):
                    shell(f"am force-stop {PKG}")
                    cooled()
                    shell("input keyevent KEYCODE_WAKEUP")
                    shell("wm dismiss-keyguard")
                    launched = launch(args.scenario + ":" + mode)
                    settle = timed_window(5)
                before = odpm()
                first = heartbeat()
                temps = timed_window(20)
                last = heartbeat()
                after = odpm()
                rails = {
                    rail: (after["rails_uws"][rail] - value) / 1e6
                    for rail, value in before["rails_uws"].items()
                }
                seconds = {
                    dev: (after["times_ms"][dev] - value) / 1000
                    for dev, value in before["times_ms"].items()
                }
                frames = int(re.search(r"frame=(\d+)", last)[1]) - int(
                    re.search(r"frame=(\d+)", first)[1]
                )
                record = {
                    "time": datetime.datetime.now(
                        ZoneInfo("America/New_York")
                    ).isoformat(),
                    "scenario": args.scenario,
                    "mode": mode,
                    "index": index,
                    "launch": launched,
                    "before": before,
                    "after": after,
                    "first": first,
                    "last": last,
                    "frames": frames,
                    "seconds": seconds,
                    "joules": rails,
                    "total_j": sum(rails.values()),
                    "initial_thermal": initial,
                    "settle_thermal": settle,
                    "thermal": temps,
                    "meminfo": shell(f"dumpsys meminfo {PKG}"),
                }
                prefix.with_suffix(".json").write_text(
                    json.dumps(record, indent=2) + "\n"
                )
                print(
                    f"window={index} mode={mode} J={record['total_j']:.4f} frames={frames} seconds={seconds}",
                    flush=True,
                )
            else:
                thermal()
                prefix.with_suffix(".sf.txt").write_text(
                    shell("dumpsys SurfaceFlinger")
                )
                with prefix.with_suffix(".png").open("wb") as output:
                    subprocess.run(
                        ["adb", "exec-out", "screencap", "-p"],
                        stdout=output,
                        check=True,
                    )
                prefix.with_suffix(".log.txt").write_text(
                    shell(f"logcat -d -v raw -s {TAG}:D")
                )
                prefix.with_suffix(".memory.txt").write_text(
                    shell(f"dumpsys meminfo {PKG}")
                )
            shell(f"am force-stop {PKG}")
            shell("input keyevent KEYCODE_SLEEP")
    except Exception as error:
        (args.out / "aborted.json").write_text(
            json.dumps(
                {
                    "time": datetime.datetime.now(
                        ZoneInfo("America/New_York")
                    ).isoformat(),
                    "error": str(error),
                },
                indent=2,
            )
            + "\n"
        )
        raise
    finally:
        shell(f"am force-stop {PKG}")
        for key, value in saved.items():
            shell(f"settings put system {key} {value}")
        shell("input keyevent KEYCODE_SLEEP")


if __name__ == "__main__":
    main()

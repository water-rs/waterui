"""Run on the Mac mini; one iPhone lock lease per completed measurement.

Launch with nohup and --completion-fifo. The app persists its report and
exits; devicectl --console is the completion signal, not a timed sleep.
"""

import argparse
import datetime
import fcntl
import json
import pathlib
import signal
import subprocess
import uuid
from zoneinfo import ZoneInfo

DEVICE = "00008140-001845681E98801C"
LOCK = pathlib.Path("/tmp/device-locks") / (DEVICE + ".lock")
BUNDLE = "dev.cherenkov.bench"


def now():
    return datetime.datetime.now(ZoneInfo("America/New_York")).isoformat()


def command(log, *args):
    result = subprocess.run(
        ["xcrun", "devicectl", *args], stdout=log, stderr=subprocess.STDOUT, check=False
    )
    if result.returncode:
        raise RuntimeError(f"devicectl exited {result.returncode}: {args}")


def lock_timeout(_signal, _frame):
    raise TimeoutError("iPhone lock remained held for 15 minutes")


def measure(args):
    args.out.mkdir(parents=True, exist_ok=True)
    signal.signal(signal.SIGALRM, lock_timeout)
    order = ["plane", "engine"] * 6 + ["engine", "plane", "engine", "plane"]
    for index, mode in enumerate(order):
        result_path = args.out / f"{index:02d}-{mode}.json"
        if index < args.start_index:
            if not result_path.is_file():
                raise RuntimeError(f"missing completed window: {result_path}")
            continue
        if result_path.exists():
            raise RuntimeError(f"refusing to overwrite completed window: {result_path}")
        identity = str(uuid.uuid4())
        print(now(), "waiting for lock", index, mode, flush=True)
        with LOCK.open("a+") as lock:
            signal.alarm(15 * 60)
            try:
                fcntl.flock(lock, fcntl.LOCK_EX)
            finally:
                signal.alarm(0)
            print(now(), "lock acquired", index, mode, flush=True)
            with (args.out / f"{index:02d}-{mode}.log").open("w") as log:
                # Other jobs use the same signed bundle ID. Installation is
                # inside each lease so every launch runs this exact build.
                command(
                    log, "device", "install", "app", "--device", DEVICE, str(args.app)
                )
                command(
                    log,
                    "device",
                    "process",
                    "launch",
                    "--device",
                    DEVICE,
                    "--terminate-existing",
                    "--console",
                    BUNDLE,
                    "--",
                    "--scenario",
                    f"static:512:1:0:{mode}",
                    "--measure",
                    "--run-id",
                    identity,
                )
                command(
                    log,
                    "device",
                    "copy",
                    "from",
                    "--device",
                    DEVICE,
                    "--domain-type",
                    "appDataContainer",
                    "--domain-identifier",
                    BUNDLE,
                    "--source",
                    "Documents/planes-result.json",
                    "--destination",
                    str(result_path),
                )
            result = json.loads(result_path.read_text())
            if result["run_id"] != identity:
                raise RuntimeError("the app did not produce this run's report")
            expected = "TranslucentAbove" if mode == "engine" else "promoted"
            if not result["decision"].startswith(expected):
                raise RuntimeError(f"wrong realization: {result['decision']}")
            print(
                now(),
                index,
                mode,
                result["energy"],
                "frames",
                result["frames"],
                flush=True,
            )
        print(now(), "lock released", index, mode, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--completion-fifo", type=pathlib.Path, required=True)
    parser.add_argument("--start-index", type=int, choices=range(16), default=0)
    args = parser.parse_args()
    result = {"time": now(), "exit_code": 1}
    try:
        measure(args)
        result["exit_code"] = 0
    except (OSError, RuntimeError, ValueError, KeyError) as error:
        result["error"] = str(error)
        print(now(), "stopped", str(error), flush=True)
    finally:
        args.out.mkdir(parents=True, exist_ok=True)
        result["finished"] = now()
        (args.out / "completion.json").write_text(json.dumps(result, indent=2) + "\n")
        # The FIFO is created before nohup starts this job. The remote waiter
        # reads one completion record and returns when this writer closes.
        with args.completion_fifo.open("w") as complete:
            json.dump(result, complete)
            complete.write("\n")
    raise SystemExit(result["exit_code"])


if __name__ == "__main__":
    main()

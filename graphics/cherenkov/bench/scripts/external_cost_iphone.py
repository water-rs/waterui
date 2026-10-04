#!/usr/bin/env python3
"""iPhone 16 Pro driver for `cherenkov-bench external-cost` (#168).

Runs on the Mac mini where the phone is attached, through devicectl.
Several jobs share the phone and the same bundle id, so the device lock
(`fcntl.flock`) is held for one run's whole cycle — uninstall, install,
push args, launch (always `--terminate-existing`: the app never exits,
so a plain launch would reuse a stale instance), wait for done, pull —
and released after the pull. An exception after launch terminates
that process and uninstalls before the lock drops. A successful pull
leaves the app installed for the next cycle, which uninstalls before
it installs again; the hot thermal path uninstalls before it returns.
Time blocked on the lock accumulates; past 15 minutes the driver stops
with whatever it has already pulled.

Before the measurement the host writes `Documents/thermal.json` from
`ProcessInfo.thermalState` and does not run the bench unless the state
is nominal or fair. A serious or critical reading ends that launch, the
lock is released, and the driver waits outside the lock until the next
probe. iOS has no host-side thermal query, so the probe is that launch.

iOS carries the same matrix minus `--energy` (the meter is ODPM/
powermetrics only) and `--cpu` (affinity is Linux/Android only).

A pull is accepted only when `done.json`'s `run_id` is the one this
launch wrote into `bench-args.json`, its thermal words are nominal or
fair, and the report is the `external-cost` cell this binary was
installed to measure. The git SHA comes from the report
(`CHERENKOV_GIT_SHA` in the binary), not from a `--head` argument.

Raw reports land in `--out-dir`, never in git.

SIGTERM and SIGHUP end the run through one exit path: status.json
records ``matrix stopped: <signal>`` and the log prints that line.
A driver launched under nohup, which ignores SIGHUP, becomes its own
session before the handler is installed, so the launching shell's
hangup is not delivered and an explicit ``kill -HUP`` still is.
"""

import argparse
import errno
import fcntl
import hashlib
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import threading
import time
import traceback
import uuid

from external_cost_matrix import (
    CELLS,
    OK_THERMAL,
    ORDERS,
    args_for,
    order_name,
    verify as verify_report,
)

UDID = "00008140-001845681E98801C"
LOCK = "/tmp/device-locks/{}.lock".format(UDID)
BUNDLE = "dev.cherenkov.bench"
DEFAULT_APP = (
    pathlib.Path(__file__).resolve().parent.parent
    / "ios"
    / "build"
    / "Build"
    / "Products"
    / "Release-iphoneos"
    / "CherenkovBench.app"
)

LOCK_BUDGET_S = 15 * 60
COOL_LIMIT_S = 300
COOL_GAP_S = 15

# Seconds spent blocked in flock across the process.
lock_waited = 0.0


class LockBudget(Exception):
    """Cumulative time blocked on the device lock passed 15 minutes."""


class ThermalTimeout(Exception):
    """The phone stayed above fair for the cool-down limit."""


class Stopped(Exception):
    """SIGTERM or SIGHUP. The process records the stop and exits."""

    def __init__(self, signame):
        super().__init__(signame)
        self.signame = signame


def run(cmd, timeout=180):
    return subprocess.run(
        cmd, capture_output=True, text=True, timeout=timeout, check=False
    )


def devicectl(*args):
    result = run(["xcrun", "devicectl", *args])
    if result.returncode != 0:
        raise RuntimeError(
            "devicectl {} failed: {}".format(args, result.stderr or result.stdout)
        )
    return result.stdout


def device_copy_from(source, dest):
    """Copy `source` out of the app container.

    The destination must not already exist: a file source lands as that
    file, and a directory source lands as its contents, without a
    Documents/ wrapper. Copying a file onto an existing directory fails.
    """
    if dest.exists():
        if dest.is_dir():
            shutil.rmtree(dest)
        else:
            dest.unlink()
    dest.parent.mkdir(parents=True, exist_ok=True)
    return run([
        "xcrun", "devicectl", "device", "copy", "from",
        "--device", UDID, "--domain-type", "appDataContainer",
        "--domain-identifier", BUNDLE,
        "--source", source, "--destination", str(dest),
    ])


def fetched(source, dest):
    """The file `device_copy_from` just wrote, wherever devicectl put it."""
    if dest.is_file():
        return dest
    if dest.is_dir():
        return find_named(dest, pathlib.Path(source).name)
    return None


def find_named(root, name):
    direct = root / name
    if direct.is_file():
        return direct
    matches = [path for path in root.rglob(name) if path.is_file()]
    if not matches:
        return None
    return min(matches, key=lambda path: len(path.parts))


def uninstall():
    result = run([
        "xcrun", "devicectl", "device", "uninstall", "app",
        "--device", UDID, BUNDLE,
    ])
    if result.returncode == 0:
        return
    blob = (result.stderr or "") + (result.stdout or "")
    lowered = blob.lower()
    if "not installed" in lowered or "could not find" in lowered or "not found" in lowered:
        return
    raise RuntimeError("devicectl uninstall failed: {}".format(blob))


class Lock:
    """Exclusive flock. The blocked time counts toward the 15-minute budget."""

    def __enter__(self):
        global lock_waited
        self.held = False
        self.f = open(LOCK, "a+")
        remaining = LOCK_BUDGET_S - lock_waited
        if remaining <= 0:
            self.f.close()
            raise LockBudget(
                "lock wait already {:.0f}s".format(lock_waited)
            )
        started = time.monotonic()
        acquired = threading.Event()

        def grab():
            try:
                fcntl.flock(self.f, fcntl.LOCK_EX)
            except OSError:
                return
            acquired.set()

        threading.Thread(target=grab, daemon=True).start()
        ok = acquired.wait(remaining)
        waited = time.monotonic() - started
        lock_waited += waited
        if not ok:
            raise LockBudget(
                "lock wait {:.0f}s exceeds 15 min".format(lock_waited)
            )
        if lock_waited > LOCK_BUDGET_S:
            fcntl.flock(self.f, fcntl.LOCK_UN)
            self.f.close()
            raise LockBudget(
                "lock wait {:.0f}s exceeds 15 min".format(lock_waited)
            )
        self.held = True
        print(
            "lock acquired after {:.1f}s (cumulative {:.1f}s)".format(
                waited, lock_waited
            ),
            flush=True,
        )
        return self

    def __exit__(self, *exc):
        if self.held:
            fcntl.flock(self.f, fcntl.LOCK_UN)
            self.f.close()
            self.held = False


def cdhash(app):
    result = run(["codesign", "-dvvv", app])
    blob = result.stderr + result.stdout
    for line in blob.splitlines():
        if line.startswith("CDHash="):
            return line.split("=", 1)[1].strip()
    raise RuntimeError("codesign reported no CDHash for {}".format(app))


def exe_sha256(app):
    path = pathlib.Path(app) / "CherenkovBench"
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    return digest


def wait_file(source, dest, timeout, predicate):
    """Poll `source` until it copies to `dest` and `predicate` accepts it."""
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        result = device_copy_from(source, dest)
        found = fetched(source, dest)
        if found is not None:
            try:
                if predicate(found):
                    return found.read_bytes()
            except (OSError, json.JSONDecodeError, KeyError, IndexError, AssertionError) as exc:
                last = exc
        else:
            last = (result.stderr or result.stdout or "").strip()
        time.sleep(2)
    raise RuntimeError(
        "timeout waiting for {} ({})".format(source, last)
    )


def verify(report_path, size, transfer, path):
    """The pulled report is this build's `external-cost` cell, on Metal."""
    report = verify_report(
        json.loads(pathlib.Path(report_path).read_text()), size, transfer, path
    )
    if str(report["backend"]).lower() != "metal":
        raise AssertionError(report["backend"])
    if "apple" not in report["adapter"].lower():
        raise AssertionError(report["adapter"])
    if path == "e" and report["import_form"] != "planes":
        raise AssertionError(report["import_form"])
    return report


def process_identifier(obj):
    """`processIdentifier` anywhere in a devicectl launch JSON document."""
    if isinstance(obj, dict):
        if "processIdentifier" in obj:
            return int(obj["processIdentifier"])
        for value in obj.values():
            found = process_identifier(value)
            if found is not None:
                return found
    elif isinstance(obj, list):
        for value in obj:
            found = process_identifier(value)
            if found is not None:
                return found
    return None


def stop_launched(pid):
    """SIGKILL the launch, then uninstall. Both stay inside the lock."""
    if pid is not None:
        result = run([
            "xcrun", "devicectl", "device", "process", "terminate",
            "--device", UDID, "--pid", str(pid), "--kill",
        ], timeout=60)
        if result.returncode != 0:
            print(
                "terminate pid {} failed: {}".format(
                    pid, (result.stderr or result.stdout or "").strip()
                ),
                flush=True,
            )
    try:
        uninstall()
    except Exception as exc:
        print("uninstall during cleanup failed: {}".format(exc), flush=True)


def snapshot_app(src, out_dir):
    """Copy the .app into `out_dir` and return that copy."""
    dest = out_dir / "CherenkovBench.app"
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(src, dest)
    return dest


def cycle(app, path, size, transfer, rep, order_name, run_id, out_dir, identity):
    """One locked cycle. Returns ("ok", row) or ("hot", state).

    ``bench-args.json`` carries this launch's ``run_id``. The report
    file is ``ext-{size}-{transfer}-{path}-{rep}.json`` with no run id
    in the name. Cleanup runs only when an exception is in flight, so
    a normal return is not torn down while the pull is still in use.
    """
    pid = None
    try:
        name = "{}-{}-{}-{}".format(size, transfer, path, rep)
        remote_name = "ext-{}.json".format(name)
        remote = "Documents/out/{}".format(remote_name)
        args = args_for(path, size, transfer, remote)
        payload = {"run_id": run_id, "runs": [args]}
        print(
            "[{}/{} {} rep{}] path {} run {} — installing".format(
                size, transfer, order_name, rep, path, run_id
            ),
            flush=True,
        )
        uninstall()
        devicectl("device", "install", "app", "--device", UDID, str(app))

        args_file = out_dir / "bench-args.json"
        args_file.write_text(json.dumps(payload))
        devicectl(
            "device", "copy", "to", "--device", UDID,
            "--domain-type", "appDataContainer",
            "--domain-identifier", BUNDLE,
            "--source", str(args_file),
            "--destination", "Documents/bench-args.json",
        )
        back_path = out_dir / "args-readback.json"
        back_result = device_copy_from("Documents/bench-args.json", back_path)
        back = fetched("Documents/bench-args.json", back_path)
        if back is None or json.loads(back.read_text()) != payload:
            raise RuntimeError(
                "bench-args.json readback failed: {}".format(
                    (back_result.stderr or back_result.stdout or "").strip()
                )
            )

        stale_path = out_dir / "thermal-stale.json"
        device_copy_from("Documents/thermal.json", stale_path)
        if fetched("Documents/thermal.json", stale_path) is not None:
            raise RuntimeError("stale thermal.json survived uninstall")

        launched_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        launch_json = out_dir / "launch-{}.json".format(run_id)
        devicectl(
            "device", "process", "launch",
            "--terminate-existing", "--device", UDID,
            "--json-output", str(launch_json),
            BUNDLE,
        )
        pid = process_identifier(json.loads(launch_json.read_text()))
        if pid is None:
            raise RuntimeError("launch json has no processIdentifier")

        def this_thermal(path):
            doc = json.loads(path.read_text())
            return doc.get("run_id") == run_id and isinstance(doc.get("state"), str)

        thermal_bytes = wait_file(
            "Documents/thermal.json",
            out_dir / "thermal.json",
            60,
            this_thermal,
        )
        thermal = json.loads(thermal_bytes.decode())["state"]
        print("    thermal {}".format(thermal), flush=True)
        if thermal not in OK_THERMAL:
            def hot_done(path):
                doc = json.loads(path.read_text())
                return doc.get("run_id") == run_id and doc.get("thermal") == thermal

            wait_file(
                "Documents/out/done.json",
                out_dir / "done-hot.json",
                45,
                hot_done,
            )
            uninstall()
            return ("hot", thermal)

        run_dir = out_dir / "run-{}-{}".format(name, run_id)
        scratch = out_dir / ".out-pull"
        deadline = time.monotonic() + 300
        pulled = None
        names = []
        while time.monotonic() < deadline:
            result = device_copy_from("Documents/out", scratch)
            done = find_named(scratch, "done.json") if scratch.exists() else None
            log = find_named(scratch, "run-0.log") if scratch.exists() else None
            report_file = find_named(scratch, remote_name) if scratch.exists() else None
            if done is not None:
                try:
                    doc = json.loads(done.read_text())
                except json.JSONDecodeError:
                    doc = None
                results = doc.get("results") if isinstance(doc, dict) else None
                if (
                    isinstance(doc, dict)
                    and doc.get("run_id") == run_id
                    and isinstance(results, list)
                    and results
                ):
                    row = results[0]
                    before = row.get("thermal_before")
                    after = row.get("thermal_after")
                    if (
                        row.get("args") != args
                        or row.get("exit_code") != 0
                        or before not in OK_THERMAL
                        or after not in OK_THERMAL
                    ):
                        raise RuntimeError(
                            "done.json rejected: exit {} thermal {} -> {}".format(
                                row.get("exit_code"), before, after
                            )
                        )
                    if log is None or report_file is None:
                        names = sorted(
                            item.name for item in scratch.rglob("*") if item.is_file()
                        )
                    else:
                        if run_dir.exists():
                            shutil.rmtree(run_dir)
                        shutil.copytree(scratch, run_dir)
                        pulled = (run_dir, before, after)
                        break
            if scratch.exists():
                names = sorted(
                    item.name for item in scratch.rglob("*") if item.is_file()
                )
            elif result.returncode != 0:
                names = [(result.stderr or result.stdout or "").strip()]
            time.sleep(2)
        if pulled is None:
            raise RuntimeError(
                "timeout waiting for done.json run {}; pulled {}".format(run_id, names)
            )

        run_dir, thermal_before, thermal_after = pulled
        report_path = find_named(run_dir, remote_name)
        report = verify(report_path, size, transfer, path)
        accept = {
            "run_id": run_id,
            "launched_at_utc": launched_at,
            "thermal_before": thermal_before,
            "thermal_after": thermal_after,
            "git_sha": report["git_sha"],
            "cdhash": identity["cdhash"],
            "exe_sha256": identity["exe_sha256"],
            "args": args,
            "adapter": report["adapter"],
            "backend": report["backend"],
        }
        (run_dir / "accept.json").write_text(json.dumps(accept, indent=2) + "\n")
        gpu = report["total_seconds"]
        print(
            "    accepted {} thermal {} -> {} gpu p50 {:.2f} ms".format(
                run_id, thermal_before, thermal_after, gpu[0] * 1e3
            ),
            flush=True,
        )
        return ("ok", {
            "cell": "{}/{}".format(size, transfer),
            "path": path,
            "order": order_name,
            "rep": rep,
            "run_id": run_id,
            "thermal_before": thermal_before,
            "thermal_after": thermal_after,
            "git_sha": report["git_sha"],
            "dir": run_dir.name,
            "gpu_p50_s": gpu[0],
            "gpu_p99_s": gpu[2],
        })
    finally:
        if sys.exc_info()[0] is not None:
            stop_launched(pid)


def one_run(app, path, size, transfer, rep, order_name, out_dir, identity):
    """Thermal-gated run. Retries once on an operational failure."""
    last = None
    for attempt in (1, 2):
        run_id = uuid.uuid4().hex[:12]
        cool_start = None
        try:
            while True:
                with Lock():
                    kind, payload = cycle(
                        app, path, size, transfer, rep, order_name,
                        run_id, out_dir, identity,
                    )
                if kind == "ok":
                    return payload
                if cool_start is None:
                    cool_start = time.monotonic()
                elapsed = time.monotonic() - cool_start
                if elapsed > COOL_LIMIT_S:
                    raise ThermalTimeout(
                        "thermal stayed {} for {:.0f}s".format(payload, elapsed)
                    )
                print(
                    "    thermal {}; cooling {:.0f}s outside the lock".format(
                        payload, elapsed
                    ),
                    flush=True,
                )
                time.sleep(COOL_GAP_S)
        except (LockBudget, ThermalTimeout, Stopped):
            raise
        except Exception as exc:
            last = exc
            print(
                "    attempt {} failed: {}".format(attempt, exc),
                flush=True,
            )
    raise last


def write_status(path, payload):
    payload["lock_wait_s"] = round(lock_waited, 1)
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(payload, indent=2))
    tmp.replace(path)


def signal_done(out_dir, state):
    fifo = out_dir / "done.fifo"
    try:
        fd = os.open(str(fifo), os.O_WRONLY | os.O_NONBLOCK)
    except OSError:
        return
    try:
        os.write(fd, (state + "\n").encode())
    finally:
        os.close(fd)


def install_stop_signals():
    """Raise Stopped on SIGTERM and SIGHUP.

    nohup sets SIGHUP to ignore so an ssh hangup does not kill a
    detached driver. Leave that process group first — an explicit
    ``kill -HUP`` is still delivered to the pid — then install the
    handler. A foreground run, whose SIGHUP is not ignored, keeps its
    session and records the hangup. ``setsid`` failing with EPERM is
    the only case that leaves SIGHUP ignored; any other OSError is
    raised.
    """
    names = {signal.SIGTERM: "SIGTERM", signal.SIGHUP: "SIGHUP"}

    def handle(signum, _frame):
        raise Stopped(names[signum])

    if (
        signal.getsignal(signal.SIGHUP) == signal.SIG_IGN
        and os.getpid() != os.getsid(0)
    ):
        try:
            os.setsid()
        except OSError as exc:
            if exc.errno != errno.EPERM:
                raise
            print(
                "setsid failed with EPERM; leaving SIGHUP ignored",
                flush=True,
            )
            signal.signal(signal.SIGTERM, handle)
            return

    signal.signal(signal.SIGTERM, handle)
    signal.signal(signal.SIGHUP, handle)


def finish(out_dir, status_path, status, code, line):
    """Write status.json, wake a fifo waiter, print the matrix line, exit."""
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    write_status(status_path, status)
    signal_done(out_dir, status["state"])
    print(line, flush=True)
    sys.exit(code)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", default=os.path.expanduser(DEFAULT_APP))
    parser.add_argument("--out-dir", default="/tmp/external-cost-iphone")
    parser.add_argument(
        "--lock-waited",
        type=float,
        default=0.0,
        help="seconds already spent blocked on the device lock",
    )
    args = parser.parse_args()
    global lock_waited
    lock_waited = args.lock_waited
    out_dir = pathlib.Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    status_path = out_dir / "status.json"
    results = []
    status = {
        "state": "running",
        "reason": "",
        "git_sha": "",
        "cdhash": "",
        "exe_sha256": "",
        "results": results,
    }
    write_status(status_path, status)
    install_stop_signals()
    code = 0
    line = "matrix complete"
    try:
        app = snapshot_app(pathlib.Path(args.app).expanduser(), out_dir)
        identity = {"cdhash": cdhash(app), "exe_sha256": exe_sha256(app)}
        status["cdhash"] = identity["cdhash"]
        status["exe_sha256"] = identity["exe_sha256"]
        print(
            "binary {} {} lock_waited {:.1f}s".format(
                identity["cdhash"], identity["exe_sha256"][:16], lock_waited
            ),
            flush=True,
        )
        for size, transfer in CELLS:
            for order in ORDERS:
                for rep, path in enumerate(order):
                    row = one_run(
                        app, path, size, transfer, rep,
                        order_name(order), out_dir, identity,
                    )
                    sha = row["git_sha"]
                    if status["git_sha"] == "":
                        status["git_sha"] = sha
                        print("git {}".format(sha), flush=True)
                    elif status["git_sha"] != sha:
                        raise RuntimeError(
                            "git_sha changed from {} to {}".format(
                                status["git_sha"], sha
                            )
                        )
                    results.append(row)
                    status["results"] = results
                    write_status(status_path, status)
        status["state"] = "complete"
    except Stopped as exc:
        code = 2
        status["state"] = "stopped"
        status["reason"] = "matrix stopped: {}".format(exc.signame)
        line = status["reason"]
    except LockBudget as exc:
        code = 2
        status["state"] = "stopped"
        status["reason"] = str(exc)
        print("stopped: {}".format(exc), flush=True)
        line = "matrix stopped"
    except Exception as exc:
        code = 1
        status["state"] = "failed"
        status["reason"] = str(exc)
        traceback.print_exc()
        line = "matrix failed"
    status["results"] = results
    finish(out_dir, status_path, status, code, line)


if __name__ == "__main__":
    main()

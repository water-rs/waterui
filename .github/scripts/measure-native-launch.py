#!/usr/bin/env python3
"""Launch a packaged WaterUI app and prove — or measure — its first paint.

usage:
    measure-native-launch.py ios-simulator <udid> <bundle-id> [--metrics-json PATH]
    measure-native-launch.py macos <executable> [--metrics-json PATH]

Readiness is the app's own signal, not a returned pid. One bounded launch
protocol serves both platforms: attach `log stream --style ndjson` —
inside the booted simulator on iOS, on the host for macOS — and wait for
the stream's `Filtering the log data` attach header BEFORE launching, so a
marker can never be lost to an attach race. Then launch and accept
`waterui_first_paint_ms=` only when the event's processID is this launch's
own pid. A marker emitted by an older run of the same bundle id, or by any
other process on the subsystem, is rejected. Because nothing is consumed
while the launch is in flight, no event is read before the pid is known;
there is no poll, no sleep, and no replay of the persisted log store — an
old run's marker can never satisfy this launch.

Without --metrics-json the proof is strict: stream loss, a failed launch,
a process that exits before painting, or deadline expiry all exit nonzero
— the native tests rely on that proof. --metrics-json PATH switches to
reporting: the launch must still be proven (attach and launch failures
exit nonzero), but once the app is running a marker that never arrives
records "first_paint_ms": null — the app ran, the marker pipeline broke,
and that distinction belongs in the data — while the owned pid's resident
set is sampled over the post-launch idle window into "peak_rss_bytes". No
measurement is ever bound to a pid this run did not create.

One 30 s deadline covers the whole protocol — stream spawn, attach,
launch and the marker wait all draw from it. RSS sampling runs parallel
to the marker wait inside its own ~4 s window, and cleanup is separately
bounded, so no path can wait forever. On every path — success, failure,
interruption — the owned log-stream child is killed and the launched app
is terminated.

stdout: `first paint: <ms> ms` (strict) or a one-line summary (report);
diagnostics on stderr; the report goes to the --metrics-json file.
"""

from __future__ import annotations

import argparse
import asyncio
import dataclasses
import json
import re
import sys
import time
from pathlib import Path

SUBSYSTEM = "dev.waterui"
MARKER = re.compile(r"waterui_first_paint_ms=(\d+)\b")
ATTACH_PREFIX = "Filtering the log data"
LAUNCH_PID = re.compile(r":\s*(\d+)\s*$")
# One protocol budget covers stream spawn, attach, launch and the marker
# wait in both modes — the same 30 s the strict proof always had.
DEADLINE_S = 30.0
RSS_SAMPLES = 8
RSS_INTERVAL_S = 0.5
# A hung `ps` is a pathology, not data: bound each probe.
PS_TIMEOUT_S = 5.0
# The RSS window is the sampling loop's own bound, not a second 30 s: the
# samples run parallel to the marker wait starting at launch.
RSS_BOUND_S = RSS_SAMPLES * RSS_INTERVAL_S + PS_TIMEOUT_S
# Cleanup is bounded too: every kill/reap and terminate gets this grace.
TERMINATE_GRACE_S = 5.0
# The outer bound the whole process answers to: the 30 s protocol budget,
# then the tail of the RSS window, then concurrently-bounded cleanup
# (reap + drain + terminate each use one grace). ~54 s worst case.
TOTAL_BOUND_S = DEADLINE_S + RSS_BOUND_S + 3 * TERMINATE_GRACE_S


class Failure(Exception):
    """Any path on which this launch never proved it reached first paint."""


class LaunchFailed(Failure):
    """The owned launch itself failed or the process died before painting."""


class _Deadline:
    """One absolute deadline every protocol phase draws from."""

    __slots__ = ("_ends",)

    def __init__(self, budget_s):
        self._ends = time.monotonic() + budget_s

    async def wait(self, awaitable):
        return await asyncio.wait_for(
            awaitable, timeout=max(self._ends - time.monotonic(), 0.0))


async def _reap(proc):
    """SIGKILL `proc` if it is still running and reap it within the grace.

    Every child this script owns exits through here — an unbounded wait
    after a kill would defeat the deadline that asked for the kill, and a
    child that does not die is a failure worth reporting, not a leak.
    """
    if proc.returncode is None:
        proc.kill()
    try:
        await asyncio.wait_for(proc.wait(), TERMINATE_GRACE_S)
    except asyncio.TimeoutError as exc:
        raise Failure(f"child {proc.pid} did not exit after kill") from exc


@dataclasses.dataclass(frozen=True)
class SimulatorLaunch:
    """An app launched inside a booted iOS simulator by bundle id."""

    udid: str
    bundle_id: str


@dataclasses.dataclass(frozen=True)
class MacOSLaunch:
    """An app launched on the host by executing its bundle binary."""

    executable: Path


def _describe(target):
    if isinstance(target, SimulatorLaunch):
        return target.bundle_id
    return str(target.executable)


def _stream_argv(target):
    argv = [
        "log", "stream", "--level", "info",
        "--predicate", f'subsystem == "{SUBSYSTEM}"', "--style", "ndjson",
    ]
    if isinstance(target, SimulatorLaunch):
        return ["xcrun", "simctl", "spawn", target.udid, *argv]
    return argv


async def _launch(target):
    """Start the app; return (owned pid, app process or None for simctl)."""
    if isinstance(target, SimulatorLaunch):
        proc = await asyncio.create_subprocess_exec(
            "xcrun", "simctl", "launch", "--terminate-running-process",
            target.udid, target.bundle_id,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )
        try:
            out, _ = await proc.communicate()
        finally:
            await _reap(proc)
        text = out.decode("utf-8", errors="replace").strip()
        if proc.returncode != 0:
            raise LaunchFailed(f"simctl launch {target.bundle_id} failed: {text}")
        match = LAUNCH_PID.search(text)
        if not match:
            raise LaunchFailed(f"simctl launch reported no pid: {text!r}")
        return int(match.group(1)), None
    if not target.executable.is_file():
        raise LaunchFailed(
            f"bundle executable does not exist: {target.executable}")
    proc = await asyncio.create_subprocess_exec(
        str(target.executable),
        stdout=asyncio.subprocess.DEVNULL,
        stderr=asyncio.subprocess.DEVNULL,
    )
    return proc.pid, proc


def _marker_ms(event, pid):
    """Marker value when `event` is the first-paint log from process `pid`."""
    if not isinstance(event, dict):
        raise Failure("log stream event is not a JSON object")
    if event.get("processID") != pid or event.get("subsystem") != SUBSYSTEM:
        return None
    match = MARKER.search(event.get("eventMessage") or "")
    return int(match.group(1)) if match else None


async def _lines(stdout):
    while True:
        raw = await stdout.readline()
        if not raw:
            raise Failure("log stream closed before the first-paint marker")
        yield raw.decode("utf-8", errors="replace").rstrip("\n")


async def _await_attach(lines):
    async for line in lines:
        if line.startswith(ATTACH_PREFIX):
            return
        raise Failure(f"unexpected log stream output before attach: {line!r}")


async def _await_marker(lines, pid):
    async for line in lines:
        if line.startswith(ATTACH_PREFIX):
            continue
        try:
            event = json.loads(line)
        except ValueError as exc:
            raise Failure(f"unexpected log stream output: {line!r}") from exc
        ms = _marker_ms(event, pid)
        if ms is not None:
            return ms
    raise Failure("log stream ended before the first-paint marker")


async def _await_first_paint(lines, pid, app_proc):
    """First-paint ms from `pid`, or LaunchFailed if the app died first."""
    marker = asyncio.ensure_future(_await_marker(lines, pid))
    exit_watch = (asyncio.ensure_future(app_proc.wait())
                  if app_proc is not None else None)
    waiters = {marker} | ({exit_watch} if exit_watch is not None else set())
    try:
        done, _pending = await asyncio.wait(
            waiters, return_when=asyncio.FIRST_COMPLETED)
        if exit_watch is not None and exit_watch in done:
            raise LaunchFailed(
                f"the app exited during launch (status {app_proc.returncode})")
        return await marker
    finally:
        pending = [task for task in waiters if not task.done()]
        for task in pending:
            task.cancel()
        if pending:
            await asyncio.gather(*pending, return_exceptions=True)


async def _rss_kb_once(pid):
    """One RSS probe of `pid` in KiB, or None once the process is gone.

    `ps -o rss=` prints exactly one integer and exits 0 for a live pid;
    for an exited pid it prints nothing and exits 1. Only those two
    shapes are valid — empty output at any other status, a nonzero
    status with output, or anything but one integer violates the probe's
    contract and is a Failure, never a sample to fold in.
    """
    proc = await asyncio.create_subprocess_exec(
        "ps", "-o", "rss=", "-p", str(pid),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL,
    )
    try:
        out, _ = await proc.communicate()
    finally:
        await _reap(proc)
    fields = out.split()
    if proc.returncode == 1 and not fields:
        return None
    if proc.returncode != 0 or len(fields) != 1:
        raise Failure(
            f"ps -o rss= -p {pid} exited {proc.returncode}: {out!r}")
    try:
        return int(fields[0])
    except ValueError as exc:
        raise Failure(
            f"ps -o rss= -p {pid} printed non-integer output: {out!r}") from exc


async def _sample_peak_rss(pid):
    """Peak RSS in bytes of `pid` over the post-launch idle window.

    Simulator apps are host processes and `simctl launch` returns the host
    pid, so `ps` covers both platforms directly. Sampling stops early when
    the owned process exits — the window is defined by its lifetime.
    """
    peak_kb = 0
    for _ in range(RSS_SAMPLES):
        kb = await asyncio.wait_for(_rss_kb_once(pid), PS_TIMEOUT_S)
        if kb is None:
            break
        peak_kb = max(peak_kb, kb)
        await asyncio.sleep(RSS_INTERVAL_S)
    return peak_kb * 1024 if peak_kb else None


async def _drain(task):
    """Cancel an owned task if it is still running and return its result.

    Returns the task's exception object for the caller to report — every
    owned task's outcome is retrieved on every path, so no failure is
    abandoned inside a task nobody awaited.
    """
    if task is None:
        return None
    if not task.done():
        task.cancel()
    (result,) = await asyncio.gather(task, return_exceptions=True)
    return result


async def _terminate(target, app_proc):
    """End the owned app: the simulator app by bundle id, the host process.

    Every wait is bounded — cleanup must finish inside the run's outer
    deadline even when the platform layer misbehaves.
    """
    if isinstance(target, SimulatorLaunch):
        terminate = await asyncio.wait_for(
            asyncio.create_subprocess_exec(
                "xcrun", "simctl", "terminate", target.udid, target.bundle_id,
                stdout=asyncio.subprocess.DEVNULL,
                stderr=asyncio.subprocess.DEVNULL),
            TERMINATE_GRACE_S)
        try:
            await asyncio.wait_for(terminate.wait(), TERMINATE_GRACE_S)
        finally:
            await _reap(terminate)
        if terminate.returncode != 0:
            raise Failure(
                f"simctl terminate {target.bundle_id} "
                f"exited {terminate.returncode}")
        return
    if app_proc is not None and app_proc.returncode is None:
        app_proc.terminate()
        try:
            await asyncio.wait_for(app_proc.wait(), TERMINATE_GRACE_S)
        except asyncio.TimeoutError:
            await _reap(app_proc)


async def _run(target, metrics_path):
    deadline = _Deadline(DEADLINE_S)
    try:
        stream = await deadline.wait(asyncio.create_subprocess_exec(
            *_stream_argv(target),
            stdout=asyncio.subprocess.PIPE,
        ))
    except asyncio.TimeoutError as exc:
        raise Failure("the log stream did not start in time") from exc
    app_proc = None
    rss_task = None
    # Once the launch attempt exists the app may have started even if the
    # launch call itself is interrupted — cleanup keys on the attempt, not
    # on a parsed pid or a live process handle.
    launch_attempted = False
    try:
        lines = _lines(stream.stdout)
        try:
            await deadline.wait(_await_attach(lines))
        except asyncio.TimeoutError as exc:
            raise Failure("the log stream did not attach in time") from exc
        launch_attempted = True
        try:
            pid, app_proc = await deadline.wait(_launch(target))
        except asyncio.TimeoutError as exc:
            raise LaunchFailed(
                f"{_describe(target)} did not launch within "
                f"{DEADLINE_S:.0f}s") from exc
        if metrics_path is None:
            ms = await deadline.wait(_await_first_paint(lines, pid, app_proc))
            print(f"first paint: {ms} ms")
            return
        # RSS sampling runs parallel to the marker wait so the whole idle
        # window of the owned process is measured; it owns a small bound of
        # its own, not a second protocol deadline.
        rss_task = asyncio.ensure_future(_sample_peak_rss(pid))
        try:
            ms = await deadline.wait(_await_first_paint(lines, pid, app_proc))
        except LaunchFailed:
            raise
        except (Failure, asyncio.TimeoutError) as exc:
            reason = str(exc) or f"no marker within {DEADLINE_S:.0f}s"
            print(f"::warning::{_describe(target)} ran but its first-paint "
                  f"marker could not be proven ({reason})", file=sys.stderr)
            ms = None
        try:
            peak_rss = await asyncio.wait_for(rss_task, RSS_BOUND_S)
        except asyncio.TimeoutError as exc:
            raise Failure("peak RSS sampling did not complete in time") \
                from exc
        metrics_path.write_text(
            json.dumps(
                {"first_paint_ms": ms, "peak_rss_bytes": peak_rss}) + "\n",
            encoding="utf-8")
        print(f"first paint: {ms if ms is not None else 'null'} ms; "
              f"peak rss: {peak_rss if peak_rss is not None else 'null'} B")
    finally:
        # Every owned resource is cleaned up independently and
        # concurrently — a failure in one must not starve the others, and
        # each step carries its own bound. A task we cancelled ourselves
        # reports CancelledError and is expected; anything else that failed
        # is surfaced, never swallowed.
        cleanups = [_drain(rss_task), _reap(stream)]
        if launch_attempted:
            cleanups.append(_terminate(target, app_proc))
        results = await asyncio.gather(*cleanups, return_exceptions=True)
        problems = [
            repr(result) for result in results
            if isinstance(result, BaseException)
            and not isinstance(result, asyncio.CancelledError)]
        if problems:
            raise Failure("cleanup failed: " + "; ".join(problems))


def main():
    parser = argparse.ArgumentParser(
        prog="measure-native-launch.py",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(
        dest="platform", required=True, metavar="platform")
    ios = sub.add_parser(
        "ios-simulator",
        help="launch by bundle id inside a booted iOS simulator")
    ios.add_argument("udid", help="UDID of the booted simulator")
    ios.add_argument("bundle_id", help="CFBundleIdentifier of the installed app")
    macos = sub.add_parser(
        "macos", help="launch a bundle's executable on this host")
    macos.add_argument(
        "executable", type=Path,
        help="the app's CFBundleExecutable inside the packaged .app")
    for command in (ios, macos):
        command.add_argument(
            "--metrics-json", type=Path, metavar="PATH", default=None,
            help="report first_paint_ms and peak_rss_bytes to PATH; a "
                 "launched app whose marker never arrives records null "
                 "instead of failing (launch failures still fail)")
    args = parser.parse_args()
    if args.platform == "ios-simulator":
        target = SimulatorLaunch(args.udid, args.bundle_id)
    else:
        target = MacOSLaunch(args.executable)
    try:
        # One explicit bound for the whole run: the protocol deadline
        # inside plus the bounded cleanup in _run's finally.
        asyncio.run(asyncio.wait_for(
            _run(target, args.metrics_json), TOTAL_BOUND_S))
    except Failure as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
    except asyncio.TimeoutError:
        print(f"error: no first-paint marker from {_describe(target)} "
              f"within {DEADLINE_S:.0f}s", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()

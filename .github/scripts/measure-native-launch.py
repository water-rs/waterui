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
# Inside that one grace, TERM comes first with this much of it: TERM is
# the signal a wrapper can forward — `simctl spawn` reparents its log
# child under simlaunchd, so a wrapper that is only ever SIGKILLed leaks
# the child while a TERM'd wrapper forwards and exits. The remainder of
# the grace is the hard bound for a child that resists TERM.
TERM_GRACE_S = 3.0
# The outer bound the whole process answers to: the 30 s protocol budget,
# then the tail of the RSS window, then concurrently-bounded cleanup
# (reap + drain + terminate each use one grace). ~54 s worst case.
TOTAL_BOUND_S = DEADLINE_S + RSS_BOUND_S + 3 * TERMINATE_GRACE_S


class Failure(Exception):
    """A launch-protocol failure, named by the phase that produced it."""

    def __init__(self, phase, detail):
        self.phase = phase
        super().__init__(f"{phase}: {detail}")


class LaunchFailed(Failure):
    """The owned launch itself failed or the process died before painting."""


class CleanupFailed(Failure):
    """One bounded cleanup step failed, named by the step it ran."""

    def __init__(self, step, detail):
        super().__init__(f"cleanup {step}", detail)


class FailureReport(Failure):
    """Every failure the run produced, kept together and named by phase.

    A cleanup failure never replaces the protocol failure it ran
    alongside — the report carries all of them. Members that are
    themselves reports are flattened, so every cause is named once.
    """

    def __init__(self, failures):
        flat = []
        for failure in failures:
            if isinstance(failure, FailureReport):
                flat.extend(failure.failures)
            else:
                flat.append(failure)
        self.failures = tuple(flat)
        detail = "; ".join(
            _describe_failure(failure) for failure in self.failures)
        super().__init__(f"{len(self.failures)} failures", detail)


def _describe_failure(exc):
    """One failure rendered for stderr: its own message, or a repr."""
    if isinstance(exc, asyncio.CancelledError):
        return "interrupted: CancelledError"
    return str(exc) or repr(exc)


def _proc_outcome(proc):
    """A child's reaped outcome for cleanup diagnostics."""
    if proc.returncode is None:
        return f"pid={proc.pid} still running"
    return f"pid={proc.pid} exited {proc.returncode}"


class _Deadline:
    """One absolute deadline every protocol phase draws from."""

    __slots__ = ("_ends",)

    def __init__(self, budget_s):
        self._ends = time.monotonic() + budget_s

    def expired(self):
        """Whether the shared budget has already been consumed."""
        return time.monotonic() >= self._ends

    async def wait(self, awaitable):
        return await asyncio.wait_for(
            awaitable, timeout=max(self._ends - time.monotonic(), 0.0))


async def _reap(proc, subtree=False):
    """SIGTERM `proc` first, escalating to SIGKILL inside one grace.

    Every child this script owns exits through here. TERM is the
    forwarding signal: a wrapper child (a `simctl spawn` log stream is
    reparented under simlaunchd) delivers it to the real child and then
    exits itself — SIGKILL can never be forwarded. A child that resists
    TERM is killed with the remaining budget. When `subtree` marks a
    wrapper whose real child lives under a foreign parent, a forced kill
    reaps only the wrapper — the forwarded child may still run — so that
    is reported as a named failure, never claimed as a clean reap.
    """
    if proc.returncode is not None:
        await proc.wait()
        return
    proc.terminate()
    try:
        await asyncio.wait_for(proc.wait(), TERM_GRACE_S)
        return
    except asyncio.TimeoutError:
        pass
    proc.kill()
    try:
        await asyncio.wait_for(proc.wait(), TERMINATE_GRACE_S - TERM_GRACE_S)
    except asyncio.TimeoutError as exc:
        raise Failure(
            "reap", f"child {proc.pid} did not exit after kill") from exc
    if subtree:
        raise Failure(
            "reap",
            f"child {proc.pid} resisted TERM and was SIGKILLed; a child "
            f"it forwarded under simlaunchd may still run")


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


async def _cleanup(name, describe, awaitable):
    """Run one cleanup step without replacing a pending failure.

    A cleanup failure while an exception is already propagating joins
    that exception in a FailureReport instead of masking it; alone it
    raises as a CleanupFailed named for the step. Cancellation is
    re-raised only when no other failure is in flight — otherwise the
    interrupted step is still reported by name.
    """
    pending = sys.exc_info()[1]
    started = time.monotonic()
    try:
        await awaitable
    except BaseException as exc:
        if pending is None and isinstance(exc, asyncio.CancelledError):
            raise
        issue = CleanupFailed(
            name, f"{describe()} after {time.monotonic() - started:.2f}s: "
                  f"{_describe_failure(exc)}")
        if pending is None:
            raise issue from exc
        raise FailureReport((pending, issue)) from pending


async def _attempt_cleanup(name, describe, awaitable):
    """Await one cleanup step; return its named failure, or None.

    A task we cancelled ourselves reports CancelledError and is
    expected; anything else that failed is surfaced, never swallowed.
    """
    started = time.monotonic()
    try:
        result = await awaitable
    except BaseException as exc:
        result = exc
    if (isinstance(result, BaseException)
            and not isinstance(result, asyncio.CancelledError)):
        if isinstance(result, (CleanupFailed, FailureReport)):
            return result
        return CleanupFailed(
            name, f"{describe()} after {time.monotonic() - started:.2f}s: "
                  f"{_describe_failure(result)}")
    return None


class _OwnedChild:
    """A subprocess slot owned by this run, created before the wait.

    `spawn` records the Process immediately after
    `create_subprocess_exec` returns, with no await between recording
    and returning — so a run whose result delivery is discarded by
    cancellation still owns the child it created. `reap` is a no-op in
    the unspawned state; afterwards it reaps through the bounded
    mechanism and never masks a failure already propagating.
    """

    __slots__ = ("argv", "process")

    def __init__(self):
        self.argv = None
        self.process = None

    async def spawn(self, argv, **kwargs):
        self.argv = argv
        self.process = await asyncio.create_subprocess_exec(
            *argv, **kwargs)
        return self.process

    def describe(self):
        """Target pid/argv/outcome for cleanup diagnostics."""
        return f"{_proc_outcome(self.process)} argv={self.argv!r}"

    async def reap(self, step, subtree=False):
        if self.process is not None:
            await _cleanup(
                step, self.describe,
                _reap(self.process, subtree=subtree))


async def _launch(target, app_slot):
    """Start the app; return (owned pid, app process or None for simctl).

    The macOS child is spawned into the caller-owned `app_slot`, so the
    run keeps its handle even if this coroutine's result is discarded
    by a cancellation on delivery.
    """
    if isinstance(target, SimulatorLaunch):
        slot = _OwnedChild()
        proc = await slot.spawn(
            ["xcrun", "simctl", "launch", "--terminate-running-process",
             target.udid, target.bundle_id],
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )
        try:
            out, _ = await proc.communicate()
        finally:
            await slot.reap("reap simctl launch")
        text = out.decode("utf-8", errors="replace").strip()
        if proc.returncode != 0:
            raise LaunchFailed(
                "launch",
                f"simctl launch {target.bundle_id} failed: {text}")
        match = LAUNCH_PID.search(text)
        if not match:
            raise LaunchFailed(
                "launch", f"simctl launch reported no pid: {text!r}")
        return int(match.group(1)), None
    if not target.executable.is_file():
        raise LaunchFailed(
            "launch",
            f"bundle executable does not exist: {target.executable}")
    proc = await app_slot.spawn(
        [str(target.executable)],
        stdout=asyncio.subprocess.DEVNULL,
        stderr=asyncio.subprocess.DEVNULL,
    )
    return proc.pid, proc


def _marker_ms(event, pid):
    """Marker value when `event` is the first-paint log from process `pid`."""
    if not isinstance(event, dict):
        raise Failure(
            "first-paint", "log stream event is not a JSON object")
    if event.get("processID") != pid or event.get("subsystem") != SUBSYSTEM:
        return None
    match = MARKER.search(event.get("eventMessage") or "")
    return int(match.group(1)) if match else None


async def _lines(stdout):
    while True:
        raw = await stdout.readline()
        if not raw:
            raise Failure(
                "log stream", "closed before the first-paint marker")
        yield raw.decode("utf-8", errors="replace").rstrip("\n")


async def _await_attach(lines):
    async for line in lines:
        if line.startswith(ATTACH_PREFIX):
            return
        raise Failure(
            "attach",
            f"unexpected log stream output before attach: {line!r}")


async def _await_marker(lines, pid):
    async for line in lines:
        if line.startswith(ATTACH_PREFIX):
            continue
        try:
            event = json.loads(line)
        except ValueError as exc:
            raise Failure(
                "first-paint",
                f"unexpected log stream output: {line!r}") from exc
        ms = _marker_ms(event, pid)
        if ms is not None:
            return ms
    raise Failure(
        "first-paint", "log stream ended before the first-paint marker")


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
                "launch",
                f"the app exited during launch "
                f"(status {app_proc.returncode})")
        return await marker
    finally:
        for task in waiters:
            if not task.done():
                task.cancel()
        # Every waiter is gathered, not only the cancelled ones — a task
        # that already completed with an exception is retrieved here too,
        # never abandoned unreported.
        await asyncio.gather(*waiters, return_exceptions=True)


async def _rss_kb_once(pid):
    """One RSS probe of `pid` in KiB, or None once the process is gone.

    `ps -o rss=` prints exactly one integer and exits 0 for a live pid;
    for an exited pid it prints nothing and exits 1. Only those two
    shapes are valid — empty output at any other status, a nonzero
    status with output, or anything but one integer violates the probe's
    contract and is a Failure, never a sample to fold in.
    """
    slot = _OwnedChild()
    proc = await slot.spawn(
        ["ps", "-o", "rss=", "-p", str(pid)],
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL,
    )
    try:
        out, _ = await proc.communicate()
    finally:
        await slot.reap("reap ps")
    fields = out.split()
    if proc.returncode == 1 and not fields:
        return None
    if proc.returncode != 0 or len(fields) != 1:
        raise Failure(
            "rss",
            f"ps -o rss= -p {pid} exited {proc.returncode}: {out!r}")
    try:
        return int(fields[0])
    except ValueError as exc:
        raise Failure(
            "rss",
            f"ps -o rss= -p {pid} printed non-integer output: {out!r}") \
            from exc


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


def _app_outcome(target, app_proc):
    """The owned app's identity for cleanup diagnostics."""
    if app_proc is not None:
        return (f"{_proc_outcome(app_proc)} "
                f"argv={[str(target.executable)]!r}")
    return f"simulator app {_describe(target)}"


async def _terminate(target, app_proc):
    """End the owned app: the simulator app by bundle id, the host process.

    Every wait is bounded — cleanup must finish inside the run's outer
    deadline even when the platform layer misbehaves.
    """
    if isinstance(target, SimulatorLaunch):
        slot = _OwnedChild()
        try:
            terminate = await asyncio.wait_for(
                slot.spawn(
                    ["xcrun", "simctl", "terminate",
                     target.udid, target.bundle_id],
                    stdout=asyncio.subprocess.DEVNULL,
                    stderr=asyncio.subprocess.DEVNULL),
                TERMINATE_GRACE_S)
            try:
                await asyncio.wait_for(terminate.wait(), TERMINATE_GRACE_S)
            except asyncio.TimeoutError as exc:
                raise Failure(
                    "terminate",
                    f"simctl terminate {target.bundle_id} did not exit "
                    f"within {TERMINATE_GRACE_S:.0f}s") from exc
        finally:
            await slot.reap("reap simctl terminate")
        if terminate.returncode != 0:
            raise Failure(
                "terminate",
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
    # Every spawned child is owned through a slot established before the
    # cancellable wait that delivers it — a spawn whose result never
    # reaches its caller is still reaped by the slot.
    stream_slot = _OwnedChild()
    app_slot = _OwnedChild()
    app_proc = None
    rss_task = None
    # Once the launch attempt exists the app may have started even if the
    # launch call itself is interrupted — cleanup keys on the attempt, not
    # on a parsed pid or a live process handle.
    launch_attempted = False
    try:
        try:
            await deadline.wait(stream_slot.spawn(
                _stream_argv(target), stdout=asyncio.subprocess.PIPE))
        except asyncio.TimeoutError as exc:
            raise Failure(
                "attach", "the log stream did not start in time") from exc
        lines = _lines(stream_slot.process.stdout)
        try:
            await deadline.wait(_await_attach(lines))
        except asyncio.TimeoutError as exc:
            raise Failure(
                "attach", "the log stream did not attach in time") from exc
        launch_attempted = True
        try:
            pid, app_proc = await deadline.wait(_launch(target, app_slot))
        except asyncio.TimeoutError as exc:
            raise LaunchFailed(
                "launch",
                f"{_describe(target)} did not launch within "
                f"{DEADLINE_S:.0f}s") from exc
        except FailureReport as exc:
            # The launch wait was interrupted and _launch's cleanup
            # failed too — the report preempted wait_for's TimeoutError.
            # Name the interrupted phase in place of the bare
            # CancelledError: the deadline's own expiry only when it has
            # provably passed, a generic interruption otherwise.
            members = [
                LaunchFailed(
                    "launch",
                    f"{_describe(target)} did not launch within "
                    f"{DEADLINE_S:.0f}s"
                    if deadline.expired() else
                    f"{_describe(target)} was interrupted during launch")
                if isinstance(failure, asyncio.CancelledError)
                else failure
                for failure in exc.failures]
            raise FailureReport(members) from exc
        if metrics_path is None:
            try:
                ms = await deadline.wait(
                    _await_first_paint(lines, pid, app_proc))
            except asyncio.TimeoutError as exc:
                raise Failure(
                    "first-paint",
                    f"no first-paint marker from {_describe(target)} "
                    f"within {DEADLINE_S:.0f}s") from exc
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
            raise Failure(
                "rss", "peak RSS sampling did not complete in time") \
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
        # each step carries its own bound. Cleanup failures never replace
        # a failure already propagating: all of them are kept together in
        # one FailureReport, each named by the phase that produced it.
        pending = sys.exc_info()[1]
        steps = [
            ("drain RSS sampler", lambda: f"task={rss_task!r}",
             _drain(rss_task)),
            ("reap log stream", stream_slot.describe,
             stream_slot.reap(
                 "reap log stream",
                 subtree=isinstance(target, SimulatorLaunch))),
        ]
        if launch_attempted:
            steps.append((
                "terminate app",
                lambda: _app_outcome(target, app_slot.process),
                _terminate(target, app_slot.process)))
        issues = [
            issue for issue in await asyncio.gather(
                *(_attempt_cleanup(name, describe, awaitable)
                  for name, describe, awaitable in steps),
                return_exceptions=True)
            if issue is not None]
        if issues:
            if pending is None:
                if len(issues) == 1:
                    raise issues[0]
                raise FailureReport(issues)
            raise FailureReport((pending, *issues)) from pending


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
        causes = exc.failures if isinstance(exc, FailureReport) else (exc,)
        for cause in causes:
            print(f"error: {_describe_failure(cause)}", file=sys.stderr)
        sys.exit(1)
    except asyncio.TimeoutError:
        print(f"error: launch of {_describe(target)} did not complete "
              f"within the {TOTAL_BOUND_S:.0f}s run bound",
              file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
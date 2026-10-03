#!/usr/bin/env python3
"""Per-command privileged launch observer for the macOS launch leg.

`/usr/bin/log stream` requires an admin account, while the measurement
driver runs as the dedicated non-admin bench account. The existing
administrator runs this parent once per measurement command:

    sudo python3 observer.py [--user bench282] [--timeout SECONDS] -- <command...>

It spawns the shared argv tail from stream_args.py via the absolute
/usr/bin/log host binary, hands the pipe read end to the command's child as
BENCH282_LOG_STREAM_FD, and drops the child to the real bench account via
initgroups/setgid/setuid with a minimal dedicated-account environment —
real home, own toolchain bins, a bench-owned TMPDIR, plus only the
explicitly permitted DEVELOPER_DIR and BENCH282_* inputs; ambient
privileged caches/PATH/TMPDIR are not inherited. Observer stderr and its
exit status go to BENCH282_LOG_STREAM_DIAG, so the driver reports true
stream diagnostics.

Ownership: the `--timeout` deadline is armed before the first spawn, so
acquisition itself is bounded. Signal handlers install before any
acquisition and cancel the run — a SIGTERM during a spawn still unwinds
through teardown (each factory is shielded long enough to register the
process it produced, or record the acquisition as unknown). `owned`
(role, proc) pairs are the single source of truth; one finally tears down
every acquired process on every exit path: each exact group is signalled
SIGTERM then SIGKILL (the group is signalled even when its leader already
exited — descendants outlive leaders), leaders are reaped through bounded
asyncio waits, signalling errors are preserved and reported, and a status
that could not be established is a reported failure rather than an
assumed one. An observer that exits on its own — anything other than the
SIGPIPE of the driver's reader closing or a SIGTERM this parent delivered
while it was alive — fails this parent even when the child exited zero.
Pipe ends and the diagnostic file are owned as file objects closed once.
No sudoers, daemon, or account/security changes.
"""

import asyncio
import os
import pwd
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from stream_args import HOST_LOG_STREAM  # noqa: E402

MAX_STEP_S = 1800
# Ambient variables explicitly permitted through to the bench child.
PASSTHROUGH = ("DEVELOPER_DIR", "TERM", "LANG", "LC_ALL", "LC_CTYPE")


def die(msg):
    print(f"observer: {msg}", file=sys.stderr)
    sys.exit(2)


def describe(rc):
    """Human-readable outcome; rc follows waitstatus_to_exitcode semantics."""
    if rc is None:
        return "unknown"
    return f"rc={rc}" if rc >= 0 else f"signal {-rc}"


async def acquire(coro, role, owned, remaining, errors):
    """Await a subprocess factory under the shared deadline. On cancel or
    timeout, give the spawn a short bounded window to finish so the process
    it produced is registered in `owned` before propagating — a factory
    that still has not returned is cancelled and recorded as an unknown
    acquisition (reported as failure), never silently left."""
    task = asyncio.ensure_future(coro)
    try:
        proc = await asyncio.wait_for(asyncio.shield(task), remaining())
    except (asyncio.CancelledError, asyncio.TimeoutError):
        try:
            proc = await asyncio.wait_for(asyncio.shield(task), 2)
        except BaseException:
            if task.done():
                if task.cancelled():
                    owned.append((role, None))
                else:
                    # The factory actually finished — register its real
                    # result (task.result() re-raises a real spawn error
                    # so a known failure is never reported as unknown).
                    owned.append((role, task.result()))
            else:
                task.cancel()
                owned.append((role, None))
                errors.append(f"{role}: spawn interrupted/timed out "
                              f"before the factory returned — outcome "
                              f"unknown")
            raise
        owned.append((role, proc))
        raise
    owned.append((role, proc))
    return proc


async def run(argv, ent, bench_root, logs_dir, timeout):
    loop = asyncio.get_running_loop()
    interrupted = {"sig": None}
    main_task = asyncio.current_task()
    phase = ["operate"]

    def interrupt(sig):
        # The first signal is always recorded for the honest exit status;
        # it cancels the task only while acquisition/operation is live —
        # during cleanup a signal must not abort the in-flight teardown.
        if interrupted["sig"] is None:
            interrupted["sig"] = sig
        if phase[0] == "operate":
            main_task.cancel()

    # Installed before any acquisition: a signal arriving while processes
    # are being spawned still unwinds through the teardown below.
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, interrupt, sig)

    diag_path = os.path.join(
        logs_dir, f"observer-{os.getpid()}-{time.monotonic_ns()}.log")
    owned = []          # (role, proc) for every acquisition, in order
    read_f = write_f = diag = None
    timed_out = abnormal = False
    cleanup_errors = []
    notes = []
    deadline = loop.time() + timeout

    def remaining():
        return max(0.001, deadline - loop.time())

    try:
        diag = open(diag_path, "ab", buffering=0)
        # New per-command diagnostic artifact we own: keep it bench-owned so
        # the driver's records and cold-clean ownership checks stay coherent.
        os.chown(diag_path, ent.pw_uid, ent.pw_gid)
        rfd, wfd = os.pipe()
        read_f = os.fdopen(rfd, "rb", buffering=0)
        write_f = os.fdopen(wfd, "wb", buffering=0)

        await acquire(
            asyncio.create_subprocess_exec(
                *HOST_LOG_STREAM, stdout=write_f, stderr=diag,
                start_new_session=True), "observer", owned, remaining,
            cleanup_errors)
        write_f.close()  # write end transferred to the observer process

        env = {
            "HOME": ent.pw_dir,
            "USER": ent.pw_name,
            "LOGNAME": ent.pw_name,
            "SHELL": ent.pw_shell,
            "PATH": ":".join([os.path.join(ent.pw_dir, ".cargo/bin"),
                              os.path.join(ent.pw_dir, ".local/bin"),
                              "/opt/homebrew/bin", "/usr/bin", "/bin",
                              "/usr/sbin", "/sbin"]),
            "TMPDIR": os.path.join(bench_root, "tmp"),
            "BENCH282_ROOT": bench_root,
            "BENCH282_LOG_STREAM_FD": str(read_f.fileno()),
            "BENCH282_LOG_STREAM_DIAG": diag_path,
        }
        for key in PASSTHROUGH:
            if key in os.environ:
                env[key] = os.environ[key]
        for key, value in os.environ.items():
            if key.startswith("BENCH282_") and key not in env:
                env[key] = value

        def drop_credentials():
            os.initgroups(ent.pw_name, ent.pw_gid)
            os.setgid(ent.pw_gid)
            os.setuid(ent.pw_uid)

        try:
            await acquire(
                asyncio.create_subprocess_exec(
                    *argv, env=env, pass_fds=(read_f.fileno(),),
                    start_new_session=True,
                    preexec_fn=drop_credentials), "child", owned, remaining,
                cleanup_errors)
        except OSError as exc:
            die(f"failed to spawn command: {exc}")
        read_f.close()  # read end transferred to the child process

        observer = owned[0][1]
        child = owned[1][1]
        observer_task = asyncio.ensure_future(observer.wait())
        child_task = asyncio.ensure_future(child.wait())
        pending = {observer_task, child_task}
        while pending:
            done, pending = await asyncio.wait(
                pending, timeout=remaining(),
                return_when=asyncio.FIRST_COMPLETED)
            if not done:
                timed_out = True
                break
            if observer_task in done and not child_task.done():
                rc = observer_task.result()
                if rc == -signal.SIGPIPE:
                    # The driver's reader closed early: normal stream
                    # teardown, keep waiting on the child alone.
                    continue
                abnormal = True  # observer exited on its own — fail
                break
            if child_task in done:
                break
    except asyncio.TimeoutError:
        timed_out = True
    except asyncio.CancelledError:
        pass  # the interrupting signal is already recorded
    finally:
        # Teardown for every exit path: signal each exact owned group — the
        # group is signalled even when its leader already exited, since
        # descendants outlive the leader — then reap each leader through a
        # bounded native wait with KILL escalation. A signal arriving here
        # is only recorded; nothing cancels the cleanup itself, and every
        # wait is bounded against one shared cleanup deadline.
        phase[0] = "cleanup"
        cleanup_deadline = loop.time() + 30

        def cleanup_remaining():
            return max(0.001, cleanup_deadline - loop.time())

        alive_at_term = {}
        for role, proc in owned:
            if proc is None:
                continue
            alive_at_term[role] = proc.returncode is None
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            except PermissionError as exc:
                # Deliverable to no member — all already exiting (Darwin
                # EPERM on P_LEXIT) or unsignalable; judged after the wait.
                alive_at_term[role] = exc
        for role, proc in owned:
            if proc is None:
                continue
            if proc.returncode is None:
                try:
                    await asyncio.wait_for(
                        proc.wait(), min(5, cleanup_remaining()))
                    exited = True
                except asyncio.TimeoutError:
                    exited = False
                if not exited:
                    if isinstance(alive_at_term.get(role), PermissionError):
                        cleanup_errors.append(
                            f"SIGTERM killpg({role} {proc.pid}): "
                            f"{alive_at_term[role]}")
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    except PermissionError as exc:
                        cleanup_errors.append(
                            f"SIGKILL killpg({role} {proc.pid}): {exc}")
                    try:
                        await asyncio.wait_for(
                            proc.wait(), min(10, cleanup_remaining()))
                    except asyncio.TimeoutError:
                        cleanup_errors.append(
                            f"wait({role} {proc.pid}): still running")
            elif isinstance(alive_at_term.get(role), PermissionError):
                notes.append(
                    f"SIGTERM undeliverable to group {proc.pid}: leader "
                    f"already exited; members already exiting "
                    f"({alive_at_term[role]})")
            # Descendant sweep: signal the group even after the leader is
            # reaped — descendants outlive it. An EPERM here means the
            # remaining members could not be signalled: recorded as a
            # failure unless they provably exited, which the sweep's ESRCH
            # case already covers.
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass  # group empty — natively verified gone
            except PermissionError as exc:
                cleanup_errors.append(
                    f"SIGKILL killpg({role} {proc.pid}): {exc}")
        if diag is not None:
            for role, proc in owned:
                diag.write(f"{role}_exit_status="
                           f"{proc.returncode if proc else None}\n".encode())
            for line in cleanup_errors + notes:
                diag.write(f"cleanup: {line}\n".encode())
        for owned_file in (read_f, write_f, diag):
            if owned_file is not None:
                owned_file.close()

    def proc_of(role):
        return next((p for r, p in owned if r == role), None)

    child = proc_of("child")
    observer = proc_of("observer")
    child_rc = child.returncode if child else None
    observer_rc = observer.returncode if observer else None
    status = f"child {describe(child_rc)}, observer {describe(observer_rc)}"
    # Normal observer ends: clean exit, the SIGPIPE of the driver's reader
    # closing, or a SIGTERM this parent delivered while it was alive.
    observer_normal = (
        observer_rc in (0, -signal.SIGPIPE)
        or (observer_rc == -signal.SIGTERM
            and alive_at_term.get("observer") is True))
    sig = interrupted["sig"]
    failure = (abnormal or not observer_normal or cleanup_errors
               or (sig is None
                   and (child_rc is None or observer_rc is None)))
    if failure:
        print(f"observer: {status} — failed", file=sys.stderr)
        for line in cleanup_errors + notes:
            print(f"observer: cleanup: {line}", file=sys.stderr)
        sys.exit(3)
    if sig is not None:
        print(f"observer: interrupted by signal {sig}: {status}",
              file=sys.stderr)
        sys.exit(128 + sig)
    if timed_out:
        print(f"observer: timed out after {timeout}s: {status}",
              file=sys.stderr)
        sys.exit(4)
    print(f"observer: {status}", file=sys.stderr)
    if child_rc < 0:
        sys.exit(128 - child_rc)  # conventional 128+signal
    sys.exit(child_rc)


def main():
    argv = sys.argv[1:]
    user, timeout = "bench282", MAX_STEP_S
    while argv[:1] and argv[0] != "--":
        flag, argv = argv[0], argv[1:]
        if not argv:
            die(f"{flag} requires a value")
        if flag == "--user":
            user = argv[0]
        elif flag == "--timeout":
            try:
                timeout = int(argv[0])
            except ValueError:
                die(f"invalid --timeout {argv[0]!r}")
            if not 0 < timeout <= MAX_STEP_S:
                die(f"--timeout {timeout}s exceeds {MAX_STEP_S}s")
        else:
            die(f"unknown flag {flag}")
        argv = argv[1:]
    if argv[:1] == ["--"]:
        argv = argv[1:]
    if not argv:
        die("usage: sudo python3 observer.py [--user NAME] [--timeout SECONDS] "
            "-- <command...>")
    if os.geteuid() != 0:
        die("must run as root (via sudo); it owns the log stream observer")
    try:
        ent = pwd.getpwnam(user)
    except KeyError:
        die(f"unknown user {user!r}")

    bench_root = os.environ.get("BENCH282_ROOT",
                                os.path.join(ent.pw_dir, "bench282"))
    tmp_dir = os.path.join(bench_root, "tmp")
    logs_dir = os.path.join(bench_root, "logs")
    for path in (bench_root, tmp_dir, logs_dir):
        if os.path.exists(path):
            if os.stat(path).st_uid != ent.pw_uid:
                die(f"{path} exists but is not owned by {user}")
        else:
            os.makedirs(path)
            os.chown(path, ent.pw_uid, ent.pw_gid)
            print(f"observer: created {path} owned by {user}",
                  file=sys.stderr)

    asyncio.run(run(argv, ent, bench_root, logs_dir, timeout))


if __name__ == "__main__":
    main()

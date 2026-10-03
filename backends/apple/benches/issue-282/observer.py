#!/usr/bin/env python3
"""Per-command privileged launch observer for the macOS launch leg.

`/usr/bin/log stream` requires an admin account, while the measurement
driver runs as the dedicated non-admin bench account. The existing
administrator runs this parent once per measurement command:

    sudo python3 observer.py [--user bench282] [--timeout SECONDS] -- <command...>

It spawns `log stream` with the shared argv tail (stream_args.py), hands the
pipe read end to the command's child as BENCH282_LOG_STREAM_FD, and drops the
child to the real bench account via initgroups/setgid/setuid with a minimal
dedicated-account environment — real home, own toolchain bins, plus only the
explicitly permitted DEVELOPER_DIR and BENCH282_* inputs; ambient privileged
caches/PATH are not inherited. Observer stderr and its exit status go to
BENCH282_LOG_STREAM_DIAG, so the driver reports true stream diagnostics.

Ownership: this parent owns the observer and child process groups. Both
subprocesses are awaited concurrently (asyncio); the child wait is bounded
by --timeout, and SIGTERM/SIGINT cancel the wait so the finally block still
terminates and reaps the exact owned groups with bounded waits. An observer
that exits on its own — anything other than the SIGPIPE of the driver's
reader closing, or the SIGTERM this parent itself delivered — fails this
parent even when the child exited zero. Pipe ends and the diagnostic file
are owned as file objects and closed exactly once. No sudoers, daemon, or
account/security changes.
"""

import asyncio
import os
import pwd
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from stream_args import LOG_STREAM_TAIL  # noqa: E402

MAX_STEP_S = 1800
# Ambient variables explicitly permitted through to the bench child.
PASSTHROUGH = ("DEVELOPER_DIR", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR")


def die(msg):
    print(f"observer: {msg}", file=sys.stderr)
    sys.exit(2)


def describe(rc):
    """Human-readable outcome; rc follows waitstatus_to_exitcode semantics."""
    return f"rc={rc}" if rc >= 0 else f"signal {-rc}"


def group_alive(pid):
    """Whether the process group whose leader was pid still has members."""
    try:
        os.killpg(pid, 0)
        return True
    except PermissionError:
        return True  # members exist but are not signalable
    except ProcessLookupError:
        return False


async def run(argv, ent, bench_root, logs_dir, timeout):
    diag_path = os.path.join(
        logs_dir, f"observer-{os.getpid()}-{time.monotonic_ns()}.log")
    diag = open(diag_path, "ab", buffering=0)
    # New per-command diagnostic artifact we own: keep it bench-owned so the
    # driver's records and cold-clean ownership checks stay coherent.
    os.chown(diag_path, ent.pw_uid, ent.pw_gid)
    read_f = write_f = None
    observer = child = None
    cleanup_errors = []
    try:
        rfd, wfd = os.pipe()
        read_f = os.fdopen(rfd, "rb", buffering=0)
        write_f = os.fdopen(wfd, "wb", buffering=0)

        observer = await asyncio.create_subprocess_exec(
            *LOG_STREAM_TAIL, stdout=write_f, stderr=diag,
            start_new_session=True)
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
            child = await asyncio.create_subprocess_exec(
                *argv, env=env, pass_fds=(read_f.fileno(),),
                start_new_session=True, preexec_fn=drop_credentials)
        except OSError as exc:
            die(f"failed to spawn command: {exc}")
        read_f.close()  # read end transferred to the child process

        loop = asyncio.get_running_loop()
        main_task = asyncio.current_task()
        interrupted = {"sig": None}

        def interrupt(sig):
            if interrupted["sig"] is None:
                interrupted["sig"] = sig
                main_task.cancel()

        for sig in (signal.SIGTERM, signal.SIGINT):
            loop.add_signal_handler(sig, interrupt, sig)

        observer_task = asyncio.ensure_future(observer.wait())
        child_task = asyncio.ensure_future(child.wait())
        terminated_by_us = set()
        pending = {observer_task, child_task}
        deadline = loop.time() + timeout
        timed_out = abnormal = False
        try:
            while pending:
                remaining = deadline - loop.time()
                if remaining <= 0:
                    timed_out = True
                    break
                done, pending = await asyncio.wait(
                    pending, timeout=remaining,
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
        except asyncio.CancelledError:
            pass  # interrupted["sig"] holds which signal arrived
        finally:
            for proc in (child, observer):
                if proc.returncode is None:
                    try:
                        os.killpg(proc.pid, signal.SIGTERM)
                        terminated_by_us.add(proc.pid)
                    except ProcessLookupError:
                        pass
                    except PermissionError:
                        # Already exiting (Darwin EPERM on P_LEXIT, e.g. the
                        # observer handling SIGPIPE after the reader
                        # closed); the bounded wait below reaps the true
                        # status.
                        pass
            for proc, task in ((child, child_task),
                               (observer, observer_task)):
                if proc.returncode is None:
                    try:
                        await asyncio.wait_for(
                            asyncio.shield(task), 5)
                    except asyncio.TimeoutError:
                        try:
                            os.killpg(proc.pid, signal.SIGKILL)
                            terminated_by_us.add(proc.pid)
                        except ProcessLookupError:
                            pass
                        except PermissionError as exc:
                            cleanup_errors.append(
                                f"killpg {proc.pid}: {exc}")
                        try:
                            await asyncio.wait_for(
                                asyncio.shield(task), 10)
                        except asyncio.TimeoutError:
                            cleanup_errors.append(
                                f"reap {proc.pid}: still running")
        # Group-survivor check: the parent only claims cleanup when no
        # member of the exact owned groups is left after the leaders exit.
        survivors = []
        for proc in (child, observer):
            if group_alive(proc.pid):
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
                await asyncio.sleep(0.2)
                if group_alive(proc.pid):
                    survivors.append(proc.pid)
        if survivors:
            cleanup_errors.append(f"process groups survived: {survivors}")

        child_rc = child.returncode
        observer_rc = observer.returncode
        diag.write(f"observer_exit_status={observer_rc}\n"
                   f"child_exit_status={child_rc}\n".encode())

        # Normal observer ends: clean exit, the SIGPIPE of the driver's
        # reader closing, or a SIGTERM this parent itself delivered.
        observer_normal = (
            observer_rc in (0, -signal.SIGPIPE)
            or (observer_rc == -signal.SIGTERM
                and observer.pid in terminated_by_us))
        if abnormal or not observer_normal:
            print(f"observer: child {describe(child_rc)}, "
                  f"observer {describe(observer_rc)} — observer failed",
                  file=sys.stderr)
            for err in cleanup_errors:
                print(f"observer: cleanup: {err}", file=sys.stderr)
            sys.exit(3)
        if interrupted["sig"] is not None:
            sig = interrupted["sig"]
            print(f"observer: interrupted by signal {sig}: child "
                  f"{describe(child_rc)}, observer {describe(observer_rc)}",
                  file=sys.stderr)
            sys.exit(128 + sig)
        if timed_out:
            print(f"observer: timed out after {timeout}s: child "
                  f"{describe(child_rc)}, observer {describe(observer_rc)}",
                  file=sys.stderr)
            sys.exit(4)
        if cleanup_errors:
            for err in cleanup_errors:
                print(f"observer: cleanup: {err}", file=sys.stderr)
            sys.exit(3)
        print(f"observer: child {describe(child_rc)}, "
              f"observer {describe(observer_rc)}", file=sys.stderr)
        if child_rc < 0:
            sys.exit(128 - child_rc)  # conventional 128+signal
        sys.exit(child_rc)
    finally:
        for owned in (read_f, write_f, diag):
            if owned is not None:
                owned.close()


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
    logs_dir = os.path.join(bench_root, "logs")
    for path in (bench_root, logs_dir):
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

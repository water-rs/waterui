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
This parent owns the observer: every resource is held by an ExitStack from
acquisition, the child wait is bounded by --timeout (SIGALRM), and an
observer that exits early or abnormally fails this parent even when the
child exited zero. No sudoers, daemon, or account/security changes.
"""

import os
import pwd
import signal
import subprocess
import sys
import time
from contextlib import ExitStack

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from stream_args import LOG_STREAM_TAIL  # noqa: E402

MAX_STEP_S = 1800
# Ambient variables explicitly permitted through to the bench child.
PASSTHROUGH = ("DEVELOPER_DIR", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR")


def die(msg):
    print(f"observer: {msg}", file=sys.stderr)
    sys.exit(2)


def close_fd(fd):
    try:
        os.close(fd)
    except OSError:
        pass  # already closed by an earlier transfer


def describe(rc):
    """Human-readable outcome; rc follows waitstatus_to_exitcode semantics."""
    return f"rc={rc}" if rc >= 0 else f"signal {-rc}"


def reap(proc, grace_s=5):
    """SIGTERM, bounded grace, SIGKILL, reap — an exact owned process group."""
    if proc.poll() is None:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            # Gone, or already exiting (Darwin returns EPERM on P_LEXIT —
            # e.g. the observer handling SIGPIPE after the reader closed);
            # the wait below reaps it and reports its true exit status.
            pass
        try:
            proc.wait(timeout=grace_s)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
            proc.wait(timeout=10)


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
    if os.path.exists(bench_root) and os.stat(bench_root).st_uid != ent.pw_uid:
        die(f"{bench_root} exists but is not owned by {user}")
    logs_dir = os.path.join(bench_root, "logs")
    if os.path.isdir(logs_dir):
        if os.stat(logs_dir).st_uid != ent.pw_uid:
            die(f"{logs_dir} exists but is not owned by {user}")
    else:
        os.makedirs(logs_dir, exist_ok=True)
        os.chown(logs_dir, ent.pw_uid, ent.pw_gid)
    diag_path = os.path.join(
        logs_dir, f"observer-{os.getpid()}-{time.monotonic_ns()}.log")

    timed_out = False
    child_rc = observer_rc = None
    with ExitStack() as stack:
        read_fd, write_fd = os.pipe()
        stack.callback(close_fd, read_fd)
        stack.callback(close_fd, write_fd)
        diag = open(diag_path, "ab", buffering=0)
        stack.callback(diag.close)
        # New per-command diagnostic artifact we own: keep it bench-owned so
        # the driver's records and cold-clean ownership checks stay coherent.
        os.chown(diag_path, ent.pw_uid, ent.pw_gid)

        observer = subprocess.Popen(LOG_STREAM_TAIL, stdout=write_fd,
                                    stderr=diag, start_new_session=True)
        # Registered before the reaps below so it unwinds after them but
        # before diag.close: appends the true final observer status.
        stack.callback(lambda: diag.write(
            f"observer_exit_status={observer.returncode}\n".encode()))
        stack.callback(reap, observer)
        os.close(write_fd)
        write_fd = -1  # transferred to the observer process

        # Minimal dedicated-account environment: real identity + own toolchain
        # bins + explicitly permitted inputs only. No ambient CARGO_HOME,
        # RUSTUP_HOME, XDG caches or privileged PATH leaks.
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
            "BENCH282_LOG_STREAM_FD": str(read_fd),
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
            child = subprocess.Popen(argv, env=env, pass_fds=(read_fd,),
                                     start_new_session=True,
                                     preexec_fn=drop_credentials)
        except OSError as exc:
            die(f"failed to spawn command: {exc}")
        stack.callback(reap, child)  # unwinds first: child group, then observer
        os.close(read_fd)
        read_fd = -1  # transferred to the child process

        def timeout_handler(signum, frame):
            raise TimeoutError

        signal.signal(signal.SIGALRM, timeout_handler)
        signal.alarm(timeout)
        try:
            child.wait()
            child_rc = child.returncode
        except TimeoutError:
            timed_out = True
            print(f"observer: command timed out after {timeout}s",
                  file=sys.stderr)
        finally:
            signal.alarm(0)
        # Reap the observer if it already exited so reap() does not race a
        # mid-exit (P_LEXIT) process.
        observer.poll()

    if child_rc is None:
        # Timeout path: child was reaped by the stack's reap callback.
        child_rc = child.returncode if child.returncode is not None else -15
    observer_rc = observer.returncode if observer.returncode is not None else -9

    # Normal ends: clean exit, our own SIGTERM teardown, or SIGPIPE once the
    # child's reader closed. Any other observer outcome is a real failure.
    abnormal_observer = observer_rc not in (0, -signal.SIGTERM,
                                            -signal.SIGPIPE)
    print(f"observer: child {describe(child_rc)}, "
          f"observer {describe(observer_rc)}, timeout={timed_out}",
          file=sys.stderr)
    if abnormal_observer:
        print("observer: observer process failed — stream may have died "
              "mid-run", file=sys.stderr)
        sys.exit(3)
    if timed_out:
        sys.exit(4)
    if child_rc < 0:
        sys.exit(128 - child_rc)  # conventional 128+signal
    sys.exit(child_rc)


if __name__ == "__main__":
    main()

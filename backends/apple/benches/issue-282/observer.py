#!/usr/bin/env python3
"""Per-command privileged launch observer for the macOS launch leg.

`/usr/bin/log stream` requires an admin account, while the measurement
driver runs as the dedicated non-admin bench account. The existing
administrator runs this parent once per measurement command:

    sudo python3 observer.py -- <command...>
    sudo python3 observer.py --user bench282 -- <command...>

It spawns `log stream` with the same argv tail the driver records
(drive.py LOG_STREAM_TAIL — keep identical), hands the pipe read end to the
command's child as BENCH282_LOG_STREAM_FD, and drops the child to the real
bench account via initgroups/setgid/setuid with its real home environment —
no HOME impersonation, no driver-as-root. Observer stderr is appended to
BENCH282_LOG_STREAM_DIAG together with its final exit status, so the driver
can report true diagnostics when a stream fails. The parent owns and reaps
the observer, terminating its exact process group when the command exits.
No sudoers change, no daemon, no account or security changes.
"""

import os
import pwd
import signal
import subprocess
import sys
import time

# MUST equal drive.py LOG_STREAM_TAIL (asserted in test_protocol.py).
LOG_STREAM_TAIL = ["log", "stream", "--level", "info", "--style", "ndjson"]


def die(msg):
    print(f"observer: {msg}", file=sys.stderr)
    sys.exit(2)


def main():
    argv = sys.argv[1:]
    user = "bench282"
    if argv[:1] == ["--user"]:
        if len(argv) < 2:
            die("--user requires a name")
        user = argv[1]
        argv = argv[2:]
    if argv[:1] == ["--"]:
        argv = argv[1:]
    if not argv:
        die("usage: sudo python3 observer.py [--user NAME] -- <command...>")
    if os.geteuid() != 0:
        die("must run as root (via sudo); it owns the log stream observer")
    try:
        ent = pwd.getpwnam(user)
    except KeyError:
        die(f"unknown user {user!r}")

    bench_root = os.environ.get("BENCH282_ROOT",
                                os.path.join(ent.pw_dir, "bench282"))
    logs_dir = os.path.join(bench_root, "logs")
    os.makedirs(logs_dir, exist_ok=True)
    if os.stat(logs_dir).st_uid != ent.pw_uid:
        os.chown(logs_dir, ent.pw_uid, ent.pw_gid)
    diag_path = os.path.join(
        logs_dir, f"observer-{os.getpid()}-{time.monotonic_ns()}.log")

    read_fd, write_fd = os.pipe()
    diag = open(diag_path, "ab", buffering=0)
    observer = subprocess.Popen(LOG_STREAM_TAIL, stdout=write_fd,
                                stderr=diag, start_new_session=True)
    os.close(write_fd)

    env = dict(os.environ)
    env.update({
        "HOME": ent.pw_dir,
        "USER": ent.pw_name,
        "LOGNAME": ent.pw_name,
        "SHELL": ent.pw_shell,
        "BENCH282_ROOT": bench_root,
        "BENCH282_LOG_STREAM_FD": str(read_fd),
        "BENCH282_LOG_STREAM_DIAG": diag_path,
    })

    def drop_credentials():
        os.initgroups(ent.pw_name, ent.pw_gid)
        os.setgid(ent.pw_gid)
        os.setuid(ent.pw_uid)

    try:
        child = subprocess.Popen(argv, env=env, pass_fds=(read_fd,),
                                 preexec_fn=drop_credentials)
    except OSError as exc:
        die(f"failed to spawn command: {exc}")
    finally:
        os.close(read_fd)

    rc = child.wait()

    # Parent owns the observer: terminate its exact process group, bounded.
    # The observer may already have exited (poll() reaps it); skip signaling a
    # dead/recycled process group.
    if observer.poll() is None:
        try:
            os.killpg(observer.pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass
    try:
        obs_rc = observer.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(observer.pid, signal.SIGKILL)
        obs_rc = observer.wait(timeout=10)
    diag.write(f"observer_exit_status={obs_rc}\n".encode())
    diag.close()
    sys.exit(rc)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Run one native command under a shared monotonic deadline, with receipts.

usage:
    run-with-deadline.py deadline --seconds N
    run-with-deadline.py remaining --deadline-at T
    run-with-deadline.py run --deadline-at T --receipt PATH --label NAME -- argv...

`deadline` prints an absolute deadline on the system-wide CLOCK_MONOTONIC
clock — the one epoch every process on the host reads identically, so a
value computed once by the caller is a single budget that all later
invocations draw down. `remaining` prints the seconds left and exits nonzero
once the deadline has passed, so a shell loop can bound its iterations on
the same clock the commands answer to. `run` executes argv (no shell),
waiting at most the remaining budget; on expiry it SIGTERMs, then SIGKILLs,
only the child it spawned — never a process group, a simulator service, or
anything else it does not own — and reaps it before returning.

The deadline rides clock_gettime(CLOCK_MONOTONIC), not time.monotonic():
the Python documentation only guarantees monotonic() is "the same for all
processes" on macOS since 3.10 — on older interpreters its epoch is
process-relative — so it cannot carry a deadline between the shell and the
helper.

Every `run` logs a JSON `command_started` record (label, argv, owned pid,
deadline, remaining budget, wall start) to stderr before it waits, so a hang
is attributable to an exact command even if no receipt ever lands. On
completion it appends one JSON receipt to --receipt and logs the same line
to stderr: the argv, the owned pid, wall-clock start/end, the monotonic
duration, the remaining budget at start, and the outcome — exited / timeout
/ budget_exhausted / spawn_error / reap_failed — with the raw returncode.
The child's own stdout and stderr pass through untouched, so `$(...)`
capture by the caller still works.

Exit status is the contract the shell checks: the child's own status on
exit, 128+signal when the child died on a signal, 124 when the deadline
killed it (or was already spent), 125 when the deadline-killed child could
not be reaped, and 127 when the command could not be spawned at all.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import logging
import subprocess
import sys
import time
from datetime import datetime, timezone

# Matches GNU timeout(1): the shell distinguishes "the command ran and
# failed" from "the deadline ran out" by this status alone.
TIMEOUT_EXIT = 124
# The owned child outlived SIGKILL — its outcome is unverifiable, which is an
# infrastructure failure, reported distinctly rather than claimed reaped.
REAP_FAILED_EXIT = 125
SPAWN_ERROR_EXIT = 127
# A SIGKILLed child cannot refuse to die on this platform; the grace after
# SIGTERM gives a well-behaved tool a moment to flush before the kill.
REAP_GRACE_S = 5.0


@dataclasses.dataclass(frozen=True)
class Receipt:
    """One supervised command invocation, start to settled outcome."""

    label: str
    argv: list[str]
    deadline_at: float
    remaining_at_start_s: float
    started_at: str
    ended_at: str
    duration_s: float
    status: str
    pid: int | None
    returncode: int | None
    timed_out: bool


def _clock() -> float:
    # The shared budget crosses process boundaries, so it must ride the one
    # clock every process reads identically. time.monotonic() cannot serve:
    # the Python documentation only guarantees it is the same clock for all
    # processes on macOS since 3.10 — before that its epoch is
    # process-relative and cannot carry a deadline between processes.
    return time.clock_gettime(time.CLOCK_MONOTONIC)


def _iso_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _record(path: str, receipt: Receipt) -> None:
    line = json.dumps(dataclasses.asdict(receipt), separators=(",", ":"))
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(line + "\n")
    logging.info("%s", line)


def _cmd_deadline(args: argparse.Namespace) -> int:
    print(f"{_clock() + args.seconds:.6f}")
    return 0


def _cmd_remaining(args: argparse.Namespace) -> int:
    remaining = args.deadline_at - _clock()
    print(f"{remaining:.3f}")
    return 0 if remaining > 0 else 1


def _cmd_run(args: argparse.Namespace) -> int:
    argv = args.argv[1:] if args.argv[:1] == ["--"] else args.argv
    if not argv:
        logging.error("run requires a command after --")
        return 2
    started_at = _iso_now()
    started_mono = _clock()
    remaining = args.deadline_at - started_mono

    def finish(status, pid, returncode, timed_out):
        ended_mono = _clock()
        _record(
            args.receipt,
            Receipt(
                label=args.label,
                argv=argv,
                deadline_at=args.deadline_at,
                remaining_at_start_s=round(remaining, 6),
                started_at=started_at,
                ended_at=_iso_now(),
                duration_s=round(ended_mono - started_mono, 6),
                status=status,
                pid=pid,
                returncode=returncode,
                timed_out=timed_out,
            ),
        )

    if remaining <= 0:
        finish("budget_exhausted", None, None, True)
        return TIMEOUT_EXIT
    try:
        proc = subprocess.Popen(argv)
    except OSError as exc:
        logging.error("%s could not be spawned: %s", args.label, exc)
        finish("spawn_error", None, None, False)
        return SPAWN_ERROR_EXIT
    # The start record lands before the wait, so a hang is attributable to an
    # exact argv/pid even if the final receipt never arrives.
    logging.info(
        "%s",
        json.dumps(
            {
                "event": "command_started",
                "label": args.label,
                "argv": argv,
                "pid": proc.pid,
                "deadline_at": args.deadline_at,
                "remaining_at_start_s": round(remaining, 6),
                "started_at": started_at,
            },
            separators=(",", ":"),
        ),
    )
    timed_out = False
    try:
        proc.wait(timeout=remaining)
    except subprocess.TimeoutExpired:
        timed_out = True
        # Only the owned child is signalled — no process-group kill, so a
        # CoreSimulator service or any sibling the tool spawned is untouched.
        # Popen's signal/wait methods already absorb a child that exited
        # between the deadline and the signal; a wait that still times out
        # after SIGKILL is a real reap failure, surfaced rather than claimed.
        proc.terminate()
        try:
            proc.wait(timeout=REAP_GRACE_S)
        except subprocess.TimeoutExpired:
            proc.kill()
            try:
                proc.wait(timeout=REAP_GRACE_S)
            except subprocess.TimeoutExpired:
                finish("reap_failed", proc.pid, proc.returncode, True)
                raise RuntimeError(
                    f"{args.label}: owned child pid {proc.pid} did not exit "
                    "within the reap grace after SIGKILL")
    status = "timeout" if timed_out else "exited"
    finish(status, proc.pid, proc.returncode, timed_out)
    if timed_out:
        return TIMEOUT_EXIT
    if proc.returncode < 0:
        return 128 + (-proc.returncode)
    return proc.returncode


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="run-with-deadline: %(message)s")
    parser = argparse.ArgumentParser(
        prog="run-with-deadline.py",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = parser.add_subparsers(dest="command", required=True)
    deadline = sub.add_parser(
        "deadline", help="print an absolute shared deadline N seconds out")
    deadline.add_argument(
        "--seconds", type=float, required=True,
        help="budget in seconds from now")
    remaining = sub.add_parser(
        "remaining",
        help="print seconds left; exit 1 once the deadline has passed")
    remaining.add_argument(
        "--deadline-at", type=float, required=True,
        help="absolute deadline on the CLOCK_MONOTONIC clock")
    run = sub.add_parser(
        "run", help="run argv under the shared deadline, recording a receipt")
    run.add_argument(
        "--deadline-at", type=float, required=True,
        help="absolute deadline on the CLOCK_MONOTONIC clock")
    run.add_argument(
        "--receipt", required=True, metavar="PATH",
        help="JSONL file each invocation appends its receipt to")
    run.add_argument(
        "--label", required=True,
        help="receipt label identifying this command's role")
    run.add_argument("argv", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command == "deadline":
        sys.exit(_cmd_deadline(args))
    if args.command == "remaining":
        sys.exit(_cmd_remaining(args))
    try:
        sys.exit(_cmd_run(args))
    except RuntimeError as exc:
        logging.error("%s", exc)
        sys.exit(REAP_FAILED_EXIT)


if __name__ == "__main__":
    main()

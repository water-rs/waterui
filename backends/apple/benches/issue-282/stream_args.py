"""Shared launch-leg stream argv tail for drive.py and observer.py.

Raw unfiltered NDJSON: PID/subsystem filtering is owned once by
StructuredLogStream because an idle stream emits no dev.waterui events and
a wire-level predicate would deadlock attach-before-spawn.
"""

LOG_STREAM_TAIL = ("log", "stream", "--level", "info", "--style", "ndjson")

# Absolute host binary for the privileged parent (observer.py); the iOS leg
# keeps the in-simulator argv0 "log" via `simctl spawn`.
HOST_LOG_STREAM = ("/usr/bin/log",) + LOG_STREAM_TAIL[1:]

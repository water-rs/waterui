"""Shared launch-leg stream argv for drive.py and observer.py.

NDJSON is unfiltered on the wire — PID/subsystem filtering is owned once by
drive.py's StructuredLogStream, because an idle stream emits no dev.waterui
events and a wire-level predicate would deadlock attach-before-spawn.
The observer (macOS) and `xcrun simctl spawn` (iOS) run this same tail.
"""

LOG_STREAM_TAIL = ["log", "stream", "--level", "info", "--style", "ndjson"]

"""Input-to-presented-frame latency.

Section 6: correlate an input/edit ID with the first presented frame
containing its result. Hardware-input delivery-to-present and
IME-edit-to-present are measured separately; "time until invalidate" is not
latency. Measurement markers live behind equivalent benchmark configuration
in both apps and their overhead is measured separately.

The markers are logcat lines the harness and the backends emit with a shared
monotonic clock domain:

  WUI-BENCH-INPUT   id=<n> kind=<tap|ime_edit> t_input=<device_epoch_ms>
  WUI-BENCH-PRESENT id=<n> t_present=<device_epoch_ms>

`collect` pairs them per id and reports delivery→presented ms. An id that
arrives with no matching present within the timeout is a dropped marker —
reported, never silently discarded.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

from . import adb_out

INPUT_RE = re.compile(
    r"WUI-BENCH-INPUT\s+id=(\d+)\s+kind=(\w+)\s+t_input=(\d+)"
)
PRESENT_RE = re.compile(r"WUI-BENCH-PRESENT\s+id=(\d+)\s+t_present=(\d+)")


def parse_markers(logcat_text: str) -> dict:
    inputs: dict[int, tuple[str, int]] = {}
    presents: dict[int, int] = {}
    for line in logcat_text.splitlines():
        if m := INPUT_RE.search(line):
            inputs[int(m.group(1))] = (m.group(2), int(m.group(3)))
        elif m := PRESENT_RE.search(line):
            presents[int(m.group(1))] = int(m.group(2))
    per_kind: dict[str, list[int]] = {}
    dropped = []
    for marker_id, (kind, t_input) in inputs.items():
        t_present = presents.get(marker_id)
        if t_present is None:
            dropped.append(marker_id)
            continue
        per_kind.setdefault(kind, []).append(t_present - t_input)
    out: dict = {"markers_dropped": len(dropped)}
    for kind, latencies in per_kind.items():
        latencies.sort()
        out[f"{kind}_count"] = len(latencies)
        out[f"{kind}_ms_p50"] = latencies[len(latencies) // 2]
        out[f"{kind}_ms_p99"] = latencies[
            min(len(latencies) - 1, int(len(latencies) * 0.99))
        ]
        out[f"{kind}_ms_max"] = latencies[-1]
    return out


def collect(serial: str, log_path: Path, name: str) -> dict:
    """Read the round's marker stream out of logcat (dump, don't clear)."""
    try:
        text = adb_out(
            serial, "logcat", "-d", "-b", "main"
        ).decode("utf-8", "replace")
    except subprocess.CalledProcessError:
        return {}
    marker_lines = [
        line
        for line in text.splitlines()
        if "WUI-BENCH-" in line
    ]
    (log_path / f"{name}.markers.log").write_text("\n".join(marker_lines))
    return parse_markers("\n".join(marker_lines))

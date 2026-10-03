"""Frame metrics: gfxinfo/FrameMetrics collection for a fixture round.

Section 6 requires p50/p99 from actual presented-frame timing, missed slots
and maximum stalls, with 120-Hz demand during active workloads — plus the
caveat it records explicitly: window FrameMetrics can measure the decor
window while missing a GPU SurfaceView, so a Cherenkov run must additionally
capture per-surface presentation timestamps (trace/ holds the Perfetto
config; `collect` reports whether surface-level coverage was present).
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

from . import adb_out

# The percentiles the plan's frame metric requires, keyed for the result
# record. `dumpsys gfxinfo <pkg>` prints them over the process lifetime; for
# a scripted round that window is exactly the script's profile.
GFXINFO_PATTERNS = {
    "frames_total": r"Total frames rendered:\s*(\d+)",
    "frames_janky": r"Janky frames:\s*(\d+)",
    "frame_ms_p50": r"50th percentile:\s*(\d+)ms",
    "frame_ms_p90": r"90th percentile:\s*(\d+)ms",
    "frame_ms_p95": r"95th percentile:\s*(\d+)ms",
    "frame_ms_p99": r"99th percentile:\s*(\d+)ms",
    "missed_vsync": r"Number Missed Vsync:\s*(\d+)",
    "deadline_missed": r"Number Frame deadline missed:\s*(\d+)",
    "high_input_latency": r"Number High input latency:\s*(\d+)",
    "slow_ui_thread": r"Number Slow UI thread:\s*(\d+)",
    "slow_bgc_thread": r"Number Slow bitmap uploads:\s*(\d+)",
    "slow_issue_draw": r"Number Slow issue draw commands:\s*(\d+)",
}


def parse_gfxinfo(text: str) -> dict:
    return {
        key: int(m.group(1))
        for key, pattern in GFXINFO_PATTERNS.items()
        if (m := re.search(pattern, text))
    }


def parse_framestats(text: str) -> dict:
    """Per-frame stats from `dumpsys gfxinfo <pkg> framestats`.

    The CSV's INTENDED_VSYNC/VSYNC and FRAME_COMPLETED columns give actual
    presented-frame timing: frame duration = FRAME_COMPLETED - INTENDED_VSYNC
    in ns, and a missed deadline = duration > vsync period + full deadline.
    """

    lines = [
        line
        for line in text.splitlines()
        if line and not line.startswith("---") and not line.startswith("Flags")
    ]
    durations_ns: list[int] = []
    missed = 0
    max_stall_ns = 0
    header: list[str] | None = None
    for line in lines:
        fields = line.split(",")
        if header is None:
            if fields and fields[0] == "PROFILEDATA":
                header = fields[1:] if fields[1] else fields
                # framestats CSVs open with a column list row; keep the first
                # seen as the header and parse subsequent PROFILEDATA rows.
            continue
        if fields[0] != "PROFILEDATA" or len(fields) < len(header) + 1:
            continue
        row = dict(zip(header, fields[1:]))
        try:
            intended = int(row["INTENDED_VSYNC"])
            completed = int(row["FRAME_COMPLETED"])
            vsync = int(row.get("VSYNC", intended))
        except (KeyError, ValueError):
            continue
        duration = completed - intended
        if duration <= 0:
            continue
        durations_ns.append(duration)
        period = max(vsync - intended, 1)
        if duration > 2 * period:
            missed += 1
        max_stall_ns = max(max_stall_ns, duration)
    durations_ns.sort()
    out: dict = {}
    if durations_ns:

        def pct(p: float) -> float:
            idx = min(len(durations_ns) - 1, int(len(durations_ns) * p))
            return durations_ns[idx] / 1e6

        out["presented_frames"] = len(durations_ns)
        out["presented_ms_p50"] = round(pct(0.50), 3)
        out["presented_ms_p99"] = round(pct(0.99), 3)
        out["missed_slots"] = missed
        out["max_stall_ms"] = round(max_stall_ns / 1e6, 3)
    return out


def collect(
    serial: str, package: str, artifacts_dir: Path, name: str
) -> dict:
    """One round's frame metrics for `package` on `serial`."""
    metrics: dict = {}
    try:
        text = adb_out(serial, "shell", "dumpsys", "gfxinfo", package).decode(
            "utf-8", "replace"
        )
    except subprocess.CalledProcessError:
        return metrics
    (artifacts_dir / f"{name}.gfxinfo.txt").write_text(text)
    metrics.update(parse_gfxinfo(text))
    try:
        raw = adb_out(
            serial, "shell", "dumpsys", "gfxinfo", package, "framestats"
        ).decode("utf-8", "replace")
    except subprocess.CalledProcessError:
        raw = ""
    if raw:
        (artifacts_dir / f"{name}.framestats.csv").write_text(raw)
        metrics.update(parse_framestats(raw))
    # Whether the round carried surface-level presentation coverage — the
    # Perfetto SurfaceFlinger frametimeline the GPU surface requires.
    metrics["surface_presentation_captured"] = bool(
        (artifacts_dir / f"{name}.pftrace").exists()
    )
    return metrics

"""Frame metrics: gfxinfo/FrameMetrics collection for a fixture round.

Section 6 requires p50/p99 from actual presented-frame timing, missed slots
and maximum stalls, with 120-Hz demand during active workloads — plus the
caveat it records explicitly: window FrameMetrics can measure the decor
window while missing a GPU SurfaceView, so a Cherenkov run must additionally
capture per-surface presentation timestamps (trace/ holds the Perfetto
config; `collect` reports whether surface-level coverage was present).
"""

from __future__ import annotations

import csv
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
    metrics = {
        key: int(m.group(1))
        for key, pattern in GFXINFO_PATTERNS.items()
        if (m := re.search(pattern, text))
    }
    metrics.update(parse_render_nodes(text))
    return metrics


# `dumpsys gfxinfo <pkg>` closes each window's view-hierarchy section with
# "<n> views, <x> kB of render nodes": the View hierarchy size and its RenderNode memory, not a count of all
# retained RenderNodes. Summed over the package's
# windows.
_RENDER_NODES = re.compile(r"(\d+) views,\s*([\d.]+) kB of render nodes")


def parse_render_nodes(text: str) -> dict:
    """View count and RenderNode memory from `dumpsys gfxinfo`."""
    windows = _RENDER_NODES.findall(text)
    if not windows:
        return {}
    return {
        "android_view_count": sum(int(views) for views, _ in windows),
        "display_list_kb": round(sum(float(kb) for _, kb in windows), 2),
    }


def parse_framestats(text: str) -> dict:
    """Read Android's named CSV columns between PROFILEDATA delimiters.

    FrameCompleted measures window-frame completion, not surface presentation.
    FrameDeadline and FrameInterval determine missed display intervals;
    Vsync minus IntendedVsync is scheduling delay, not the display period.
    """
    required = {
        "Flags", "IntendedVsync", "FrameCompleted", "FrameDeadline",
        "FrameInterval",
    }
    durations_ns: list[int] = []
    missed = 0
    max_stall_ns = 0
    header: list[str] | None = None
    for fields in csv.reader(text.splitlines()):
        if not fields:
            continue
        if fields[0] == "---PROFILEDATA---":
            header = None
            continue
        if fields[0] == "Flags":
            missing = required.difference(fields)
            if missing:
                raise ValueError(f"framestats header lacks {sorted(missing)}")
            header = fields
            continue
        if header is None:
            continue
        row = dict(zip(header, fields, strict=True))
        # Android marks nonordinary frames with Flags.
        if int(row["Flags"]) != 0:
            continue
        intended = int(row["IntendedVsync"])
        completed = int(row["FrameCompleted"])
        deadline = int(row["FrameDeadline"])
        interval = int(row["FrameInterval"])
        duration = completed - intended
        if interval <= 0 or duration <= 0:
            raise ValueError(f"invalid framestats timing: {row}")
        durations_ns.append(duration)
        lateness = max(completed - deadline, 0)
        missed += (lateness + interval - 1) // interval
        max_stall_ns = max(max_stall_ns, duration)
    durations_ns.sort()
    out: dict = {}
    if durations_ns:

        def pct(p: float) -> float:
            idx = min(len(durations_ns) - 1, int(len(durations_ns) * p))
            return durations_ns[idx] / 1e6

        out["frame_samples"] = len(durations_ns)
        out["frame_ms_p50"] = round(pct(0.50), 3)
        out["frame_ms_p99"] = round(pct(0.99), 3)
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

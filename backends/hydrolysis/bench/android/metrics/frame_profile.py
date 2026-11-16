"""Per-frame painter counters and CPU timings from the profiling build.

The HWUI painter logs one `hydrolysis::frame_profile` line per presented
frame (`backends/hydrolysis/src/hwui/profile.rs`): JNI calls, command bytes,
nodes re-recorded, text-shape misses and font registrations since the
previous frame. A failed HWUI-versus-Compose metric is attributed with these
counters, so a round records their distribution rather than one number.
The Android host separately reports frame-transaction and renderer-stage
timings in milliseconds; these are distributions, never accumulated work.

`dumpsys gfxinfo` supplies the Android View count and RenderNode memory
separately from the encoder's live retained-node gauge
(`metrics.frames.parse_render_nodes`).

Only the HWUI painter reports: the GPU painter draws through Cherenkov, whose
stage timing is the Perfetto trace's job, so its record names the gap rather
than carrying zeros.
"""

from __future__ import annotations

from contextlib import contextmanager
import re
import subprocess
from pathlib import Path
from typing import Callable, TypeVar

from . import adb

# The counters `FrameStats::report` logs, in the order it logs them.
COUNTERS = (
    "jni_calls",
    "command_bytes",
    "nodes_rerecorded",
    "text_shape_misses",
    "font_registrations",
)

GAUGES = ("live_render_nodes",)
FIELDS = COUNTERS + GAUGES

STAGES = (
    "total_ms", "input_ms", "render_ms", "publication_ms",
    "update_ms", "layout_ms", "encode_ms",
)

_FIELD = re.compile(r"(\w+)=([^\s]+)")
Number = TypeVar("Number", int, float)


def clear_logcat(serial: str) -> None:
    """Empties every logcat buffer so a round counts only its own frames."""
    adb(serial, "logcat", "-c")


@contextmanager
def record(serial: str, output: Path):
    """Stream the round to disk before logcat's finite ring can overwrite it."""
    clear_logcat(serial)
    with output.open("wb") as stream:
        process = subprocess.Popen(
            ["adb", "-s", serial, "logcat", "-s", "hydrolysis:I"],
            stdout=stream,
        )
        try:
            yield output
        finally:
            status = process.poll()
            if status is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                    raise
            else:
                raise RuntimeError(f"frame-profile logcat exited unexpectedly: {status}")


def _parse_records(
    text: str, label: str, names: tuple[str, ...], convert: Callable[[str], Number],
) -> list[dict[str, Number]]:
    pattern = re.compile(
        rf"hydrolysis::frame_profile:.*?\b{re.escape(label)}\b(?P<fields>.*)$"
    )
    records = []
    for line in text.splitlines():
        match = pattern.search(line)
        if match is None:
            continue
        fields = dict(_FIELD.findall(match.group("fields")))
        missing = [name for name in names if name not in fields]
        if missing:
            raise ValueError(
                f"frame_profile line lacks {missing}: {line.strip()!r}"
            )
        records.append({name: convert(fields[name]) for name in names})
    return records


def parse_frames(text: str) -> list[dict[str, int]]:
    """One counter dict per HWUI counter record."""
    return _parse_records(text, "hwui frame", FIELDS, int)


def parse_stages(text: str) -> list[dict[str, float]]:
    """One millisecond timing dict per completed Android frame transaction."""
    return _parse_records(text, "hwui stages", STAGES, float)


def _percentile(sorted_values: list[Number], fraction: float) -> Number:
    return sorted_values[min(len(sorted_values) - 1, int(len(sorted_values) * fraction))]


def summarize(
    frames: list[dict[str, Number]],
    fields: tuple[str, ...] = FIELDS,
    count_key: str = "frame_profile_frames",
) -> dict:
    """Distributions for every field; totals only for accumulated counters."""
    summary: dict = {count_key: len(frames)}
    for counter in fields:
        values = sorted(frame[counter] for frame in frames)
        if counter in COUNTERS:
            summary[f"{counter}_total"] = sum(values)
        if values:
            summary[f"{counter}_p50"] = _percentile(values, 0.50)
            summary[f"{counter}_p99"] = _percentile(values, 0.99)
            summary[f"{counter}_max"] = values[-1]
    return summary


def collect(painter: str, logcat: Path) -> dict:
    """The round's counters from logcat, or the reason there are none.

    `painter` is the CLI's painter name. A build without `frame-profile`
    logs no frame line, which this reports as `frame_profile: absent` — a
    missing build feature, never a zero count.
    """
    if painter != "hwui":
        return {
            "frame_profile": "unavailable",
            "frame_profile_reason": (
                f"the {painter} painter reports its stage timing through the "
                "Perfetto trace, not the HWUI counters"
            ),
        }
    text = logcat.read_text()
    frames = parse_frames(text)
    if not frames:
        return {
            "frame_profile": "absent",
            "frame_profile_reason": (
                "no hydrolysis::frame_profile line in logcat: the app was "
                "built without the `frame-profile` feature"
            ),
        }
    return {
        "frame_profile": "present",
        **summarize(frames),
        **summarize(parse_stages(text), STAGES, "frame_stage_samples"),
    }

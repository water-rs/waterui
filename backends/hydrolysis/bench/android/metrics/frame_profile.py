"""Per-frame painter counters from the `frame-profile` Hydrolysis build.

The HWUI painter logs one `hydrolysis::frame_profile` line per presented
frame (`backends/hydrolysis/src/hwui/profile.rs`): JNI calls, command bytes,
nodes re-recorded, text-shape misses and font registrations since the
previous frame. A failed HWUI-versus-Compose metric is attributed with these
counters, so a round records their distribution rather than one number.

`dumpsys gfxinfo` supplies the two device-side figures the counters cannot:
the window's RenderNode count and the display-list memory they hold
(`metrics.frames.parse_render_nodes`).

Only the HWUI painter reports: the GPU painter draws through Cherenkov, whose
stage timing is the Perfetto trace's job, so its record names the gap rather
than carrying zeros.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

from . import adb, adb_out

# The counters `FrameStats::report` logs, in the order it logs them.
COUNTERS = (
    "jni_calls",
    "command_bytes",
    "nodes_rerecorded",
    "text_shape_misses",
    "font_registrations",
)

_LINE = re.compile(r"hydrolysis::frame_profile:.*?\bhwui frame\b(?P<fields>.*)$")
_FIELD = re.compile(r"(\w+)=(\d+)")


def clear_logcat(serial: str) -> None:
    """Empties every logcat buffer so a round counts only its own frames."""
    adb(serial, "logcat", "-c")


def parse_frames(text: str) -> list[dict[str, int]]:
    """One counter dict per `frame_profile` line in `text`."""
    frames = []
    for line in text.splitlines():
        match = _LINE.search(line)
        if match is None:
            continue
        fields = {k: int(v) for k, v in _FIELD.findall(match.group("fields"))}
        missing = [c for c in COUNTERS if c not in fields]
        if missing:
            raise ValueError(
                f"frame_profile line lacks {missing}: {line.strip()!r}"
            )
        frames.append({c: fields[c] for c in COUNTERS})
    return frames


def _percentile(sorted_values: list[int], fraction: float) -> int:
    return sorted_values[min(len(sorted_values) - 1, int(len(sorted_values) * fraction))]


def summarize(frames: list[dict[str, int]]) -> dict:
    """Total, p50, p99 and max of each counter over the round's frames."""
    summary: dict = {"frame_profile_frames": len(frames)}
    for counter in COUNTERS:
        values = sorted(frame[counter] for frame in frames)
        summary[f"{counter}_total"] = sum(values)
        if values:
            summary[f"{counter}_p50"] = _percentile(values, 0.50)
            summary[f"{counter}_p99"] = _percentile(values, 0.99)
            summary[f"{counter}_max"] = values[-1]
    return summary


def collect(serial: str, painter: str, artifacts_dir: Path, name: str) -> dict:
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
    try:
        data = adb_out(serial, "logcat", "-d", "-s", "hydrolysis:I")
    except subprocess.CalledProcessError as error:
        return {"frame_profile": "unavailable", "frame_profile_reason": str(error)}
    text = data.decode("utf-8", "replace")
    (artifacts_dir / f"{name}.logcat.txt").write_text(text)
    frames = parse_frames(text)
    if not frames:
        return {
            "frame_profile": "absent",
            "frame_profile_reason": (
                "no hydrolysis::frame_profile line in logcat: the app was "
                "built without the `frame-profile` feature"
            ),
        }
    return {"frame_profile": "present", **summarize(frames)}

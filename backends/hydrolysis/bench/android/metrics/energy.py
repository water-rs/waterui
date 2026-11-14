"""Energy: ODPM rail integration over equal-duration scripts.

Section 6: report total joules/script, joules/presented frame, frame count
and missed frames — plus idle watts. Energy/frame is undefined when idle
produces no frames. System work (IME, composition) is included; ODPM is
system-wide on Pixel 6-class and newer physical devices, so results name the
rails sampled. On hardware without ODPM, `collect` reports the battery-level
fallback as uncalibrated (a coarse proxy, kept visible rather than faked).
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

from . import adb_out

ODPM_RAILS = "/sys/bus/iio/devices/iio:device*/energy_value"


def sample_rails(serial: str) -> dict[str, float]:
    """Cumulative microjoules per ODPM rail, or empty without ODPM."""
    try:
        out = adb_out(
            serial, "shell", f"cat {ODPM_RAILS} 2>/dev/null"
        ).decode("utf-8", "replace")
    except subprocess.CalledProcessError:
        return {}
    rails: dict[str, float] = {}
    # `cat` over a glob loses the rail names; read names alongside values.
    try:
        names = adb_out(
            serial,
            "shell",
            "cat /sys/bus/iio/devices/iio:device*/name 2>/dev/null",
        ).decode("utf-8", "replace")
    except subprocess.CalledProcessError:
        names = ""
    values = [v for v in out.split() if v.strip()]
    name_list = [n.strip() for n in names.splitlines() if n.strip()]
    for idx, value in enumerate(values):
        label = name_list[idx] if idx < len(name_list) else f"rail{idx}"
        try:
            rails[label] = float(value)
        except ValueError:
            continue
    return rails


def collect(
    serial: str,
    package: str,
    duration_s: float,
    frames_presented: int | None,
    frames_missed: int | None,
    artifacts_dir: Path,
    name: str,
    script_seconds: float | None = None,
) -> dict:
    """Integrate ODPM rails over `duration_s`; report joules and per-frame
    energy. `script_seconds` names the nominal script duration so a drifted
    window stays visible."""
    start = sample_rails(serial)
    metrics: dict = {"odpm_rails_present": bool(start)}
    if not start:
        # Coarse fallback: battery-level delta over the window. A full-run
        # proxy only — labeled, never silently substituted for rail joules.
        try:
            level_text = adb_out(
                serial, "shell", "dumpsys", "battery"
            ).decode("utf-8", "replace")
        except subprocess.CalledProcessError:
            return metrics
        metrics["battery_dump"] = level_text[:2000]
        metrics["energy_source"] = "battery_level_uncalibrated"
        return metrics

    import time

    deadline = time.monotonic() + duration_s
    while time.monotonic() < deadline:
        time.sleep(min(1.0, deadline - time.monotonic()))
    end = sample_rails(serial)
    total_uj = sum(end.get(k, 0) - start.get(k, 0) for k in end)
    metrics["energy_source"] = "odpm"
    metrics["joules_script"] = round(total_uj / 1e6, 6)
    metrics["script_seconds_nominal"] = script_seconds or duration_s
    metrics["per_rail_uj"] = {
        k: round(end[k] - start.get(k, 0)) for k in end
    }
    if frames_presented:
        metrics["joules_per_presented_frame"] = round(
            total_uj / 1e6 / frames_presented, 9
        )
    metrics["frames_presented"] = frames_presented
    metrics["frames_missed"] = frames_missed
    (artifacts_dir / f"{name}.energy.json").write_text(
        __import__("json").dumps(metrics, indent=2)
    )
    return metrics

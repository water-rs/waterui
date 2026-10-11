"""Memory: PSS and GPU memory at the four section-6 sample points.

Sample points are first-content peak, settled idle, steady interaction and
script peak; the expensive `dumpsys meminfo` runs in a SEPARATE replay so it
never manufactures a frame hitch. Embedded subprocesses are included
consistently; shared/system allocations are reported separately.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

from . import adb_out


def parse_meminfo(text: str) -> dict:
    """Headline fields of `dumpsys meminfo <pkg>`, in kilobytes."""
    metrics: dict = {}
    match = re.search(
        r"TOTAL PSS:\s*(\d+)\s+TOTAL RSS:\s*(\d+)\s+TOTAL SWAP PSS:\s*(\d+)",
        text,
    )
    if match:
        metrics["total_pss_kb"] = int(match.group(1))
        metrics["total_rss_kb"] = int(match.group(2))
        metrics["swap_pss_kb"] = int(match.group(3))
    for key, label in (
        ("java_heap_pss_kb", "Java Heap"),
        ("native_heap_pss_kb", "Native Heap"),
        ("graphics_pss_kb", "Graphics"),
        ("private_other_pss_kb", "Private Other"),
        ("system_pss_kb", "System"),
    ):
        match = re.search(rf"^\s*{label}:\s*(\d+)", text, re.MULTILINE)
        if match:
            metrics[key] = int(match.group(1))
    return metrics


def collect_at(
    serial: str, package: str, point: str, artifacts_dir: Path, name: str
) -> dict:
    """`dumpsys meminfo` at one section-6 sample point.

    `point` is one of first_content_peak | settled_idle | steady_interaction
    | script_peak — recorded on the result record so a replay round's samples
    stay labeled by when they were taken.
    """
    try:
        data = adb_out(serial, "shell", "dumpsys", "meminfo", package)
    except subprocess.CalledProcessError:
        return {}
    text = data.decode("utf-8", "replace")
    (artifacts_dir / f"{name}.{point}.meminfo.txt").write_text(text)
    metrics = parse_meminfo(text)
    metrics["sample_point"] = point
    return metrics


def collect_proc_tree_pss(serial: str, package: str) -> dict:
    """PSS summed over the package's whole process tree — embedded
    subprocesses count consistently, per section 6."""
    try:
        ps = adb_out(serial, "shell", "ps", "-A", "-o", "PID,NAME").decode(
            "utf-8", "replace"
        )
    except subprocess.CalledProcessError:
        return {}
    total = 0
    seen = 0
    for line in ps.splitlines()[1:]:
        fields = line.split()
        if len(fields) != 2 or not fields[1].startswith(package):
            continue
        try:
            out = adb_out(
                serial,
                "shell",
                "dumpsys",
                "meminfo",
                fields[0],
            ).decode("utf-8", "replace")
        except subprocess.CalledProcessError:
            continue
        m = re.search(r"TOTAL PSS:\s*(\d+)", out)
        if m:
            total += int(m.group(1))
            seen += 1
    return {"process_tree_pss_kb": total, "process_count": seen} if seen else {}

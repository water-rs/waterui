"""Package size: stripped production artifact measurement.

Section 6 requires measuring the stripped production artifact (not the
debug/unsigned intermediate) and comparing package bytes with zero
tolerance. `collect` reports total bytes plus a per-entry breakdown so a
regression is attributable (dex / resources / native libs / assets).
"""

from __future__ import annotations

import zipfile
from pathlib import Path


def collect(apk: Path) -> dict:
    """Byte accounting for a built APK/AAB entry file."""
    if not apk.exists():
        return {"error": f"artifact missing: {apk}"}
    metrics: dict = {
        "artifact": apk.name,
        "total_bytes": apk.stat().st_size,
    }
    if zipfile.is_zipfile(apk):
        groups = {
            "dex": 0,
            "native_libs": 0,
            "resources": 0,
            "assets": 0,
            "kotlin_meta": 0,
            "other": 0,
        }
        with zipfile.ZipFile(apk) as archive:
            for info in archive.infolist():
                name = info.filename
                size = info.file_size
                if name.startswith("classes") and name.endswith(".dex"):
                    groups["dex"] += size
                elif name.startswith("lib/"):
                    groups["native_libs"] += size
                elif name == "resources.arsc" or name.startswith("res/"):
                    groups["resources"] += size
                elif name.startswith("assets/"):
                    groups["assets"] += size
                elif name.endswith(".kotlin_metadata"):
                    groups["kotlin_meta"] += size
                else:
                    groups["other"] += size
        metrics["uncompressed_breakdown_bytes"] = groups
    return metrics

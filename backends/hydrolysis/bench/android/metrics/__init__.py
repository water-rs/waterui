"""Shared plumbing for the bench/android metric collectors.

Every collector emits per-round records in the shared result schema the
plan's section 6 requires — each record identifies backend, painter,
app/screen/script, every source/build revision, the environment, and the
round index, then carries the metric's own samples.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path

# tomllib is 3.11+; the harness runs on 3.10 in CI images, so the tomli
# backport is aliased before the frozen e2e helpers are imported.
try:
    import tomllib  # type: ignore[import-not-found]
except ModuleNotFoundError:  # pragma: no cover - host python < 3.11
    import tomli

    tomllib = tomli  # type: ignore[no-redef]
    sys.modules.setdefault("tomllib", tomli)

BENCH_DIR = Path(__file__).resolve().parent.parent


def load_suite(path: Path | None = None) -> dict:
    """The frozen fixture inventory."""
    suite_path = path or BENCH_DIR / "suite.toml"
    with open(suite_path, "rb") as handle:
        return tomllib.load(handle)


def load_lock(path: Path | None = None) -> dict:
    lock_path = path or BENCH_DIR / "toolchain-lock.json"
    return json.loads(lock_path.read_text())


def adb(serial: str, *args: str) -> None:
    subprocess.run(
        ["adb", "-s", serial, *args], check=True, capture_output=True
    )


def adb_out(serial: str, *args: str) -> bytes:
    return subprocess.run(
        ["adb", "-s", serial, *args],
        check=True,
        capture_output=True,
    ).stdout


def detect_serial() -> str:
    """The one attached emulator/device; refuses to guess when ambiguous."""
    out = adb_out_global("devices").decode()
    serials = [
        line.split()[0]
        for line in out.splitlines()[1:]
        if line.strip() and line.split()[1] == "device"
    ]
    if not serials:
        sys.exit("no Android device/emulator attached")
    if len(serials) > 1:
        sys.exit(
            "multiple devices attached; pass --serial: " + ", ".join(serials)
        )
    return serials[0]


def adb_out_global(*args: str) -> bytes:
    return subprocess.run(["adb", *args], check=True, capture_output=True).stdout


def environment_identity(serial: str) -> dict:
    """Device-side environment fields a result record carries."""

    def prop(name: str) -> str:
        try:
            return (
                adb_out(serial, "shell", "getprop", name).decode().strip()
            )
        except (subprocess.CalledProcessError, ValueError):
            return "unknown"

    return {
        "serial": serial,
        "device_model": prop("ro.product.model"),
        "device_build": prop("ro.build.fingerprint"),
        "sdk_level": prop("ro.build.version.sdk"),
        "abi": prop("ro.product.cpu.abi"),
        "kernel": prop("ro.kernel.version"),
    }


def make_result(
    *,
    backend: str,
    painter: str,
    fixture: str,
    script: str,
    round_index: int,
    revisions: dict,
    environment: dict,
    metrics: dict,
) -> dict:
    """One result record — the shared schema across backends and painters."""
    return {
        "schema": "bench/android/result@1",
        "backend": backend,
        "painter": painter,
        "fixture": fixture,
        "script": script,
        "round": round_index,
        "revisions": revisions,
        "environment": environment,
        "captured_at_unix_ms": int(time.time() * 1000),
        "metrics": metrics,
    }


def write_result(record: dict, out_dir: Path, name: str) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{name}.json"
    path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
    return path

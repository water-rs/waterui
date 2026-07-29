#!/usr/bin/env python3

import argparse
import json
import pathlib
import subprocess
import time


def process_tree(root: int) -> set[int]:
    discovered = set()
    pending = [root]
    while pending:
        process = pending.pop()
        if process in discovered:
            continue
        discovered.add(process)
        children = pathlib.Path(f"/proc/{process}/task/{process}/children")
        try:
            pending.extend(int(value) for value in children.read_text().split())
        except FileNotFoundError:
            pass
    return discovered


def resident_bytes(process: int) -> int:
    status = pathlib.Path(f"/proc/{process}/status")
    try:
        for line in status.read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
    except FileNotFoundError:
        pass
    return 0


def directory_bytes(root: pathlib.Path) -> int:
    return sum(
        path.stat().st_size
        for path in root.rglob("*")
        if path.is_file() and not path.is_symlink()
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", required=True, type=pathlib.Path)
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--metrics", required=True, type=pathlib.Path)
    parser.add_argument("--runtime", required=True, type=pathlib.Path)
    parser.add_argument("--snapshot", required=True, type=pathlib.Path)
    parser.add_argument("--timeout-seconds", required=True, type=int)
    arguments = parser.parse_args()

    identity = json.loads((arguments.runtime / "runtime.json").read_text())
    started = time.monotonic_ns()
    process = subprocess.Popen(
        [
            str(arguments.binary),
            str(arguments.runtime),
            str(arguments.snapshot),
            str(arguments.timeout_seconds),
        ]
    )
    peak_resident_bytes = 0
    peak_process_count = 0
    while process.poll() is None:
        processes = process_tree(process.pid)
        peak_resident_bytes = max(
            peak_resident_bytes,
            sum(resident_bytes(child) for child in processes),
        )
        peak_process_count = max(peak_process_count, len(processes))
        try:
            process.wait(timeout=0.01)
        except subprocess.TimeoutExpired:
            pass
    if process.returncode != 0:
        raise RuntimeError(
            f"WPE runtime smoke process exited with {process.returncode}"
        )
    if not arguments.snapshot.is_file():
        raise RuntimeError("WPE runtime smoke did not produce its GPU snapshot")

    metrics = {
        "schema_version": 1,
        "engine": identity["engine"],
        "version": identity["version"],
        "platform": identity["platform"],
        "architecture": identity["architecture"],
        "archive_bytes": arguments.archive.stat().st_size,
        "uncompressed_runtime_bytes": directory_bytes(arguments.runtime),
        "peak_process_tree_rss_bytes": peak_resident_bytes,
        "peak_process_count": peak_process_count,
        "duration_milliseconds": (time.monotonic_ns() - started) // 1_000_000,
        "snapshot": arguments.snapshot.name,
    }
    arguments.metrics.write_text(
        json.dumps(metrics, indent=2, sort_keys=True) + "\n"
    )


if __name__ == "__main__":
    main()

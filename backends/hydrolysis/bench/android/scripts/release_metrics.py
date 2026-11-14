#!/usr/bin/env python3
"""Package one example with `water package --release` and record the release
APK's size plus a per-component breakdown (native lib, dex, resources).

The nightly runs this once for a representative example so the R8 / LTO /
strip work has a number that trends instead of a one-off measurement.

Usage:
    python3 release_metrics.py --repo-root <waterui> [--example gesture]
        [--out release-metrics.json]
"""

import argparse
import json
import os
import re
import subprocess
import sys
import zipfile
from pathlib import Path

# Breakdown buckets, matched against zip entry names. Order matters — the
# first match wins.
BUCKETS = [
    ("libwaterui_app.so", re.compile(r"^lib/[^/]+/libwaterui_app\.so$")),
    ("libc++_shared.so", re.compile(r"^lib/[^/]+/libc\+\+_shared\.so$")),
    ("other native libs", re.compile(r"^lib/")),
    ("dex", re.compile(r"^classes[^/]*\.dex$")),
    ("resources.arsc", re.compile(r"^resources\.arsc$")),
    ("res/", re.compile(r"^res/")),
    ("assets/", re.compile(r"^assets/")),
    ("META-INF/", re.compile(r"^META-INF/")),
    ("kotlin metadata", re.compile(r"^kotlin/")),
]


def apk_breakdown(apk_path: Path) -> dict:
    buckets = {label: 0 for label, _ in BUCKETS}
    buckets["other"] = 0
    entry_count = 0
    with zipfile.ZipFile(apk_path) as archive:
        for info in archive.infolist():
            entry_count += 1
            # file_size is the uncompressed size; compress_size is what the
            # APK actually carries. Report the stored footprint.
            size = info.compress_size
            for label, pattern in BUCKETS:
                if pattern.search(info.filename):
                    buckets[label] += size
                    break
            else:
                buckets["other"] += size
    return {
        "entries": entry_count,
        "breakdown_bytes": {k: v for k, v in buckets.items() if v},
    }


def find_release_apk(package_dir: Path) -> Path | None:
    """Release APK in the example's package output directory.

    `water package` places the artifact at `<example>/target/package/`
    (water-rs/cli#129); that directory is the documented output location,
    not a cache to search by file age."""
    candidates = sorted(package_dir.glob("*.apk"))
    if not candidates:
        return None
    unsigned = [path for path in candidates if "release-unsigned" in path.name]
    return unsigned[0] if unsigned else candidates[0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", required=True, type=Path)
    parser.add_argument("--example", default="gesture")
    parser.add_argument("--arch", default="x86-64")
    parser.add_argument("--out", type=Path, default=Path("release-metrics.json"))
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    example_path = repo_root / "examples" / args.example
    if not example_path.is_dir():
        sys.exit(f"example not found: {example_path}")

    result = subprocess.run(
        [
            "water", "package",
            "--platform", "android",
            "--backend", "android",
            "--arch", args.arch,
            "--release",
            "--path", str(example_path),
        ],
        cwd=repo_root,
    )
    if result.returncode != 0:
        sys.exit(f"water package --release failed ({result.returncode})")

    apk = find_release_apk(example_path / "target" / "package")
    if apk is None:
        sys.exit("no release APK under target/package after packaging")

    metrics = {
        "kind": "release-package",
        "example": args.example,
        "apk": str(apk),
        "apk_bytes": apk.stat().st_size,
        **apk_breakdown(apk),
    }
    args.out.write_text(json.dumps(metrics, indent=2) + "\n")

    mb = metrics["apk_bytes"] / 1024 / 1024
    print(f"release APK: {apk}")
    print(f"total size: {mb:.2f} MB")
    for label, size in sorted(
        metrics["breakdown_bytes"].items(), key=lambda item: -item[1]
    ):
        print(f"  {label:<22} {size / 1024 / 1024:7.2f} MB")

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        lines = [
            f"### Release package size ({args.example})",
            "",
            "| | |",
            "|---|---|",
            f"| **APK total** | **{mb:.2f} MB** |",
        ]
        lines += [
            f"| {label} | {size / 1024 / 1024:.2f} MB |"
            for label, size in sorted(
                metrics["breakdown_bytes"].items(), key=lambda item: -item[1]
            )
        ]
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())

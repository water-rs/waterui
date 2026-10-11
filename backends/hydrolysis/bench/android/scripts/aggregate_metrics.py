#!/usr/bin/env python3
"""Merge per-shard example metrics and the release-package metrics into one
`nightly-metrics.json` plus a Markdown table for the run's step summary.

Usage:
    python3 aggregate_metrics.py --artifacts-root <dir> --out nightly-metrics.json

<dir> is the download-artifact merge target: every shard's `metrics-shard-N.json`
and the release job's `release-metrics.json` land anywhere underneath it.
"""

import argparse
import json
import os
import sys
from pathlib import Path


def collect(artifacts_root: Path) -> tuple[list[dict], dict | None]:
    examples: list[dict] = []
    release: dict | None = None
    for path in sorted(artifacts_root.rglob("metrics-shard-*.json")):
        try:
            entries = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        # Later shards can't duplicate an example; guard anyway so a
        # re-run directory merge keeps the newest copy.
        seen = {entry["example"] for entry in examples}
        examples.extend(e for e in entries if e.get("example") not in seen)
    for path in sorted(artifacts_root.rglob("release-metrics.json")):
        try:
            release = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
    examples.sort(key=lambda entry: entry.get("example", ""))
    return examples, release


def fmt_mb(kb: int | None) -> str:
    return f"{kb / 1024:.1f}" if kb else "-"


def fmt_bytes(value: int | None) -> str:
    return f"{value / 1024 / 1024:.1f}" if value else "-"


def fmt_ms(value: int | None) -> str:
    return str(value) if value else "-"


def fmt_pct(value: float | None) -> str:
    return f"{value:g}" if value is not None else "-"


def render_markdown(examples: list[dict], release: dict | None) -> str:
    lines = ["## Nightly device metrics", ""]
    if release:
        mb = release.get("apk_bytes", 0) / 1024 / 1024
        lines += [
            f"**Release APK ({release.get('example', '?')})**: {mb:.2f} MB",
            "",
            "| component | MB |",
            "|---|---|",
        ]
        lines += [
            f"| {label} | {size / 1024 / 1024:.2f} |"
            for label, size in sorted(
                release.get("breakdown_bytes", {}).items(), key=lambda i: -i[1]
            )
        ]
        lines.append("")
    if examples:
        lines += [
            "| example | status | APK (MB) | root ready (ms) | displayed (ms) | "
            "first frame (ms) | settle (ms) | PSS (MB) | RSS peak (MB) | CPU % | "
            "frames | janky | frame p90 (ms) |",
            "|---|---|---|---|---|---|---|---|---|---|---|---|---|",
        ]
        lines += [
            "| {e} | {s} | {a} | {rr} | {d} | {f} | {se} | {p} | {r} | {c} | "
            "{fr} | {j} | {p90} |".format(
                e=entry.get("example", "?"),
                s=entry.get("status", "?"),
                a=fmt_bytes(entry.get("apk_bytes")),
                rr=fmt_ms(entry.get("root_ready_ms")),
                d=fmt_ms(entry.get("displayed_ms")),
                f=fmt_ms(entry.get("launch_to_first_frame_ms")),
                se=fmt_ms(entry.get("settle_ms")),
                p=fmt_mb(entry.get("total_pss_kb")),
                r=fmt_mb(entry.get("rss_peak_kb") or entry.get("total_rss_kb")),
                c=fmt_pct(entry.get("cpu_pct")),
                fr=fmt_ms(entry.get("frames_total")),
                j=fmt_ms(entry.get("janky_frames")),
                p90=fmt_ms(entry.get("frame_ms_p90")),
            )
            for entry in examples
        ]
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts-root", required=True, type=Path)
    parser.add_argument("--out", type=Path, default=Path("nightly-metrics.json"))
    args = parser.parse_args()

    examples, release = collect(args.artifacts_root)
    payload = {"examples": examples, "release": release}
    args.out.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"merged {len(examples)} example metrics + "
          f"{'1' if release else 'no'} release record -> {args.out}")

    markdown = render_markdown(examples, release)
    sys.stdout.write(markdown)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(markdown)
    return 0


if __name__ == "__main__":
    sys.exit(main())

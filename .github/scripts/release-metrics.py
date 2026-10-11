#!/usr/bin/env python3
"""Metric records and reports for check-package-size.sh.

Every structured value the release-metrics job emits goes through this
helper — JSONL records, the report document, the recorded baseline — so
arbitrary paths and values survive verbatim and no whitespace-separated or
hand-built JSON exists anywhere in the pipeline.

Subcommands:

  inspect-app     Validate the .app a `water package` run reported and
                  append one measured record to --out: the bundle path,
                  its CFBundleIdentifier, and the byte sizes of the bundle
                  and of the executable CFBundleExecutable names. A
                  missing bundle, an unparsable Info.plist, or an
                  executable that escapes the bundle is rejected — the
                  measured artifact must be the packaged one, never a
                  shape another process could have left behind.

  record-runtime  Append one runtime record to --out: the first_paint_ms /
                  peak_rss_bytes a measure-native-launch.py --metrics-json
                  report produced, or explicit nulls when no report exists
                  (launch failure, or no simulator for the platform).

  report          Render release-metrics.json + release-metrics.md into
                  --metrics-dir, then either write a RECORD baseline
                  (--record), or gate the hello-world byte sizes against
                  the recorded baseline (--baseline; >5% growth fails
                  nonzero, a missing file or platform entry warns and
                  reports without gating).
"""

from __future__ import annotations

import argparse
import json
import plistlib
import stat
import sys
from pathlib import Path

# The byte gate applies to hello-world only — it is the stable minimal-app
# signal. Example subjects would be recorded for visibility, not gated.
GATE_LABEL = "helloworld"
# Growth of more than baseline + baseline // GATE_DIVISOR (5%) fails.
GATE_DIVISOR = 20


class Failure(Exception):
    """A malformed artifact, record, or report input."""


def _read_jsonl(path):
    records = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            records.append(json.loads(line))
    return records


def _append_jsonl(path, record):
    with path.open("a", encoding="utf-8") as out:
        out.write(json.dumps(record) + "\n")


def inspect_app(args):
    app = Path(args.app)
    if app.suffix != ".app" or not app.is_dir():
        raise Failure(f"reported artifact is not a packaged .app: {app}")
    if args.platform == "macos":
        plist_path = app / "Contents" / "Info.plist"
        executable_dir = app / "Contents" / "MacOS"
    else:
        plist_path = app / "Info.plist"
        executable_dir = app
    try:
        info = plistlib.loads(plist_path.read_bytes())
    except (OSError, ValueError) as exc:
        raise Failure(f"cannot parse {plist_path}: {exc}") from exc
    executable_name = info.get("CFBundleExecutable")
    if not isinstance(executable_name, str) or not executable_name:
        raise Failure(f"{plist_path} declares no CFBundleExecutable")
    bundle_id = info.get("CFBundleIdentifier")
    if not isinstance(bundle_id, str) or not bundle_id:
        raise Failure(f"{plist_path} declares no CFBundleIdentifier")
    executable = (executable_dir / executable_name).resolve()
    if not executable.is_relative_to(app.resolve()):
        raise Failure(
            f"CFBundleExecutable escapes the bundle: {executable_name!r}")
    if not executable.is_file():
        raise Failure(f"bundle executable is missing: {executable}")
    # `find -type f -exec stat -f%z` equivalent: regular files only,
    # symlinks excluded by lstat.
    app_bytes = sum(
        entry.lstat().st_size
        for entry in app.rglob("*")
        if stat.S_ISREG(entry.lstat().st_mode))
    executable_bytes = executable.stat().st_size
    _append_jsonl(args.out, {
        "label": args.label,
        "platform": args.platform,
        "app_bytes": app_bytes,
        "executable_bytes": executable_bytes,
        "app_path": str(app),
        "executable": str(executable),
        "bundle_id": bundle_id,
    })
    print(f"{args.label}/{args.platform}: app={app_bytes}B "
          f"executable={executable_bytes}B ({app})")


def record_runtime(args):
    report = {"first_paint_ms": None, "peak_rss_bytes": None}
    if args.report is not None:
        try:
            data = json.loads(Path(args.report).read_text(encoding="utf-8"))
        except (OSError, ValueError) as exc:
            raise Failure(
                f"cannot parse metrics report {args.report}: {exc}") from exc
        if not isinstance(data, dict):
            raise Failure(
                f"metrics report is not a JSON object: {args.report}")
        for key in report:
            if key in data:
                report[key] = data[key]
    _append_jsonl(args.out, {
        "label": args.label,
        "platform": args.platform,
        **report,
    })
    first_paint = report["first_paint_ms"]
    peak_rss = report["peak_rss_bytes"]
    print(f"{args.label}/{args.platform}: "
          f"first_paint={first_paint if first_paint is not None else 'null'}ms "
          f"peak_rss={peak_rss if peak_rss is not None else 'null'}B")


def _markdown(rows):
    def mib(byte_count):
        return f"{byte_count / 1048576:.1f} MB"

    lines = [
        "## Release metrics (production builds)",
        "",
        "| App | Platform | .app size | Executable | First paint | Peak RSS |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for measured, entry in rows:
        first_paint = entry["first_paint_ms"]
        peak_rss = entry["peak_rss_bytes"]
        lines.append(
            f"| `{measured['label']}` | `{measured['platform']}` | "
            f"{mib(measured['app_bytes'])} | "
            f"{mib(measured['executable_bytes'])} | "
            f"{'—' if first_paint is None else f'{first_paint} ms'} | "
            f"{'—' if peak_rss is None else mib(peak_rss)} |")
    return "\n".join(lines) + "\n"


def _trunc_div(numerator, denominator):
    """Integer division truncated toward zero, the way bash `/` behaves."""
    quotient = abs(numerator) // abs(denominator)
    return -quotient if (numerator < 0) != (denominator < 0) else quotient


def _format_growth(growth_x100):
    """'+1.25%' / '-67.87%' from hundredths of a percent."""
    if growth_x100 < 0:
        sign, growth_x100 = "-", -growth_x100
    else:
        sign = "+"
    return f"{sign}{growth_x100 // 100}.{growth_x100 % 100:02d}%"


def _check_metric(platform, metric, measured, baseline):
    limit = baseline + baseline // GATE_DIVISOR
    growth_x100 = _trunc_div((measured - baseline) * 10000, baseline)
    if measured > limit:
        print(f"::error::{platform} {metric} grew "
              f"{_format_growth(growth_x100)} ({baseline} -> {measured} "
              f"bytes, allowed +5%)")
        return False
    print(f"{platform} {metric}: {measured} bytes "
          f"(baseline {baseline}, {_format_growth(growth_x100)})")
    return True


def report(args):
    measured = _read_jsonl(args.measured)
    runtime = {}
    if args.runtime.exists():
        runtime = {
            (record["label"], record["platform"]): record
            for record in _read_jsonl(args.runtime)
        }
    metrics = {}
    rows = []
    for record in measured:
        observed = runtime.get((record["label"], record["platform"]), {})
        entry = {
            "app_bytes": record["app_bytes"],
            "executable_bytes": record["executable_bytes"],
            "first_paint_ms": observed.get("first_paint_ms"),
            "peak_rss_bytes": observed.get("peak_rss_bytes"),
        }
        metrics.setdefault(record["label"], {})[record["platform"]] = entry
        rows.append((record, entry))
    args.metrics_dir.mkdir(parents=True, exist_ok=True)
    (args.metrics_dir / "release-metrics.json").write_text(
        json.dumps(metrics, indent=2) + "\n", encoding="utf-8")
    markdown = _markdown(rows)
    (args.metrics_dir / "release-metrics.md").write_text(
        markdown, encoding="utf-8")
    sys.stdout.write(markdown)

    if args.record is not None:
        args.record.parent.mkdir(parents=True, exist_ok=True)
        baseline = {
            record["platform"]: {
                "app_bytes": record["app_bytes"],
                "executable_bytes": record["executable_bytes"],
            }
            for record, _entry in rows
            if record["label"] == GATE_LABEL
        }
        args.record.write_text(
            json.dumps(baseline, indent=2) + "\n", encoding="utf-8")
        print(f"Recorded size baseline -> {args.record}")
        sys.stdout.write(args.record.read_text(encoding="utf-8"))
        return 0
    if not args.baseline.exists():
        print(f"::warning::No size baseline at {args.baseline}; "
              "measurements reported but not gated")
        return 0
    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    gate_ok = True
    for record, _entry in rows:
        if record["label"] != GATE_LABEL:
            continue
        platform_baseline = baseline.get(record["platform"])
        if not isinstance(platform_baseline, dict):
            print(f"::warning::No baseline entry for {record['platform']}; "
                  "skipping gate")
            continue
        for metric, key in (("app", "app_bytes"),
                            ("executable", "executable_bytes")):
            if key not in platform_baseline:
                print(f"::warning::No baseline entry for "
                      f"{record['platform']}; skipping gate")
                break
            gate_ok &= _check_metric(record["platform"], metric,
                                     record[key], platform_baseline[key])
    return 0 if gate_ok else 1


def main():
    parser = argparse.ArgumentParser(
        prog="release-metrics.py",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(
        dest="command", required=True, metavar="command")

    inspect = sub.add_parser(
        "inspect-app",
        help="validate a reported .app and append its measured record")
    inspect.add_argument("--label", required=True)
    inspect.add_argument(
        "--platform", required=True, choices=["macos", "ios-simulator"])
    inspect.add_argument("--app", required=True, type=Path)
    inspect.add_argument("--out", required=True, type=Path)
    inspect.set_defaults(func=inspect_app)

    runtime = sub.add_parser(
        "record-runtime",
        help="append a first-paint/peak-RSS record (nulls without a report)")
    runtime.add_argument("--label", required=True)
    runtime.add_argument("--platform", required=True)
    runtime.add_argument("--report", type=Path, default=None)
    runtime.add_argument("--out", required=True, type=Path)
    runtime.set_defaults(func=record_runtime)

    render = sub.add_parser(
        "report",
        help="render release-metrics.{json,md} and gate or record baselines")
    render.add_argument("--measured", required=True, type=Path)
    render.add_argument("--runtime", required=True, type=Path)
    render.add_argument("--metrics-dir", required=True, type=Path)
    mode = render.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--record", type=Path,
        help="write the hello-world size baseline here and skip the gate")
    mode.add_argument(
        "--baseline", type=Path,
        help="gate hello-world byte sizes against this baseline file")
    render.set_defaults(func=report)

    args = parser.parse_args()
    try:
        sys.exit(args.func(args) or 0)
    except Failure as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()

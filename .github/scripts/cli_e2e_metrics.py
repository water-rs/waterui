#!/usr/bin/env python3
"""Cold-compile metrics for the nightly fresh-user CLI e2e (`cli-e2e`).

Each `cli-e2e` leg times its phases into a tab-separated `timings.tsv`
(`<metric>\\t<seconds>` per line, written as each phase succeeds); this
helper turns that into one JSON record per leg, renders the nightly's step
summary from the legs' records plus the stored history, and appends the
night to that history.

Subcommands:

  record     Build one leg's record from --timings (the TSV) and the
             runner's environment (ImageOS/ImageVersion, RUNNER_*,
             GITHUB_*), plus the fresh user's `rustc -vV` output from
             --rustc. Writes JSON to --out. A missing TSV still yields a
             record — with no metrics — so a leg that failed before its
             first timed phase shows up as such rather than vanishing.

  summarize  Read every *.json record in --records and the JSON-lines
             history in --history (missing = empty), print the Markdown
             step summary (one table per platform: tonight's seconds, the
             delta against the previous stored night, the 7- and 30-night
             medians) to stdout, and — with --out-dir — write the updated
             `history.jsonl` (tonight's records appended) and the flattened
             `history.csv` there.

The history lives on the orphan `ci-metrics` branch under `cli-e2e/`; the
nightly's `cli-e2e-metrics` job fetches it, runs `summarize`, and pushes
the result back (on `dev` only).
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import os
import statistics
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCHEMA = 1

# Display order and labels. Anything a leg records outside this list is
# still stored and shown, after these, under its raw key.
METRICS: list[tuple[str, str]] = [
    ("cli_install", "CLI install (`cargo install --path cli`)"),
    ("doctor_fix", "`water doctor --fix` (toolchain repair)"),
    ("doctor_fix_android", "`water doctor --fix` (Android project)"),
    ("create", "`water create`"),
    ("cold_build", "Cold app build (`water build`)"),
    ("package", "`water package`"),
    ("package_debug", "`water package --debug` (APK)"),
    ("package_release", "`water package --release` (APK)"),
    ("console_build", "Console-user build (`water build`, macOS)"),
    ("launch", "Launch (`water run` → 'Application started')"),
]
METRIC_KEYS = [key for key, _ in METRICS]

PLATFORM_ORDER = ["macos", "windows", "linux", "android"]
PLATFORM_LABELS = {
    "macos": "macOS",
    "windows": "Windows",
    "linux": "Ubuntu (Linux)",
    "android": "Android",
}

CSV_FIELDS = [
    "date",
    "run_id",
    "event",
    "sha",
    "platform",
    "runner",
    "image",
    "rustc",
    "status",
    "cold_verified",
] + METRIC_KEYS


# --------------------------------------------------------------------------
# record


def parse_timings(text: str) -> dict[str, int]:
    """`<metric>\\t<seconds>` lines → {metric: seconds}; a repeated key keeps
    its last value, blank and malformed lines are skipped."""
    metrics: dict[str, int] = {}
    for line in text.splitlines():
        parts = line.strip().split("\t")
        if len(parts) < 2 or not parts[0]:
            continue
        try:
            metrics[parts[0].strip()] = int(float(parts[1]))
        except ValueError:
            continue
    return metrics


def parse_rustc(text: str) -> str | None:
    """The `rustc X.Y.Z (hash date)` line of `rustc -vV` output."""
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("rustc "):
            return line
    return None


def parse_flags(text: str) -> dict[str, str]:
    """`key=value` lines (the leg's coldness checks)."""
    flags: dict[str, str] = {}
    for line in text.splitlines():
        key, sep, value = line.strip().partition("=")
        if sep and key:
            flags[key.strip()] = value.strip()
    return flags


def read_optional(path: str | None) -> str:
    if not path:
        return ""
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def build_record(args: argparse.Namespace, env: dict[str, str], now: datetime) -> dict:
    flags = parse_flags(read_optional(args.flags))
    cold = flags.get("cold_build_verified")
    image = " ".join(
        part for part in (env.get("ImageOS", ""), env.get("ImageVersion", "")) if part
    )
    return {
        "schema": SCHEMA,
        "date": now.strftime("%Y-%m-%d"),
        "recorded_at": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "run_id": env.get("GITHUB_RUN_ID", ""),
        "run_attempt": env.get("GITHUB_RUN_ATTEMPT", ""),
        "event": env.get("GITHUB_EVENT_NAME", ""),
        "ref": env.get("GITHUB_REF", ""),
        "sha": env.get("GITHUB_SHA", ""),
        "platform": args.platform,
        "runner": args.runner or env.get("RUNNER_OS", ""),
        "runner_arch": env.get("RUNNER_ARCH", ""),
        "image": image or None,
        "rustc": parse_rustc(read_optional(args.rustc)),
        "status": args.status,
        "cold_verified": None if cold is None else cold == "true",
        "cli_install_cold": (
            None
            if "cli_install_cold" not in flags
            else flags["cli_install_cold"] == "true"
        ),
        "metrics": parse_timings(read_optional(args.timings)),
    }


def cmd_record(args: argparse.Namespace) -> int:
    record = build_record(args, dict(os.environ), datetime.now(timezone.utc))
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(record, sort_keys=True))
    return 0


# --------------------------------------------------------------------------
# summarize


def load_history(path: str | None) -> list[dict]:
    records: list[dict] = []
    for line in read_optional(path).splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(record, dict) and isinstance(record.get("metrics"), dict):
            records.append(record)
    return records


def load_records(directory: str) -> list[dict]:
    records = []
    root = Path(directory)
    if not root.is_dir():
        return records
    for path in sorted(root.rglob("*.json")):
        try:
            record = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(record, dict) and isinstance(record.get("metrics"), dict):
            records.append(record)
    return records


def fmt_duration(seconds: float | None) -> str:
    if seconds is None:
        return "—"
    seconds = round(seconds)
    if seconds < 60:
        return f"{seconds}s"
    minutes, secs = divmod(seconds, 60)
    if minutes < 60:
        return f"{minutes}m {secs:02d}s"
    hours, minutes = divmod(minutes, 60)
    return f"{hours}h {minutes:02d}m {secs:02d}s"


def fmt_delta(current: int, previous: int | None) -> str:
    if previous is None:
        return "—"
    delta = current - previous
    if delta == 0:
        return "±0s"
    sign = "+" if delta > 0 else "−"
    pct = f" ({sign}{abs(delta) / previous * 100:.1f}%)" if previous else ""
    return f"{sign}{fmt_duration(abs(delta))}{pct}"


def parse_date(value: str) -> datetime | None:
    try:
        return datetime.strptime(value, "%Y-%m-%d").replace(tzinfo=timezone.utc)
    except (TypeError, ValueError):
        return None


def history_for(history: list[dict], platform: str, run_id: str) -> list[dict]:
    """Earlier stored records of one platform, oldest first; tonight's own
    run (a re-run of the summarize step) is excluded."""
    rows = [
        r
        for r in history
        if r.get("platform") == platform and str(r.get("run_id", "")) != str(run_id)
    ]
    rows.sort(key=lambda r: (r.get("recorded_at") or r.get("date") or ""))
    return rows


def median_since(rows: list[dict], key: str, since: datetime) -> tuple[float | None, int]:
    values = []
    for row in rows:
        when = parse_date(row.get("date", ""))
        value = row.get("metrics", {}).get(key)
        if when is not None and when >= since and isinstance(value, (int, float)):
            values.append(value)
    if not values:
        return None, 0
    return statistics.median(values), len(values)


def previous_value(rows: list[dict], key: str) -> int | None:
    for row in reversed(rows):
        value = row.get("metrics", {}).get(key)
        if isinstance(value, (int, float)):
            return int(value)
    return None


def ordered_metric_keys(metrics: dict) -> list[str]:
    known = [key for key in METRIC_KEYS if key in metrics]
    extra = sorted(key for key in metrics if key not in METRIC_KEYS)
    return known + extra


def platform_sort_key(record: dict) -> tuple[int, str]:
    platform = record.get("platform", "")
    try:
        return PLATFORM_ORDER.index(platform), platform
    except ValueError:
        return len(PLATFORM_ORDER), platform


def render_summary(
    records: list[dict],
    history: list[dict],
    *,
    history_url: str | None = None,
    artifacts_url: str | None = None,
    now: datetime | None = None,
) -> str:
    now = now or datetime.now(timezone.utc)
    labels = dict(METRICS)
    out = io.StringIO()
    out.write("## CLI fresh-user cold-compile metrics\n\n")
    if not records:
        out.write(
            "No `cli-e2e` leg uploaded a timing record "
            "(every leg failed before its first timed phase, or was skipped).\n"
        )
        return out.getvalue()
    sha = next((r.get("sha") for r in records if r.get("sha")), "")
    out.write(
        f"Commit `{sha[:12]}`. Wall-clock seconds. Every `water` phase runs as a brand-new "
        "OS user (empty `~/.water` build cache, empty `~/.cargo` registry, toolchain "
        "installed by `water doctor --fix`) with no Actions cache and no compiler wrapper; "
        "the CLI install runs on the runner account. Δ compares with the previous stored "
        "night; medians cover the stored nights before this one.\n\n"
    )
    week = now - timedelta(days=7)
    month = now - timedelta(days=30)
    for record in sorted(records, key=platform_sort_key):
        platform = record.get("platform", "?")
        rows = history_for(history, platform, record.get("run_id", ""))
        details = ", ".join(
            part
            for part in (
                record.get("runner") or None,
                record.get("image") or None,
                record.get("rustc") or None,
            )
            if part
        )
        status = record.get("status") or "unknown"
        out.write(f"### {PLATFORM_LABELS.get(platform, platform)}\n\n")
        meta = f"{details} — leg {status}" if details else f"leg {status}"
        out.write(f"{meta}\n\n")
        if record.get("cold_verified") is False:
            out.write(
                "> [!WARNING]\n> The build cache was not empty before the cold build: "
                "`cold_build` is not a cold number tonight.\n\n"
            )
        if record.get("cli_install_cold") is False:
            out.write(
                "> [!NOTE]\n> `cli_install` reused a restored `target/` (Actions cache): "
                "it is not a cold number.\n\n"
            )
        metrics = record.get("metrics", {})
        if not metrics:
            out.write("_No phase completed._\n\n")
            continue
        out.write("| Phase | Tonight | Δ vs previous | 7-night median | 30-night median |\n")
        out.write("|---|---:|---:|---:|---:|\n")
        for key in ordered_metric_keys(metrics):
            value = int(metrics[key])
            week_median, week_n = median_since(rows, key, week)
            month_median, month_n = median_since(rows, key, month)
            out.write(
                f"| {labels.get(key, f'`{key}`')} | {fmt_duration(value)} "
                f"| {fmt_delta(value, previous_value(rows, key))} "
                f"| {fmt_duration(week_median)}{f' (n={week_n})' if week_n else ''} "
                f"| {fmt_duration(month_median)}{f' (n={month_n})' if month_n else ''} |\n"
            )
        out.write("\n")
    out.write(
        "`cargo --timings` reports for the cold build (and `water package`, and the CLI "
        "install) are in each leg's `cli-e2e-<platform>-logs` artifact under "
        "`cargo-timings/`"
        + (f" ([run artifacts]({artifacts_url}))" if artifacts_url else "")
        + ".\n"
    )
    if history_url:
        out.write(
            f"\nHistory: [`ci-metrics` branch, `cli-e2e/history.csv`]({history_url}) "
            "(one row per leg per night).\n"
        )
    return out.getvalue()


def to_csv(history: list[dict]) -> str:
    buffer = io.StringIO()
    writer = csv.DictWriter(buffer, fieldnames=CSV_FIELDS, extrasaction="ignore", lineterminator="\n")
    writer.writeheader()
    for record in history:
        row = {field: record.get(field, "") for field in CSV_FIELDS if field not in METRIC_KEYS}
        if row.get("cold_verified") is None:
            row["cold_verified"] = ""
        for key in METRIC_KEYS:
            row[key] = record.get("metrics", {}).get(key, "")
        writer.writerow(row)
    return buffer.getvalue()


def merge_history(history: list[dict], records: list[dict]) -> list[dict]:
    """Append tonight's records, replacing any stored record of the same
    (run_id, platform) — a re-run of the job must not duplicate a night."""
    tonight = {(str(r.get("run_id", "")), r.get("platform")) for r in records}
    kept = [
        r for r in history if (str(r.get("run_id", "")), r.get("platform")) not in tonight
    ]
    return kept + sorted(records, key=platform_sort_key)


HISTORY_README = """# Nightly CLI cold-compile metrics

Written by the `cli-e2e-metrics` job of `.github/workflows/nightly.yml` on
`dev`; do not edit by hand. Every nightly appends one record per `cli-e2e`
leg (macOS, Windows, Ubuntu/Linux, Android).

- `history.jsonl` — the source of truth: one JSON object per leg per night
  with `date`, `run_id`, `event`, `sha`, `platform`, `runner`, `image`
  (runner image OS + version), `rustc`, `status`, `cold_verified`,
  `cli_install_cold` and `metrics` (seconds per phase).
- `history.csv` — the same records flattened, one column per phase, for a
  spreadsheet or `python -m csv`-style tooling.

Phases (wall-clock seconds, fresh OS user, no Actions cache, no compiler
wrapper): `cli_install` (`cargo install --path cli` on the runner account),
`doctor_fix`, `doctor_fix_android`, `create`, `cold_build` (the first
`water build` of the scaffolded app — the cold compile), `package`,
`package_debug` / `package_release` (Android APKs), `console_build` (macOS:
the console user's own `water build`, needed because its build cache is
separate), `launch` (`water run` start to 'Application started', after the
build). A missing value means the phase does not exist on that platform or
did not complete that night.

Quick look at one platform's cold builds:

    git show origin/ci-metrics:cli-e2e/history.csv | grep ',linux,' | cut -d, -f1,4,8,15
"""


def cmd_summarize(args: argparse.Namespace) -> int:
    records = load_records(args.records)
    history = load_history(args.history)
    sys.stdout.write(
        render_summary(
            records,
            history,
            history_url=args.history_url,
            artifacts_url=args.artifacts_url,
        )
    )
    if args.out_dir:
        out_dir = Path(args.out_dir)
        out_dir.mkdir(parents=True, exist_ok=True)
        merged = merge_history(history, records)
        (out_dir / "history.jsonl").write_text(
            "".join(json.dumps(r, sort_keys=True) + "\n" for r in merged), encoding="utf-8"
        )
        (out_dir / "history.csv").write_text(to_csv(merged), encoding="utf-8")
        (out_dir / "README.md").write_text(HISTORY_README, encoding="utf-8")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    record = sub.add_parser("record", help="build one leg's JSON record")
    record.add_argument("--platform", required=True)
    record.add_argument("--runner", default="", help="the runs-on label, e.g. xcode-27")
    record.add_argument("--status", default="unknown", help="the job status at record time")
    record.add_argument("--timings", help="the leg's timings.tsv")
    record.add_argument("--rustc", help="the fresh user's `rustc -vV` output")
    record.add_argument("--flags", help="key=value coldness checks")
    record.add_argument("--out", required=True)
    record.set_defaults(func=cmd_record)

    summarize = sub.add_parser("summarize", help="render the step summary, update history")
    summarize.add_argument("--records", required=True, help="directory of leg records")
    summarize.add_argument("--history", help="stored history.jsonl (may be missing)")
    summarize.add_argument("--out-dir", help="write updated history.jsonl/history.csv here")
    summarize.add_argument("--history-url")
    summarize.add_argument("--artifacts-url")
    summarize.set_defaults(func=cmd_summarize)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())

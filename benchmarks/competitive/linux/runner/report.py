#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# ///
"""Render a competitive-benchmark results JSON into markdown tables
(water-rs/waterui#1262). Contestants are columns; WaterUI's ratio to each
contestant is shown beneath its own value (1.00x = parity).

The generator renders measured data only — every number comes from the
results JSON, which also records per-cell successful/attempted reps and
each failed attempt's error. A results file that did not meet the
requested successful-rep floor is refused instead of reported.

Usage: uv run runner/report.py results/results-*.json [-o report.md]
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

WATERUI = "waterui-hydrolysis"


def fmt_mb(b: float | None) -> str:
    if b is None:
        return "—"
    if b < 1048576:
        return f"{b / 1024:.1f} KB"
    return f"{b / 1048576:.1f} MB"


def fmt_num(v: float | None) -> str:
    if v is None:
        return "—"
    return f"{v}"


def fmt_ms(v: float | None) -> str:
    if v is None:
        return "—"
    return f"{v:.1f} ms"


def get(entry: dict, wl: str, key: str, sub: str = "median"):
    w = entry.get("workloads", {}).get(wl, {})
    v = w.get(key)
    if isinstance(v, dict):
        return v.get(sub)
    return v


def ratio(this: float | None, other: float | None) -> str:
    """WaterUI's value as a ratio vs contestant's (>1 = larger/worse for
    sizes/memory; context column header says which)."""
    if this is None or other is None or other == 0:
        return "—"
    return f"{this / other:.2f}x"


def table(rows: list[tuple[str, list[str]]], headers: list[str]) -> str:
    out = ["| " + " | ".join(headers) + " |",
           "|" + "---|" * len(headers)]
    for name, cols in rows:
        out.append("| " + name + " | " + " | ".join(cols) + " |")
    return "\n".join(out)


def delta(new: float | None, old: float | None) -> str:
    """Percent change vs a baseline run; empty when unavailable."""
    if new is None or old is None or old == 0:
        return ""
    return f" Δ{100 * (new - old) / old:+.1f}%"


def cell(water: float | None, val: float | None, fmt,
         old: float | None = None) -> str:
    """cell = 'val (WaterUI ratio) Δ±x%' — Δ only when a baseline was given."""
    return f"{fmt(val)} ({ratio(water, val)}){delta(val, old)}"


def check_complete(results: dict) -> None:
    """Refuse to report a run that did not meet the rep floor."""
    reps = results.get("repetitions")
    incomplete = []
    for name, entry in results.get("contestants", {}).items():
        for wl, w in (entry.get("workloads") or {}).items():
            succ = w.get("runs_succeeded")
            if succ is None:  # results from before attempt accounting
                succ = w.get("runs", 0)
            if reps and succ < reps:
                incomplete.append(
                    f"{name}/{wl}: {succ}/{reps} successful "
                    f"({w.get('runs_attempted', succ)} attempted)")
    if results.get("repetitions_met") is False or incomplete:
        raise SystemExit(
            "refusing to render an incomplete measurement: "
            + "; ".join(incomplete or ["repetitions_met=false"]))


def render(results: dict, baseline: dict | None = None) -> str:
    check_complete(results)
    cs = results["contestants"]
    bs = (baseline or {}).get("contestants", {})
    names = list(cs.keys())
    others = [n for n in names if n != WATERUI]
    headers = ["Metric"] + names
    L = []
    m = results.get("machine", {})
    L.append(f"# Competitive benchmark — Linux\n")
    L.append(f"Issue: {results.get('issue','')}  ·  generated "
             f"{results.get('generated_utc','')}  ·  ≥{results.get('repetitions')} runs, median reported\n")
    L.append(f"**Machine:** {m.get('cpu','?')} · {m.get('cores','?')} cores · "
             f"{m.get('mem_gb','?')} GB · {m.get('os','?')} · GPU: {m.get('gpu','?')}\n")
    L.append(f"**Frame source:** {results.get('frame_source','')}\n")
    wc = results.get("water_cli")
    if wc:
        from pathlib import Path as _P
        L.append(f"**water CLI:** in-tree cli/ @ checkout "
                 f"{wc.get('checkout_head','?')[:12]} "
                 f"(bin {_P(wc.get('provisioned','?')).name})\n")

    # successful/attempted reps per cell — failures are listed, not hidden
    rep_rows = {}
    for name, entry in cs.items():
        for wl, w in (entry.get("workloads") or {}).items():
            cell_s = f"{w.get('runs_succeeded', w.get('runs', '?'))}/" \
                     f"{w.get('runs_attempted', '?')}"
            rep_rows.setdefault(wl.upper(), {})[name] = cell_s
    if rep_rows:
        L.append("\n## Reps (successful/attempted)\n")
        rows = []
        for wl in sorted(rep_rows):
            cells = rep_rows[wl]
            rows.append((wl, [cells.get(n, "—") for n in names]))
        L.append(table(rows, headers))
        fails = [(n, wl, f["error"])
                 for n, e in cs.items()
                 for wl, w in (e.get("workloads") or {}).items()
                 for f in w.get("failures", [])]
        if fails:
            L.append("\nFailed attempts (records kept in results JSON):\n")
            for n, wl, err in fails:
                L.append(f"- {n} {wl}: {err}")
    if results.get("development_only"):
        L.append(
            "> **Development-only run** — measured on a software GPU "
            "adapter; every frame-time and memory number below is "
            "development data, not publishable evidence.\n")

    w = cs.get(WATERUI, {})
    bw = bs.get(WATERUI, {})

    # ---- package size ------------------------------------------------------
    L.append("## Package size (installed directory)\n")
    rows = []
    for label, key in (("Uncompressed", "bytes_uncompressed"),
                       ("gzip -6 tarball", "bytes_gz")):
        wp = w.get("package", {}).get(key)
        wp_old = bw.get("package", {}).get(key) if baseline else None
        rows.append((label,
                     [fmt_mb(wp) + delta(wp, wp_old)] +
                     [cell(wp, cs[o].get("package", {}).get(key), fmt_mb,
                           bs.get(o, {}).get("package", {}).get(key)
                           if baseline else None)
                      for o in others]))
    L.append(table(rows, headers))

    # ---- cold launch --------------------------------------------------------
    L.append("\n## Cold launch → first frame\n")
    rows = []
    for wl in ("w1", "w2", "w3", "w4"):
        wv = get(w, wl, "launch_ms")
        rows.append((wl.upper(),
                     [fmt_ms(wv) + delta(wv, get(bw, wl, "launch_ms")
                                         if baseline else None)] +
                     [cell(wv, get(cs[o], wl, "launch_ms"), fmt_ms,
                           get(bs.get(o, {}), wl, "launch_ms")
                           if baseline else None)
                      for o in others]))
    L.append(table(rows, headers))

    # ---- memory -------------------------------------------------------------
    L.append("\n## Memory (RSS)\n")
    rows = []
    for label, key in (("Steady", "rss_bytes_steady"), ("Peak", "rss_bytes_peak")):
        for wl in ("w1", "w2", "w3", "w4"):
            wv = get(w, wl, key)
            rows.append((f"{wl.upper()} {label.lower()}",
                         [fmt_mb(wv) + delta(wv, get(bw, wl, key)
                                             if baseline else None)] +
                         [cell(wv, get(cs[o], wl, key), fmt_mb,
                               get(bs.get(o, {}), wl, key)
                               if baseline else None)
                          for o in others]))
    L.append(table(rows, headers))

    # ---- frame pacing (W2/W3/W4) -------------------------------------------
    L.append("\n## Frame pacing — W2 (feed), W3 (motion), W4 (scroll)\n")
    rows = []

    def frame(entry: dict, wl: str, pct: str) -> float | None:
        f = entry.get("workloads", {}).get(wl, {}).get("frame_ms")
        return f.get(pct) if isinstance(f, dict) else None

    for wl in ("w2", "w3", "w4"):
        for pct in ("p50", "p90", "p99"):
            wv = frame(w, wl, pct)
            wv_old = frame(bw, wl, pct) if baseline else None
            rows.append((f"{wl.upper()} {pct}",
                         [fmt_ms(wv) + delta(wv, wv_old)] +
                         [cell(wv, frame(cs[o], wl, pct), fmt_ms,
                               frame(bs.get(o, {}), wl, pct)
                               if baseline else None)
                          for o in others]))
        for label, key in (("dropped %", "dropped_pct"),
                           ("fps", "fps"),
                           ("commits", "commit_count")):
            wv = get(w, wl, key)
            rows.append((f"{wl.upper()} {label}",
                         [fmt_num(wv) + delta(wv, get(bw, wl, key)
                                              if baseline else None)] +
                         [cell(wv, get(cs[o], wl, key), fmt_num,
                               get(bs.get(o, {}), wl, key)
                               if baseline else None)
                          for o in others]))
    L.append(table(rows, headers))

    # ---- limitations ---------------------------------------------------------
    clims = []
    for name, entry in cs.items():
        for wl, w in (entry.get("workloads") or {}).items():
            if w.get("limitation"):
                clims.append((name, wl, w["limitation"]))
    lim = results.get("limitations", {})
    if lim or clims:
        L.append("\n## Limitations\n")
        for k, v in lim.items():
            L.append(f"- **{k}** — {v}")
        for name, wl, v in clims:
            L.append(f"- **{name} {wl}** — {v}")
        L.append("")

    # ---- versions -------------------------------------------------------------
    raw = (results.get("versions") or {}).get("raw")
    if raw:
        L.append("## Versions\n\n```\n" + raw + "\n```\n")

    if results.get("development_only") or m.get("gpu_software"):
        L.append("_Numbers taken on a software-emulated GPU are development "
                 "numbers only (issue method note)._\n")
    return "\n".join(L)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("results", type=Path)
    ap.add_argument("--baseline", type=Path, default=None,
                    help="earlier results JSON; adds a delta change per cell")
    ap.add_argument("-o", "--out", type=Path, default=None)
    a = ap.parse_args()
    results = json.loads(a.results.read_text())
    baseline = json.loads(a.baseline.read_text()) if a.baseline else None
    md = render(results, baseline)
    if a.out:
        a.out.write_text(md)
        print(f"wrote {a.out}")
    else:
        print(md)
    return 0


if __name__ == "__main__":
    sys.exit(main())

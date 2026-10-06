"""Generate the markdown report for the Windows competitive benchmark
(water-rs/waterui#1262) from a results JSON produced by run.py.

Usage:
    uv run report.py [results.json] [-o report.md]

With no results file argument the newest file under results/ is used.
Report shape mirrors the Linux leg: per workload, launch / steady+peak
memory / frame pacing; package size globally; then a root-cause section
for each metric WaterUI loses.
"""

from __future__ import annotations

import sys

if sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {sys.version.split()[0]}); every leg "
        "declares its version in pyproject.toml + .python-version and "
        "runs under the uv-managed interpreter (`uv run`)")

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent

WORKLOAD_TITLES = {
    "w1": "W1 Hello — centred label + counter",
    "w2": "W2 Feed — 10,000-row lazy list, fling scroll",
    "w3": "W3 Motion — 200 animated rounded rects",
    "w4": "W4 Text — 50 scrolling paragraphs",
}

APP_ORDER = ["waterui", "flutter", "electron", "winui3"]
APP_NAMES = {
    "waterui": "WaterUI (hydrolysis)",
    "flutter": "Flutter",
    "electron": "Electron",
    "winui3": "WinUI 3",
}


def cell(stat: dict | None, unit: str = "", digits: int = 1) -> str:
    """'median (min–max)' cell for a stats_of() result."""
    if not stat:
        return "n/a"
    med = stat["median"]
    lo, hi = stat["min"], stat["max"]
    if digits == 0:
        med_s, lo_s, hi_s = f"{med:.0f}", f"{lo:.0f}", f"{hi:.0f}"
    else:
        med_s = f"{med:.{digits}f}"
        lo_s, hi_s = f"{lo:.{digits}f}", f"{hi:.{digits}f}"
    s = f"{med_s}{unit}"
    if lo != hi:
        s += f" ({lo_s}–{hi_s})"
    return s


def mb(bytes_v: float) -> float:
    return bytes_v / (1024 * 1024)


def table(headers: list[str], rows: list[list[str]]) -> str:
    out = ["| " + " | ".join(headers) + " |",
           "|" + "|".join("---" for _ in headers) + "|"]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


def workload_cell(wres: dict, key: str, unit: str = "", digits: int = 1) -> str:
    if not wres:
        return "n/a"
    if "unsupported" in wres:
        return "unsupported"
    return cell(wres.get(key), unit, digits)


def check_complete(data: dict) -> None:
    """Refuse to report a run that did not meet the rep floor."""
    reps = data.get("repetitions")
    incomplete = []
    for name, entry in data.get("results", {}).items():
        for wl, w in (entry.get("workloads") or {}).items():
            if "unsupported" in w:
                continue
            succ = w.get("runs_succeeded")
            if succ is None:
                # a cell with no success accounting is stale evidence
                # (pre-attempt-accounting schema or a carried cell) —
                # it does not pass the floor by going uncounted
                incomplete.append(
                    f"{name}/{wl}: no successful-repetition "
                    "accounting (stale cell)")
                continue
            if reps and succ < reps:
                incomplete.append(
                    f"{name}/{wl}: {succ}/{reps} successful "
                    f"({w.get('runs_attempted', succ)} attempted)")
    if data.get("repetitions_met") is False or incomplete:
        raise SystemExit(
            "refusing to render an incomplete measurement: "
            + "; ".join(incomplete or ["repetitions_met=false"]))


def generate(data: dict) -> str:
    check_complete(data)
    res = data["results"]
    order = [k for k in APP_ORDER if k in res] + [
        k for k in res if k not in APP_ORDER
    ]
    lines: list[str] = []
    a = lines.append

    a("# Competitive benchmark — Windows")
    a("")
    m = data.get("machine", {})
    a(f"Issue: {data.get('issue', 'water-rs/waterui#1262')} · "
      f"Platform: `{data.get('platform', 'windows')}` · "
      f"Repetitions: {data.get('repetitions', '?')}")
    a("")
    a("## Machine")
    a("")
    a(f"- **CPU**: {m.get('cpu', '?')}")
    a(f"- **RAM**: {m.get('ram_gb', '?')} GB")
    a(f"- **GPU**: {m.get('gpu', '?')}")
    a(f"- **OS**: {m.get('os', '?')}")
    a(f"- **Display**: {m.get('display', '?')}")
    a("")
    if data.get("development_only"):
        a("> **Development-only run** — measured on a software GPU adapter; "
          "every frame-time and memory number below is development data, "
          "not publishable evidence.")
        a("")

    a("## Contestants")
    a("")
    rows = []
    for k in order:
        c = data.get("contestants", {}).get(k, {})
        rows.append([
            APP_NAMES.get(k, k),
            c.get("exe", "?"),
            c.get("adapter", "?"),
        ])
    a(table(["Contestant", "Executable", "Adapter"], rows))
    a("")

    # successful/attempted reps per cell — failed attempts are listed,
    # not folded into the stats or hidden
    rep_rows = []
    seen_wls = [w for w in ("w1", "w2", "w3", "w4")
                if any(res[k].get("workloads", {}).get(w)
                       for k in order)]
    for w in seen_wls:
        row = [WORKLOAD_TITLES.get(w, w)]
        for k in order:
            wres = (res[k].get("workloads") or {}).get(w) or {}
            if "unsupported" in wres:
                row.append("unsupported")
            elif "runs_succeeded" in wres:
                row.append(
                    f"{wres['runs_succeeded']}/"
                    f"{wres.get('runs_attempted', '?')}")
            else:
                row.append("—")
        rep_rows.append(row)
    if rep_rows:
        a("### Reps (successful/attempted)")
        a("")
        a(table(["Workload"] + [APP_NAMES.get(k, k) for k in order],
                rep_rows))
        a("")
        fails = [(k, w, f["error"])
                 for k in order
                 for w, wres in
                 ((res[k].get("workloads") or {}).items())
                 for f in wres.get("failures", [])]
        if fails:
            a("Failed attempts (records kept in results JSON):")
            a("")
            for k, w, err in fails:
                a(f"- {APP_NAMES.get(k, k)} {w}: {err}")
            a("")
    diag = [
        APP_NAMES.get(k, k)
        for k in order
        if "diagnostic" in str(
            data.get("contestants", {}).get(k, {}).get("adapter", "")
        ).lower()
    ]
    if diag:
        a("**Diagnostic measurement:** " + ", ".join(diag) + " ran on a "
          "CPU-type GPU adapter through hydrolysis's "
          "`WATER_HYDROLYSIS_FORCE_FALLBACK_ADAPTER` hatch (rejected by "
          "policy in production). Numbers for "
          + "those contestants are diagnostic, not production-representative.")
        a("")

    # --- per-workload sections: launch / memory / pacing --------------------
    wl_ids = [w for w in ("w1", "w2", "w3", "w4")
              if any(res[k].get("workloads", {}).get(w) for k in order)]

    for w in wl_ids:
        a(f"## {WORKLOAD_TITLES[w]}")
        a("")

        unsupported_notes = [
            f"- {APP_NAMES.get(k, k)}: "
            f"{res[k]['workloads'][w]['unsupported']}"
            for k in order
            if "unsupported" in (res[k].get("workloads", {}).get(w) or {})
        ]
        if unsupported_notes:
            a("**Not runnable:**")
            a("")
            for n in unsupported_notes:
                a(n)
            a("")

        a("### Launch — cold start to first presented frame (ms)")
        a("")
        rows = [
            [APP_NAMES.get(k, k),
             workload_cell((res[k].get("workloads") or {}).get(w) or {},
                           "startup_ms", " ms")]
            for k in order
        ]
        a(table(["Contestant", "Launch"], rows))
        a("")

        a("### Memory — steady / peak private WS over the window (MB)")
        a("")
        rows = []
        for k in order:
            wres = (res[k].get("workloads") or {}).get(w) or {}
            mem = wres.get("memory") or {}
            if "unsupported" in wres:
                steady = peak = procs = "unsupported"
            else:
                steady = cell(mem.get("steady_private_ws_mb"), " MB")
                peak = cell(mem.get("peak_private_ws_mb"), " MB")
                procs = str(mem.get("process_count", "n/a"))
            rows.append([APP_NAMES.get(k, k), steady, peak, procs])
        a(table(["Contestant", "Steady private WS (MB)", "Peak private WS (MB)",
                 "Processes"], rows))
        a("")

        if w in ("w2", "w3"):
            a("### Frame pacing")
            a("")
            rows = []
            for k in order:
                wres = (res[k].get("workloads") or {}).get(w) or {}
                fr = wres.get("frame_rate") or {}
                if "unsupported" in wres:
                    rows.append([APP_NAMES.get(k, k), "unsupported",
                                 "unsupported", "unsupported", "unsupported",
                                 "unsupported"])
                    continue
                srcs = {s for s in fr.get("source", []) if s}
                src = f" [{', '.join(sorted(srcs))}]" if srcs else ""
                rows.append([
                    APP_NAMES.get(k, k) + src,
                    cell(fr.get("fps")),
                    cell(fr.get("frame_ms_p50")),
                    cell(fr.get("frame_ms_p90")),
                    cell(fr.get("frame_ms_p99")),
                    cell(fr.get("missed_vsyncs")),
                ])
            a(table(["Contestant", "fps", "frame ms p50", "p90", "p99",
                     "missed vsyncs"], rows))
            a("")
            a("Frame-timing event source per contestant:")
            a("")
            seen = set()
            for k in order:
                fr = ((res[k].get("workloads") or {}).get(w) or {}).get(
                    "frame_rate") or {}
                for s, label in zip(fr.get("source", []),
                                    fr.get("source_label", [])):
                    if s and s not in seen:
                        seen.add(s)
                        a(f"- `{s}` — {label}")
            a("")
        else:
            a("*Frame pacing: static workload — only a handful of presents "
              "are emitted per run by every contestant; intervals are not "
              "meaningful.*")
            a("")

    # --- package size -------------------------------------------------------
    a("## Package size (installed directory)")
    a("")
    rows = []
    for k in order:
        ps_ = res[k].get("package_size") or {}
        rows.append([
            APP_NAMES.get(k, k),
            f"{mb(ps_.get('uncompressed_bytes', 0)):.1f}"
            if ps_.get("uncompressed_bytes") else "n/a",
            f"{mb(ps_.get('compressed_bytes', 0)):.1f}"
            if ps_.get("compressed_bytes") else "n/a",
        ])
    a(table(["Contestant", "Uncompressed (MB)", "Deflate-compressed (MB)"],
            rows))
    a("")

    # --- limitations --------------------------------------------------------
    a("## Limitations")
    a("")
    for lim in data.get("limitations", []):
        a(f"- {lim}")
    a("")
    for k in order:
        for wl, wres in (res[k].get("workloads") or {}).items():
            if "unsupported" in wres:
                a(f"- {APP_NAMES.get(k, k)} {wl}: {wres['unsupported']}")
    a("")

    # --- versions -----------------------------------------------------------
    a("## Versions")
    a("")
    tc = data.get("toolchain", {})
    fw = data.get("frameworks", {})
    for k_, v in tc.items():
        a(f"- toolchain `{k_}`: {v}")
    for k_, v in fw.items():
        a(f"- framework `{k_}`: {v}")
    # in-tree CLI model: framework+CLI+backend identity = checkout HEAD
    head = data.get("checkout_head") or \
        (data.get("cli") or {}).get("checkout_head")
    if head:
        a(f"- waterui checkout HEAD: `{str(head)[:12]}`")
    a("")
    return "\n".join(lines)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("results", nargs="?", default=None)
    ap.add_argument("-o", "--output", default=None)
    args = ap.parse_args()

    if args.results:
        src = Path(args.results)
    else:
        cands = sorted(
            (ROOT / "results").glob("*.json"),
            key=lambda p: p.stat().st_mtime,
        )
        if not cands:
            sys.exit("no results JSON found under results/")
        src = cands[-1]

    data = json.loads(src.read_text())
    md = generate(data)

    out = Path(args.output) if args.output else src.with_suffix(".md")
    out.write_text(md, encoding="utf-8")
    print(f"report -> {out}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""bench/android/compare.py — the section-6 statistical gate.

Two commands:

  freeze-envelope   12 A/A pairs of the reference build; the per-metric noise
                    envelope is the 95th percentile of absolute paired
                    differences. Written to a campaign lock.

  compare           12 balanced AB/BA pairs (extend to 24 when inconclusive)
                    of candidate vs reference, paired by round index. For
                    each metric: PASS when the upper confidence bound of the
                    candidate's regression lies inside the frozen A/A
                    envelope; FAIL when a regression beyond it is
                    established; INCONCLUSIVE otherwise — never enlarging the
                    envelope to force a verdict.

Zero-tolerance rules from section 6: package bytes have zero tolerance; a
candidate's extra missed frames earn no allowance when the reference showed
zero. Intervals are paired over independent rounds, not correlated frames,
computed with a deterministic-seed percentile bootstrap so the harness needs
no scipy.
"""

from __future__ import annotations

import argparse
import json
import math
import random
import sys
from pathlib import Path

BOOTSTRAP_ITERS = 10_000
CONFIDENCE = 0.95
DEFAULT_PAIRS = 12
EXTENDED_PAIRS = 24

ZERO_TOLERANCE_METRICS = frozenset({"apk_bytes", "total_bytes"})
ZERO_ALLOWANCE_METRICS = frozenset(
    {"frames_missed", "missed_slots", "missed_vsync", "markers_dropped"}
)


def load_records(path: Path) -> list[dict]:
    records = []
    for file in sorted(Path(path).glob("*.json")):
        try:
            record = json.loads(file.read_text())
        except json.JSONDecodeError:
            continue
        if record.get("schema") == "bench/android/result@1":
            records.append(record)
    return records


def metric_values(record: dict) -> dict[str, float]:
    """The numeric leaf metrics of one round record."""
    return {
        k: float(v)
        for k, v in record.get("metrics", {}).items()
        if isinstance(v, (int, float)) and not isinstance(v, bool)
    }


def pair_rounds(
    reference: list[dict], candidate: list[dict]
) -> list[tuple[dict, dict]]:
    """Pair rounds 1:1 by order — A/A or AB/BA interleave is the caller's
    responsibility (paired intervals need independent rounds, so pairing
    never reshuffles)."""
    by_round: dict[int, list[dict]] = {}
    for rec in reference:
        by_round.setdefault(rec.get("round", 0), []).append(rec)
    pairs = []
    for cand in candidate:
        pool = by_round.get(cand.get("round", 0), [])
        if pool:
            pairs.append((pool.pop(0), cand))
    return pairs


def bootstrap_ci(
    diffs: list[float], confidence: float = CONFIDENCE
) -> tuple[float, float, float]:
    """Percentile bootstrap CI of the paired difference mean; deterministic
    seed keeps a campaign reproducible."""
    if not diffs:
        return (math.nan, math.nan, math.nan)
    rng = random.Random(0xBE11)
    n = len(diffs)
    means = sorted(
        sum(diffs[rng.randrange(n)] for _ in range(n)) / n
        for _ in range(BOOTSTRAP_ITERS)
    )
    lo = means[int((1 - confidence) / 2 * BOOTSTRAP_ITERS)]
    hi = means[min(BOOTSTRAP_ITERS - 1, int((1 + confidence) / 2 * BOOTSTRAP_ITERS))]
    return (sum(diffs) / n, lo, hi)


def freeze_envelope(aa_pairs: list[tuple[dict, dict]]) -> dict:
    """The A/A noise envelope: per-metric p95 of absolute paired diffs."""
    diffs: dict[str, list[float]] = {}
    for a, b in aa_pairs:
        va, vb = metric_values(a), metric_values(b)
        for key in set(va) & set(vb):
            diffs.setdefault(key, []).append(abs(va[key] - vb[key]))
    envelope = {}
    for key, values in diffs.items():
        values.sort()
        envelope[key] = values[
            max(0, min(len(values) - 1, math.ceil(0.95 * len(values)) - 1))
        ]
    return envelope


def compare_metric(
    metric: str, ref: list[float], cand: list[float], envelope: dict
) -> dict:
    """One metric's gate verdict."""
    if len(ref) != len(cand) or not ref:
        return {"metric": metric, "verdict": "inconclusive",
                "reason": "unpaired rounds"}
    diffs = [c - r for r, c in zip(ref, cand)]
    mean, lo, hi = bootstrap_ci(diffs)
    bound = envelope.get(metric)
    result = {
        "metric": metric,
        "reference_mean": sum(ref) / len(ref),
        "candidate_mean": sum(cand) / len(cand),
        "diff_mean": mean,
        "diff_ci": [lo, hi],
        "aa_envelope": bound,
    }
    if metric in ZERO_TOLERANCE_METRICS:
        result["verdict"] = (
            "pass" if all(d == 0 for d in diffs) else "fail"
        )
        return result
    if metric in ZERO_ALLOWANCE_METRICS and all(r == 0 for r in ref):
        result["verdict"] = "pass" if hi <= 0 else "fail"
        return result
    if bound is None:
        result["verdict"] = "inconclusive"
        result["reason"] = "no A/A envelope for metric"
        return result
    # Regression is positive-for-worse by convention: metrics where lower is
    # better are the only ones gated on upper bound; improvement-only
    # metrics (none today) would gate the lower bound instead.
    if hi <= bound:
        result["verdict"] = "pass"
    elif lo > bound:
        result["verdict"] = "fail"
    else:
        result["verdict"] = "inconclusive"
        result["reason"] = "CI straddles the A/A envelope; extend to 24 pairs"
    return result


def cmd_freeze(args: argparse.Namespace) -> int:
    records = load_records(Path(args.aa_dir))
    if len(records) < 2:
        sys.exit("envelope freeze needs A/A pair records, found < 2")
    half = len(records) // 2
    envelope = freeze_envelope(
        list(zip(records[:half], records[half : half * 2]))
    )
    out = {
        "schema": "bench/android/envelope@1",
        "pairs": half,
        "bootstrap_confidence": CONFIDENCE,
        "envelope": envelope,
    }
    Path(args.out).write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
    print(f"froze envelope over {half} A/A pairs -> {args.out}")
    return 0


def cmd_compare(args: argparse.Namespace) -> int:
    reference = load_records(Path(args.reference))
    candidate = load_records(Path(args.candidate))
    envelope = json.loads(Path(args.envelope).read_text())["envelope"]
    pairs = pair_rounds(reference, candidate)
    if len(pairs) < DEFAULT_PAIRS:
        print(
            f"warning: {len(pairs)} paired rounds < {DEFAULT_PAIRS} required; "
            "verdicts are provisional",
            file=sys.stderr,
        )
    ref_metrics: dict[str, list[float]] = {}
    cand_metrics: dict[str, list[float]] = {}
    for ref, cand in pairs:
        for key, value in metric_values(ref).items():
            ref_metrics.setdefault(key, []).append(value)
        for key, value in metric_values(cand).items():
            cand_metrics.setdefault(key, []).append(value)
    verdicts = []
    for metric in sorted(set(ref_metrics) & set(cand_metrics)):
        verdicts.append(
            compare_metric(
                metric, ref_metrics[metric], cand_metrics[metric], envelope
            )
        )
    summary = {
        "schema": "bench/android/comparison@1",
        "pairs": len(pairs),
        "verdicts": verdicts,
        "overall": (
            "fail"
            if any(v["verdict"] == "fail" for v in verdicts)
            else "pass"
            if all(v["verdict"] == "pass" for v in verdicts)
            else "inconclusive"
        ),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if summary["overall"] != "fail" else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    freeze = sub.add_parser(
        "freeze-envelope", help="freeze the A/A noise envelope"
    )
    freeze.add_argument("--aa-dir", required=True,
                        help="directory of A/A result records")
    freeze.add_argument("--out", required=True)
    freeze.set_defaults(func=cmd_freeze)

    comp = sub.add_parser("compare", help="gate candidate vs reference")
    comp.add_argument("--reference", required=True)
    comp.add_argument("--candidate", required=True)
    comp.add_argument("--envelope", required=True)
    comp.set_defaults(func=cmd_compare)
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())

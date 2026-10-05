"""Uniform frame statistics for every competitive leg (WORKLOADS.md, METHOD).

One definition for all legs:

- Inputs: present timestamps (milliseconds) attributable to the
  contestant's owned processes, the measurement window, and the display
  refresh period.
- The window is [first owned present + declared warmup, + capture_ms].
  Callers pass it explicitly; startup frames sit before it and never
  enter the data.
- A gap longer than GAP_MS with no present ends an active run and is
  excluded — it is neither a frame nor a drop.
- Within active runs, frame time = present interval, and an interval
  longer than MISS_FACTOR refresh periods contributes
  round(interval / period) - 1 missed vsyncs.
"""

import sys as _sys

if _sys.version_info < (3, 10):
    raise SystemExit(
        "benchmarks/competitive requires Python >= 3.10 "
        f"(this interpreter is {_sys.version.split()[0]}); every leg "
        "declares its version in pyproject.toml + .python-version and "
        "runs under the uv-managed interpreter (`uv run`)")


GAP_MS = 100.0
MISS_FACTOR = 1.5


def _percentile(vals: list[float], p: float) -> float | None:
    if not vals:
        return None
    xs = sorted(vals)
    k = (len(xs) - 1) * p / 100.0
    lo, hi = int(k), min(int(k) + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def frame_statistics(presents_ms: list[float], window_start_ms: float,
                     capture_ms: float, refresh_ms: float) -> dict:
    """Compute frame statistics over one measurement window.

    `presents_ms` are present timestamps (any epoch — the window is
    expressed in the same units) belonging to the contestant's owned
    processes. Presents outside [window_start_ms, window_start_ms +
    capture_ms] are ignored entirely (startup frames included).

    Returns a dict with:
      presents           total owned presents inside the window
      runs               count of maximal runs of presents separated by
                         at most GAP_MS
      frame_ms_p50/p90/p99  percentiles over intervals inside active runs
      missed_vsyncs      sum of round(i / refresh_ms) - 1 over intervals
                         longer than MISS_FACTOR periods
      fps                active-run presents per active second
      intervals_ms       every inside-run interval, in order (evidence)
    """
    end = window_start_ms + capture_ms
    ts = sorted(t for t in presents_ms
                if window_start_ms <= t <= end)
    intervals: list[float] = []
    missed = 0
    runs = 0
    prev = None
    for t in ts:
        if prev is not None:
            iv = t - prev
            if iv > GAP_MS:
                runs += 1
            else:
                intervals.append(iv)
                if iv > MISS_FACTOR * refresh_ms:
                    missed += round(iv / refresh_ms) - 1
        else:
            runs = 1
        prev = t
    active_s = sum(intervals) / 1000.0
    return {
        "presents": len(ts),
        "runs": runs if ts else 0,
        "frame_ms_p50": _percentile(intervals, 50),
        "frame_ms_p90": _percentile(intervals, 90),
        "frame_ms_p99": _percentile(intervals, 99),
        "missed_vsyncs": missed,
        "fps": round(len(intervals) / active_s, 2) if active_s > 0 else None,
        "intervals_ms": intervals,
    }


def _selftest() -> None:
    # window bounds: presents outside are dropped
    r = frame_statistics([0, 16, 32, 48, 500, 516, 532],
                         window_start_ms=100, capture_ms=500,
                         refresh_ms=16.667)
    assert r["presents"] == 3 and r["intervals_ms"] == [16.0, 16.0]
    # a >100 ms gap ends the run and is excluded (not a frame, not a miss)
    r = frame_statistics([100, 116, 132, 400, 416, 432],
                         window_start_ms=100, capture_ms=500,
                         refresh_ms=16.667)
    assert r["intervals_ms"] == [16.0, 16.0, 16.0, 16.0]
    assert r["missed_vsyncs"] == 0 and r["runs"] == 2
    # one 3-period interval => round(50/16.667)-1 = 2 missed vsyncs
    r = frame_statistics([100, 116, 166, 182],
                         window_start_ms=100, capture_ms=200,
                         refresh_ms=16.667)
    assert r["missed_vsyncs"] == 2
    assert r["intervals_ms"] == [16.0, 50.0, 16.0]
    print("frame_stats self-test ok")


if __name__ == "__main__":
    _selftest()

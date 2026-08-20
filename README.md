# Benchmark history

This branch stores the wall-clock series produced by the `Bench` workflow, one
entry per push to an integration branch, under `dev/bench/`. It is written by
[`benchmark-action/github-action-benchmark`][action] and is not a website: no
GitHub Pages site is served from it unless someone configures one.

It exists because WaterUI gates performance in two tiers. Deterministic
counters — rebuild ratio, scene/clip/GPU-surface layer counts, measurement
cache misses — are hard budgets enforced in-process by `water bench` itself,
and a violation fails the build. Wall-clock timings cannot be gated that way on
shared CI runners without becoming flaky, so they are tracked here over time
and surfaced as alert comments when a trend crosses its threshold.

Do not commit to this branch by hand.

[action]: https://github.com/benchmark-action/github-action-benchmark
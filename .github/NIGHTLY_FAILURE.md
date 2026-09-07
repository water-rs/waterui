---
title: "nightly: the full matrix is red"
labels: ci
---
The nightly full matrix failed: {{ env.RUN_URL }}

Every job in that run is gating for `dev`: the per-change gate (`ci.yml`) only runs the default-features shape on Linux, so a failure here is either a shape a pull request cannot see (all-features, macOS, Windows, coverage, `cargo hack --each-feature`) or hosted-runner and toolchain drift. Read the failing job's log, fix the root cause on a topic branch, and close this issue when the next nightly is green.

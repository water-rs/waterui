---
title: "nightly: the full matrix is red"
labels: ci
---
The nightly full matrix failed: {{ env.RUN_URL }}

Every required check in that run gates nightly certification, including the WebView JavaScript bridge unit suite. The dev-push gate only runs format and compilation/lint checks; nightly also covers tests, all-features, macOS, Windows, coverage and `cargo hack --each-feature`. Read the failing job's log and uploaded test results, fix the root cause on a topic branch, and close this issue when the next nightly is green. A JavaScript suite failure must prevent certified nightly promotion even when every Rust check succeeds.

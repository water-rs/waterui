---
title: "nightly: the competitor-engine corpus render failed"
labels: ci
---
The corpus render on the Vello and Skia adapters failed in the nightly run: {{ env.RUN_URL }}

Those jobs compare Cherenkov with other projects' engines and do not gate nightly certification, so the run itself is not red. Open the failing `Corpus on Vello and Skia` job (CPU, Metal or both) in the run, find the engine and scene that failed, and fix the adapter or report the defect upstream. Close this issue when the next nightly's corpus jobs are green.

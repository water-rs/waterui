---
title: "nightly: the Windows leg failed"
labels: ci
---
A Windows job (tests, all-features tests or lint) failed in the nightly run: {{ env.RUN_URL }}

Windows is tier 2: the leg does not gate nightly certification, so the run itself is not red. Open the failing `Windows` job in the run, find the test or lint that failed, and fix it. Close this issue when the next nightly's Windows jobs are green.

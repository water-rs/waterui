---
title: "apple e2e: the example suite is red"
labels: ci
---

The Apple e2e suite failed: {{ env.RUN_URL }}

Certified inputs: framework + `backends/apple` `{{ env.WATERUI_SHA }}` ({{ env.WATERUI_REF }}), water CLI `{{ env.CLI_SHA }}` ({{ env.CLI_REF }}).

Each shard packages every runnable example in release mode and launches the
`.app` on an iOS simulator and on macOS, captures the first settled screen,
and verifies it — non-blank always, compared against the recorded baseline
where one exists under `backends/apple/Tests/E2EBaselines`, and parity-
compared against the example's SwiftUI twin rendered live by
`backends/apple/Tests/E2EReference` when one is registered. The
`e2e-shots-*` artifacts carry every capture and the amplified diff images;
`e2e-logs-*` carry the packaging and launch-marker logs for failed shards.
Read the failing job's log, fix the root cause on a topic branch, and close
this issue when the next run is green. If the new output is correct,
dispatch the `Apple E2E Suite` workflow with `record_baselines: true`.

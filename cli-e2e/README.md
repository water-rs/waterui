# Nightly CLI cold-compile metrics

Written by the `cli-e2e-metrics` job of `.github/workflows/nightly.yml` on
`dev`; do not edit by hand. Every nightly appends one record per `cli-e2e`
leg (macOS, Windows, Ubuntu/Linux, Android).

- `history.jsonl` — the source of truth: one JSON object per leg per night
  with `date`, `run_id`, `event`, `sha`, `platform`, `runner`, `image`
  (runner image OS + version), `rustc`, `status`, `cold_verified`,
  `cli_install_cold` and `metrics` (seconds per phase).
- `history.csv` — the same records flattened, one column per phase, for a
  spreadsheet or `python -m csv`-style tooling.

Phases (wall-clock seconds, fresh OS user, no Actions cache, no compiler
wrapper): `cli_install` (`cargo install --path cli` on the runner account),
`doctor_fix`, `doctor_fix_android`, `create`, `cold_build` (the first
`water build` of the scaffolded app — the cold compile), `package`,
`package_debug` / `package_release` (Android APKs), `console_build` (macOS:
the console user's own `water build`, needed because its build cache is
separate), `launch` (`water run` start to 'Application started', after the
build). A missing value means the phase does not exist on that platform or
did not complete that night.

Quick look at one platform's cold builds:

    git show origin/ci-metrics:cli-e2e/history.csv | grep ',linux,' | cut -d, -f1,4,8,15

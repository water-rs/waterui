#!/usr/bin/env bash
# cargo target runner: hands the test binary to the water CLI under test.
set -euo pipefail
exec "${WATER_BIN:?WATER_BIN must name the water CLI build under test (the runner lives in water test-runner)}" test-runner "$@"

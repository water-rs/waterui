#!/usr/bin/env bash
# Nightly prepare job. Partitions the discovered example set into
# PACKAGE_GROUP_TOTAL packaging groups of near-equal wall-clock cost so the
# build-<platform> matrix legs finish together instead of one leg
# inheriting the long tail — the serial packaging leg measured ~90 of the
# run's ~102 minutes (#252). Assignment is greedy LPT (heaviest example
# into the currently lightest group) over measured per-example packaging
# seconds in .github/example-package-weights.json; those weights come from
# the measured nightlies' packaging logs and are refreshed by hand — a
# stale weight only unbalances the split, while coverage still comes from
# discovery, so a new example joins the lightest group automatically. A
# count or checksum split does not balance (cksum % 4 leaves one leg at
# 35.8 min against a 16.1-min leg on the measured set).
#
# Prints a compact JSON array of {"i": <index>, "examples": "a,b,c"} on
# stdout; the workflow feeds it to the build job matrices through
# fromJson(needs.prepare.outputs.package_groups).
set -euo pipefail

workspace="${GITHUB_WORKSPACE:-$(pwd)}"
waterui_dir="${WATERUI_DIR:-${workspace}}"
weights_file="${WEIGHTS_FILE:-${workspace}/.github/example-package-weights.json}"
group_total="${PACKAGE_GROUP_TOTAL:-5}"

# A failed discovery exits nonzero here — its ::error line was captured into
# `discovered`, so reprint it before leaving. Reading through a substitution
# instead of process substitution keeps the exit status; a `::error` line
# captured as an example name would otherwise poison the emitted matrix.
if ! discovered="$("${workspace}/.github/scripts/discover-examples.sh" "${waterui_dir}")"; then
  printf '%s\n' "${discovered}"
  exit 1
fi

all_examples=()
while IFS= read -r example; do
  [[ -n "${example}" ]] || continue
  all_examples+=("${example}")
done <<< "${discovered}"

if (( ${#all_examples[@]} == 0 )); then
  echo "::error::No runnable examples found under ${waterui_dir}/examples."
  exit 1
fi

WEIGHTS_FILE="${weights_file}" GROUP_TOTAL="${group_total}" \
EXAMPLES_CSV="$(IFS=,; echo "${all_examples[*]}")" \
python3 - <<'PY'
import json, os, statistics

with open(os.environ["WEIGHTS_FILE"]) as f:
    weights = json.load(f)
examples = os.environ["EXAMPLES_CSV"].split(",")
# An example the table does not know is new since the last refresh; the
# table median places it in a plausible group until it is measured.
default = statistics.median(weights.values()) if weights else 120
groups = [[] for _ in range(min(int(os.environ["GROUP_TOTAL"]), len(examples)))]
sums = [0.0] * len(groups)
for name in sorted(examples, key=lambda n: (-weights.get(n, default), n)):
    i = sums.index(min(sums))
    groups[i].append(name)
    sums[i] += weights.get(name, default)
print(json.dumps(
    [{"i": i, "examples": ",".join(g)} for i, g in enumerate(groups)],
    separators=(",", ":")))
PY

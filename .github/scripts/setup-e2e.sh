#!/usr/bin/env bash
# Prepares the e2e workspace for driving the examples. The framework, the
# Apple backend and the `water` CLI are all this repository since #1446 —
# the checked-out commit is what the suite tests — so this script makes no
# checkout at all: it publishes the same SHA as the CLI revision and names
# `cli/` as the install source (`cargo install --path` on a workspace member
# compiles exactly what a fresh user would).
#
# Outputs: waterui_ref/waterui_sha and cli_ref/cli_sha are identical pairs
# now — every consumer still gets the revision it certifies by name, and a
# run can never split the certification across two different CLI commits
# because there is only one commit in a run.
set -euo pipefail

repo_root="${GITHUB_WORKSPACE:-$(pwd)}"

# The certified framework commit is the checkout the runner made: the
# backend under test is the tree at `backends/apple` in it, and the CLI
# under test is `cli/` in the same tree.
framework_sha="$(git -C "${repo_root}" rev-parse HEAD)"
cli_dir="${repo_root}/cli"
cli_ref="${GITHUB_REF_NAME:-dev}"
cli_sha="${framework_sha}"

echo "Using waterui ${framework_sha} (this checkout)"
echo "Using water-cli ${cli_sha} (cli/ in this checkout)"

if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    echo "WATERUI_DIR=${repo_root}"
    echo "WATERUI_SHA=${framework_sha}"
    echo "WATER_CLI_DIR=${cli_dir}"
    echo "WATER_CLI_REF=${cli_ref}"
    echo "WATER_CLI_SHA=${cli_sha}"
  } >> "${GITHUB_ENV}"
fi

# The `prepare` job reads these back as job outputs and hands them to every
# consumer; writing them unconditionally is harmless in the other jobs.
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "waterui_ref=${GITHUB_REF_NAME:-dev}"
    echo "waterui_sha=${framework_sha}"
    echo "cli_ref=${cli_ref}"
    echo "cli_sha=${cli_sha}"
  } >> "${GITHUB_OUTPUT}"
fi

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    echo "### Resolved inputs"
    echo "- waterui + backends/apple: \`${framework_sha}\` (${GITHUB_REF_NAME:-this checkout})"
    echo "- water CLI: \`${cli_sha}\` (cli/ in this checkout)"
  } >> "${GITHUB_STEP_SUMMARY}"
fi

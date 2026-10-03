#!/usr/bin/env bash
# Prepares the e2e workspace for driving the examples. The framework and the
# Apple backend are this repository — the checked-out commit is what the
# suite tests — so the only checkout this script makes is the `water` CLI
# (water-rs/cli). It lands OUTSIDE the framework checkout: cloned inside the
# workspace, cargo resolves the framework's root Cargo.toml as the CLI
# manifest's workspace and `cargo install --path` fails before it builds.
#
# Inputs: WATER_CLI_REF names a branch or tag (a full 40-hex commit is
# accepted verbatim); WATER_CLI_SHA names the resolved commit directly and
# takes precedence. Every job in the e2e workflows runs this script
# independently, so the CLI ref is resolved once — in the `prepare` job,
# which publishes the SHA as a job output — and every consumer fetches
# exactly that commit. A branch that moves mid-run cannot split the
# certification across two different CLI commits.
set -euo pipefail

repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
# RUNNER_TEMP is the job-owned area on a hosted runner and is cleaned for
# us; outside Actions (a local run of this script) mktemp hands over an
# owned directory with the same lifetime semantics.
cli_parent="${RUNNER_TEMP:-$(mktemp -d)}"
cli_dir="${cli_parent}/water-cli"

cli_ref="${WATER_CLI_REF:-dev}"

# A 40-hex input already names a commit; anything else is a branch or tag and
# is resolved against the remote here.
resolve_commit() {
  local url="$1" ref="$2" sha
  if [[ "${ref}" =~ ^[0-9a-fA-F]{40}$ ]]; then
    printf '%s\n' "${ref}"
    return 0
  fi
  sha="$(git ls-remote "${url}" "refs/heads/${ref}" | awk 'NR == 1 {print $1}')"
  if [[ -z "${sha}" ]]; then
    sha="$(git ls-remote "${url}" "refs/tags/${ref}" | awk 'NR == 1 {print $1}')"
  fi
  if [[ -z "${sha}" ]]; then
    echo "::error::Could not resolve '${ref}' to a commit on ${url}." >&2
    return 1
  fi
  printf '%s\n' "${sha}"
}

# `git clone --branch` does not accept a bare SHA; fetch the pinned commit
# directly instead, keeping the clone shallow.
checkout_commit() {
  local url="$1" sha="$2" dir="$3"
  rm -rf "${dir}"
  git init -q "${dir}"
  git -C "${dir}" remote add origin "${url}"
  git -C "${dir}" fetch -q --depth 1 origin "${sha}"
  git -C "${dir}" checkout -q --detach FETCH_HEAD
}

# The certified framework commit is the checkout the runner made: the
# backend under test is the tree at `backends/apple` in it.
framework_sha="$(git -C "${repo_root}" rev-parse HEAD)"
cli_sha="${WATER_CLI_SHA:-$(resolve_commit https://github.com/water-rs/cli.git "${cli_ref}")}"

echo "Using waterui ${framework_sha} (this checkout)"
echo "Using water-cli ${cli_ref} (${cli_sha})"
checkout_commit https://github.com/water-rs/cli.git "${cli_sha}" "${cli_dir}"

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
    echo "### Certified inputs"
    echo "- waterui + backends/apple: \`${framework_sha}\` (${GITHUB_REF_NAME:-this checkout})"
    echo "- water CLI: \`${cli_sha}\` (${cli_ref})"
  } >> "${GITHUB_STEP_SUMMARY}"
fi

#!/usr/bin/env bash
# Approves the pull_request runs GitHub holds on a pull request the Actions
# bot opened or pushed (release-plz's). Usage: approve-held-runs.sh <pr-number>
set -euo pipefail

pr=$1
sha=$(gh pr view "$pr" -R "$GITHUB_REPOSITORY" --json headRefOid -q .headRefOid)

# The runs are created by the push that opened or updated the pull request;
# give them a moment to appear rather than approving nothing.
deadline=$(( $(date +%s) + 120 ))
until [ "$(gh api "repos/${GITHUB_REPOSITORY}/actions/runs?head_sha=${sha}&event=pull_request" -q '.total_count')" -gt 0 ]; do
  if [ "$(date +%s)" -gt "$deadline" ]; then
    echo "::error::no pull_request run appeared on #${pr} (${sha})"
    exit 1
  fi
  sleep 5
done

for run in $(gh api "repos/${GITHUB_REPOSITORY}/actions/runs?head_sha=${sha}&event=pull_request" \
  -q '.workflow_runs[] | select(.conclusion == "action_required") | .id'); do
  gh api -X POST "repos/${GITHUB_REPOSITORY}/actions/runs/${run}/approve" > /dev/null
  echo "approved run ${run} on #${pr}"
done

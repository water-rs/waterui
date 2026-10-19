#!/usr/bin/env bash
# Refreshes the package index for Ubuntu's own archive and nothing else; see
# action.yml beside this file for why.
set -euo pipefail
if [ -f /etc/apt/sources.list.d/ubuntu.sources ]; then
  list=/etc/apt/sources.list.d/ubuntu.sources
elif [ -s /etc/apt/sources.list ]; then
  list=/etc/apt/sources.list
else
  echo "::error::no Ubuntu archive source file on this runner"
  exit 1
fi
# A stalled mirror connection otherwise holds the step until the job's own
# timeout; each fetch gives up after 30 s and the whole refresh after 5 min.
if ! timeout 300 sudo apt-get update \
  -o Acquire::Retries=10 \
  -o Acquire::http::Timeout=30 \
  -o Acquire::https::Timeout=30 \
  -o Dir::Etc::SourceList="$list" \
  -o Dir::Etc::SourceParts=/dev/null; then
  echo "::error::refreshing the Ubuntu apt index failed or exceeded its 5 minute deadline"
  exit 1
fi

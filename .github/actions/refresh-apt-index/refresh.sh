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
sudo apt-get update \
  -o Dir::Etc::SourceList="$list" \
  -o Dir::Etc::SourceParts=/dev/null

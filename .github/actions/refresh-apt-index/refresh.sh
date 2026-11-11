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

# The hosted runner's mirror list selects Azure's archive, whose cold
# downloads have held setup for over 40 minutes (#3439), so Ubuntu's official
# HTTPS endpoints come first. They are one host each, and runners have lost
# them for over an hour at a time, so each source resolves through apt's
# mirror method: the official endpoint, then the runner's own mirrors, tried
# in that order whenever a fetch fails.
#
# `mirrors <official> <runner list> <runner url>` writes that list: the
# official endpoint, then every mirror the runner's list names (or, on an
# image without one, the archive URL its sources name), without duplicates.
mirrors() {
  local official=$1 runner_list=$2 runner_url=$3
  {
    printf '%s\n' "$official"
    if [ -f "$runner_list" ]; then
      cut -f1 "$runner_list"
    else
      printf '%s\n' "$runner_url"
    fi
  } | sed -e 's|/*$||' | awk 'NF && !seen[$0]++ { printf "%s\tpriority:%d\n", $0, NR }'
}
archive_mirrors=/etc/apt/waterui-archive-mirrors.txt
security_mirrors=/etc/apt/waterui-security-mirrors.txt
mirrors https://archive.ubuntu.com/ubuntu /etc/apt/apt-mirrors.txt \
  http://azure.archive.ubuntu.com/ubuntu | sudo tee "$archive_mirrors" > /dev/null
mirrors https://security.ubuntu.com/ubuntu /etc/apt/apt-mirrors-security.txt \
  http://azure.archive.ubuntu.com/ubuntu | sudo tee "$security_mirrors" > /dev/null
sudo sed -i -E \
  -e "s#mirror\+file:/etc/apt/apt-mirrors-security\.txt#mirror+file:${security_mirrors}#g" \
  -e "s#mirror\+file:/etc/apt/apt-mirrors\.txt#mirror+file:${archive_mirrors}#g" \
  -e "s#https?://security\.ubuntu\.com/ubuntu/?#mirror+file:${security_mirrors}#g" \
  -e "s#https?://(azure\.)?archive\.ubuntu\.com/ubuntu/?#mirror+file:${archive_mirrors}#g" \
  "$list"

# A stalled mirror connection otherwise holds the step until the job's own
# timeout; each fetch gives up after 30 s and the whole refresh after 5 min.
# apt retries a failed fetch on the same mirror before the mirror method
# moves on, so one retry per mirror keeps the next mirror inside the
# deadline: ten retries of an unreachable host spent all five minutes on it.
# `--error-on=any` fails the refresh when an index cannot be fetched from
# any mirror; apt otherwise exits 0 with an empty index, and the job dies
# later at the install with "Unable to locate package".
if ! timeout 300 sudo apt-get update --error-on=any \
  -o Acquire::Retries=1 \
  -o Acquire::http::Timeout=30 \
  -o Acquire::https::Timeout=30 \
  -o Dir::Etc::SourceList="$list" \
  -o Dir::Etc::SourceParts=/dev/null; then
  echo "::error::refreshing the Ubuntu apt index failed or exceeded its 5 minute deadline"
  exit 1
fi

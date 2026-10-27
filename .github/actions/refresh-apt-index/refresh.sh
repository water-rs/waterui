#!/usr/bin/env bash
# Refreshes the package index for Ubuntu's own archive plus the source files
# passed as arguments, and nothing else; see action.yml beside this file for
# why.
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
# downloads have held setup for over 40 minutes (#3439). Resolve both the
# index and package archives through Ubuntu's official HTTPS endpoints.
sudo sed -i \
  -e 's|mirror+file:/etc/apt/apt-mirrors-security.txt|https://security.ubuntu.com/ubuntu|g' \
  -e 's|mirror+file:/etc/apt/apt-mirrors.txt|https://archive.ubuntu.com/ubuntu|g' \
  -e 's|http://azure.archive.ubuntu.com/ubuntu|https://archive.ubuntu.com/ubuntu|g' \
  -e 's|https://azure.archive.ubuntu.com/ubuntu|https://archive.ubuntu.com/ubuntu|g' \
  "$list"
# The archive and the named sources are the whole source set: a deb822
# archive and every named file go into one parts directory, so a single
# update indexes them together and drops nothing it just fetched.
parts="$(mktemp -d)"
if [ "$list" = /etc/apt/sources.list.d/ubuntu.sources ]; then
  ln -s "$list" "$parts/"
  list=/dev/null
fi
for source in "$@"; do
  test -f "$source"
  ln -s "$source" "$parts/"
done
# A stalled mirror connection otherwise holds the step until the job's own
# timeout; each fetch gives up after 30 s and the whole refresh after 5 min.
if ! timeout 300 sudo apt-get update \
  -o Acquire::Retries=10 \
  -o Acquire::http::Timeout=30 \
  -o Acquire::https::Timeout=30 \
  -o Dir::Etc::SourceList="$list" \
  -o Dir::Etc::SourceParts="$parts"; then
  echo "::error::refreshing the Ubuntu apt index failed or exceeded its 5 minute deadline"
  exit 1
fi

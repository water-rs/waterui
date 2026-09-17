#!/bin/sh
# Regenerates tests/fixtures/library.js from src/js.
set -eu
cd "$(dirname "$0")/.."
exec bun run scripts/build-test-library.mjs

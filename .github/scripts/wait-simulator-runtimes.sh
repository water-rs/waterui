#!/usr/bin/env bash
# Wait until CoreSimulator and simdiskimaged finish attaching preinstalled
# simulator cryptex runtimes so `xcodebuild -downloadComponent MetalToolchain`
# and `simctl` simulator setup do not race with runtime mounts or deletions.

set -euo pipefail

require_ios=0
timeout_seconds=120

while [[ $# -gt 0 ]]; do
  case "$1" in
    --require-ios)
      require_ios=1
      shift
      ;;
    --timeout)
      if [[ $# -lt 2 || ! "$2" =~ ^[0-9]+$ ]]; then
        echo "::error::--timeout requires a non-negative integer argument" >&2
        exit 2
      fi
      timeout_seconds="$2"
      shift 2
      ;;
    *)
      echo "::error::Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# Touch CoreSimulator first so CoreSimulatorService / simdiskimaged initialize
# before reading runtime mount states.
xcrun simctl list runtimes >/dev/null 2>&1 || true

python3 - "$require_ios" "$timeout_seconds" <<'PY'
import json
import os
import subprocess
import sys
import time

require_ios = sys.argv[1] == "1"
timeout_seconds = int(sys.argv[2])
deadline = time.monotonic() + timeout_seconds
poll_interval = 2.0


def snapshot():
    proc = subprocess.run(
        ["xcrun", "simctl", "runtime", "list", "-j"],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        return None, f"simctl runtime list exited {proc.returncode}: {proc.stderr.strip()}"
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        return None, f"invalid JSON from simctl runtime list: {exc}"
    if not isinstance(data, dict):
        return None, f"unexpected JSON root type: {type(data).__name__}"
    return data, None


def evaluate(data):
    if not data:
        if require_ios:
            return False, "no CoreSimulator runtimes registered yet"
        return True, "no CoreSimulator runtimes present"

    transitional = []
    ready_ios = 0
    for uid, entry in data.items():
        if not isinstance(entry, dict):
            continue
        state = entry.get("state", "Unknown")
        platform = entry.get("platformIdentifier", "")
        mount_path = entry.get("mountPath") or ""
        identifier = entry.get("runtimeIdentifier", uid)

        # `Unusable` duplicates (such as Cryptex Mount Preferred leftovers)
        # never transition to Ready; wait only for active mount/delete states
        # or Ready runtimes whose cryptex mount directory is not visible yet.
        if state == "Unusable":
            continue
        if state != "Ready":
            transitional.append(f"{identifier} ({uid}): state={state}")
            continue
        if not mount_path or not os.path.isdir(mount_path):
            transitional.append(
                f"{identifier} ({uid}): Ready without mountPath directory ({mount_path!r})"
            )
            continue
        if platform == "com.apple.platform.iphonesimulator":
            ready_ios += 1

    if transitional:
        return False, "; ".join(transitional)
    if require_ios and ready_ios == 0:
        return False, "no Ready iOS simulator runtime with a mounted cryptex yet"
    return True, f"settled ({len(data)} entries, ready_ios={ready_ios})"


last_reason = "not checked"
attempt = 0
while True:
    attempt += 1
    data, err = snapshot()
    if data is not None:
        ok, last_reason = evaluate(data)
        if ok:
            print(f"Simulator runtimes {last_reason} after {attempt} probe(s).", file=sys.stderr)
            sys.exit(0)
    else:
        last_reason = err

    if time.monotonic() >= deadline:
        level = "error" if require_ios else "warning"
        print(
            f"::{level}::Timed out after {timeout_seconds}s waiting for CoreSimulator runtimes: {last_reason}",
            file=sys.stderr,
        )
        diag = subprocess.run(
            ["xcrun", "simctl", "runtime", "list"],
            capture_output=True,
            text=True,
            check=False,
        )
        if diag.stdout:
            print(diag.stdout, file=sys.stderr)
        sys.exit(1 if require_ios else 0)
    time.sleep(poll_interval)
PY

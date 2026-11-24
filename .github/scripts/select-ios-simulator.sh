#!/usr/bin/env bash
# Wait for iOS simulator runtimes to settle, then print either a simulator UDID
# (`--model-regex`) or an iOS runtime identifier (`--runtime-for-device`),
# preferring Ready runtimes without an Unusable/Deleting duplicate entry.

set -euo pipefail

mode=""
target=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --model-regex)
      if [[ $# -lt 2 ]]; then
        echo "::error::--model-regex requires a pattern" >&2
        exit 2
      fi
      mode="device"
      target="$2"
      shift 2
      ;;
    --runtime-for-device)
      if [[ $# -lt 2 ]]; then
        echo "::error::--runtime-for-device requires a CoreSimulator device type identifier" >&2
        exit 2
      fi
      mode="runtime"
      target="$2"
      shift 2
      ;;
    *)
      echo "::error::Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "${mode}" || -z "${target}" ]]; then
  echo "::error::Usage: $0 (--model-regex <pattern> | --runtime-for-device <device-type>)" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"${script_dir}/wait-simulator-runtimes.sh" --require-ios

python3 - "$mode" "$target" <<'PY'
import json
import os
import re
import subprocess
import sys

mode = sys.argv[1]
target = sys.argv[2]


def run_json(*args):
    proc = subprocess.run(
        ["xcrun", "simctl", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        print(f"::error::xcrun simctl {' '.join(args)} failed: {proc.stderr.strip()}", file=sys.stderr)
        sys.exit(1)
    return json.loads(proc.stdout)


def version_key(version):
    parts = []
    for piece in str(version).split("."):
        try:
            parts.append((0, int(piece)))
        except ValueError:
            parts.append((1, piece))
    return tuple(parts)


runtime_states = run_json("runtime", "list", "-j")
runtimes_list = run_json("list", "runtimes", "--json").get("runtimes", [])

tainted = set()
ready_ios = {}
for uid, entry in runtime_states.items():
    if not isinstance(entry, dict):
        continue
    if entry.get("platformIdentifier") != "com.apple.platform.iphonesimulator":
        continue
    identifier = entry.get("runtimeIdentifier")
    if not identifier:
        continue
    state = entry.get("state")
    mount_path = entry.get("mountPath") or ""
    if state != "Ready":
        tainted.add(identifier)
        continue
    if mount_path and os.path.isdir(mount_path):
        ready_ios[identifier] = entry
    else:
        tainted.add(identifier)

clean_ios = {k: v for k, v in ready_ios.items() if k not in tainted}
available_runtimes = {
    rt["identifier"]: rt
    for rt in runtimes_list
    if rt.get("platform") == "iOS" and rt.get("isAvailable") and rt.get("identifier") in ready_ios
}

if tainted:
    print(
        f"Note: ignoring tainted iOS runtime(s) when a clean runtime is available: {sorted(tainted)}",
        file=sys.stderr,
    )

if mode == "runtime":
    candidates = [
        rt
        for rt in available_runtimes.values()
        if any(dt.get("identifier") == target for dt in rt.get("supportedDeviceTypes", []))
    ]
    if not candidates:
        print(f"::error::No available Ready iOS runtime supports {target}.", file=sys.stderr)
        sys.exit(1)
    candidates.sort(
        key=lambda rt: (1 if rt["identifier"] in clean_ios else 0, version_key(rt.get("version", ""))),
    )
    chosen = candidates[-1]
    identifier = chosen["identifier"]
    info = ready_ios.get(identifier, {})
    print(
        f"Selected iOS runtime {identifier} (version={chosen.get('version')}, build={info.get('build')}, clean={identifier in clean_ios}, mountPath={info.get('mountPath')})",
        file=sys.stderr,
    )
    print(identifier)
    sys.exit(0)

devices_by_runtime = run_json("list", "devices", "available", "--json").get("devices", {})
pattern = re.compile(target)
candidates = []
for runtime_id, devices in devices_by_runtime.items():
    if runtime_id not in available_runtimes:
        continue
    rt_meta = available_runtimes[runtime_id]
    is_clean = runtime_id in clean_ios
    for dev in devices:
        if not dev.get("isAvailable", True):
            continue
        name = dev.get("name", "")
        if not pattern.search(name):
            continue
        candidates.append((
            1 if is_clean else 0,
            version_key(rt_meta.get("version", "")),
            dev.get("state") == "Booted",
            dev,
            rt_meta,
            is_clean,
        ))

if not candidates:
    print(
        f"::error::No available iOS simulator matching /{target}/ on a Ready mounted runtime.",
        file=sys.stderr,
    )
    sys.exit(1)

candidates.sort(key=lambda item: (item[0], item[1], item[2], item[3].get("udid", "")))
_, _, _, chosen_dev, chosen_rt, is_clean = candidates[-1]
info = ready_ios.get(chosen_rt["identifier"], {})
print(
    f"Selected iOS simulator {chosen_dev['name']} ({chosen_dev['udid']}) on {chosen_rt['identifier']} (version={chosen_rt.get('version')}, build={info.get('build')}, clean={is_clean}, mountPath={info.get('mountPath')})",
    file=sys.stderr,
)
print(chosen_dev["udid"])
PY

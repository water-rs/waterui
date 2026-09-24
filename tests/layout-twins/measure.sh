#!/usr/bin/env bash
# Layout twins measurement driver.
#
# Runs the SwiftUI twin app and the WaterUI twin app (hosted in-process by the
# TwinsTests harness, which links the packaged libwaterui_app.a) and collects
# per-launch JSON frame dumps under results/<side>/<os>-s<scale>/<case>.json.
#
# Usage:
#   measure.sh ios-sim <device-udid>   # iOS simulator (scale from the device)
#   measure.sh macos                   # macOS, current machine
#
# Environment:
#   BACKEND_PATH — apple-backend checkout to build/link against. Default:
#                  <repo>/../apple-backend or ~/repos/apple-backend.
#   SIDES — space-separated subset of "swiftui waterui" to run (default both).
set -euo pipefail

export PATH="${HOME}/.cargo/bin:${PATH}"

TWINS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${TWINS_DIR}/../.." && pwd)"
MODE="${1:?usage: measure.sh ios-sim <udid> | macos}"

SWIFTUI_DIR="${TWINS_DIR}/swiftui"
WATERUI_DIR="${TWINS_DIR}/waterui"
HARNESS_DIR="${TWINS_DIR}/harness"
RESULTS="${TWINS_DIR}/results"
CASES=(all a12.scroll a12.ignore a12.bar)
BUNDLE_ID="dev.twins.swiftui"

mkdir -p "${RESULTS}"

# Extract the TWINS_JSON_BEGIN..END block from a log file into $2.
extract_json() {
  sed -n '/TWINS_JSON_BEGIN/,/TWINS_JSON_END/p' "$1" | sed '1d;$d' > "$2"
  [[ -s "$2" ]]
}

# --------------------------------------------------------------------------
# SwiftUI twin: `xcodebuild build` produces a bare executable; wrap it in a
# minimal .app for the simulator, run it directly on macOS.
# --------------------------------------------------------------------------
swiftui_ios() {
  local udid="$1" dd="$2" scale="$3" osver="$4"
  (cd "${SWIFTUI_DIR}" && xcodebuild -scheme LayoutTwins \
    -destination "platform=iOS Simulator,id=${udid}" \
    -derivedDataPath "${dd}" build)
  local bin="${dd}/Build/Products/Debug-iphonesimulator/LayoutTwins"
  [[ -x "${bin}" ]] || { echo "swiftui build produced no binary" >&2; return 1; }

  local app="${dd}/LayoutTwins.app"
  rm -rf "${app}"
  mkdir -p "${app}"
  cp "${bin}" "${app}/LayoutTwins"
  cat > "${app}/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>LayoutTwins</string>
  <key>CFBundleIdentifier</key><string>dev.twins.swiftui</string>
  <key>CFBundleName</key><string>LayoutTwins</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>DTPlatformName</key><string>iphonesimulator</string>
  <key>MinimumOSVersion</key><string>26.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>UILaunchScreen</key><dict/>
  <key>UISupportedInterfaceOrientations</key>
  <array><string>UIInterfaceOrientationPortrait</string></array>
</dict>
</plist>
PLIST
  codesign --force --sign - "${app}" >/dev/null 2>&1 || true
  xcrun simctl install "${udid}" "${app}"

  local outdir="${RESULTS}/swiftui/ios${4}-s${scale}"
  mkdir -p "${outdir}"
  local c log
  for c in "${CASES[@]}"; do
    echo "== swiftui ios case ${c} =="
    log="${dd}/swiftui-${c}.log"
    xcrun simctl terminate "${udid}" "${BUNDLE_ID}" 2>/dev/null || true
    SIMCTL_CHILD_TWINS_CASE="${c}" SIMCTL_CHILD_TWINS_EXIT=1 \
      xcrun simctl launch --console-pty "${udid}" "${BUNDLE_ID}" \
      >"${log}" 2>&1 || true
    if extract_json "${log}" "${outdir}/${c}.json"; then
      echo "   wrote ${outdir}/${c}.json"
    else
      echo "   FAILED to extract json for ${c} (see ${log})" >&2
    fi
  done
}

swiftui_macos() {
  local dd="$1" scale="$2"
  (cd "${SWIFTUI_DIR}" && xcodebuild -scheme LayoutTwins \
    -destination 'platform=macOS' \
    -derivedDataPath "${dd}" build)
  local bin="${dd}/Build/Products/Debug/LayoutTwins"
  [[ -x "${bin}" ]] || { echo "swiftui build produced no binary" >&2; return 1; }

  local outdir="${RESULTS}/swiftui/macos$(sw_vers -productVersion)-s${scale}"
  mkdir -p "${outdir}"
  local c log
  for c in "${CASES[@]}"; do
    echo "== swiftui macos case ${c} =="
    log="${dd}/swiftui-mac-${c}.log"
    (cd /tmp && TWINS_CASE="${c}" TWINS_EXIT=1 "${bin}" >"${log}" 2>&1) || true
    if extract_json "${log}" "${outdir}/${c}.json"; then
      echo "   wrote ${outdir}/${c}.json"
    else
      echo "   FAILED to extract json for ${c} (see ${log})" >&2
    fi
  done
}

# --------------------------------------------------------------------------
# WaterUI twin: `water package` produces the .app and libwaterui_app.a; the
# TwinsTests harness hosts the archive in-process and dumps frames.
# --------------------------------------------------------------------------
waterui_package() {
  local platform="$1" log app_path archive
  log="$(mktemp)"
  (cd "${WATERUI_DIR}" && water package --platform "${platform}" \
    --backend apple 2>&1) | tee "${log}" >&2
  app_path="$(sed -n 's/.*Packaged at //p' "${log}" \
    | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r' | tail -n1)"
  rm -f "${log}"
  [[ -n "${app_path}" ]] || { echo "water package produced no bundle" >&2; return 1; }
  # `water package` stages libwaterui_app.a beside the .app inside the managed
  # DerivedData products dir, not beside the renamed copy `Packaged at` names.
  local products="Debug"
  [[ "${platform}" == "ios-simulator" ]] && products="Debug-iphonesimulator"
  archive="$(dirname "${app_path}")/libwaterui_app.a"
  if [[ ! -f "${archive}" ]]; then
    archive="$(find "${HOME}/.water/build_cache${WATERUI_DIR}" \
      -path "*/managed_backends/apple/DerivedData/Build/Products/${products}/libwaterui_app.a" \
      -print -quit 2>/dev/null)"
  fi
  [[ -n "${archive}" && -f "${archive}" ]] \
    || { echo "no libwaterui_app.a for ${app_path}" >&2; return 1; }
  # The app archive references __swift_bridge__ and helper symbols defined in
  # the sibling archives the real app links; pass the whole products dir.
  local adir
  adir="$(dirname "${archive}")"
  for a in "${adir}"/*.a; do
    printf '%s\n' "${a}"
  done
}

# Case names map to test methods: a case needs a fresh process, and env vars
# do not reach a simulator test runner, so the fixture is selected by the
# method name and picked with -only-testing.
case_method() {
  case "$1" in
    all)         echo testCaseAll ;;
    a12.scroll)  echo testCaseA12Scroll ;;
    a12.ignore)  echo testCaseA12Ignore ;;
    a12.bar)     echo testCaseA12Bar ;;
    *) return 1 ;;
  esac
}

waterui_run() {
  local dest="$1" dd="$2" archives="$3" outdir="$4" platform="$5"
  # Every archive in the backend products dir is linked so the package's
  # ___swift_bridge__ helpers resolve at link time (no dynamic_lookup).
  local ldflags=""
  local a
  for a in ${archives}; do ldflags+=" ${a}"; done
  # The Rust stack pulls in SCNetwork* on macOS only.
  [[ "${platform}" == "macos" ]] && ldflags+=" -framework SystemConfiguration"
  mkdir -p "${outdir}" "${dd}"
  local c log
  for c in "${CASES[@]}"; do
    echo "== waterui ${platform} case ${c} =="
    log="${dd}/waterui-${platform}-${c}.log"
    (cd "${HARNESS_DIR}" && \
      TWINS_OUT="/tmp/twins-waterui-${c}.json" \
      xcodebuild test -scheme LayoutTwinsHarness-Package \
        -destination "${dest}" \
        -derivedDataPath "${dd}" \
        -only-testing:TwinsTests/TwinsTests/"$(case_method "${c}")" \
        OTHER_LDFLAGS="${ldflags}" \
        >"${log}" 2>&1) || true
    if extract_json "${log}" "${outdir}/${c}.json"; then
      echo "   wrote ${outdir}/${c}.json"
    else
      echo "   FAILED for ${c} (see ${log})" >&2
    fi
  done
}

# Ensure the harness' apple-backend path dependency resolves.
BACKEND="${BACKEND_PATH:-}"
if [[ -z "${BACKEND}" ]]; then
  for cand in "${REPO_ROOT}/../apple-backend" "${HOME}/repos/apple-backend"; do
    if [[ -d "${cand}/Sources/WaterUI" ]]; then BACKEND="${cand}"; break; fi
  done
fi
[[ -n "${BACKEND}" && -d "${BACKEND}/Sources/WaterUI" ]] \
  || { echo "apple-backend checkout not found; set BACKEND_PATH" >&2; exit 1; }
# Harness Package.swift resolves ../../../../apple-backend.
[[ -e "${REPO_ROOT}/../apple-backend" ]] \
  || ln -sfn "${BACKEND}" "${REPO_ROOT}/../apple-backend"
# A playground picks up a local backend only via <waterui_path>/backends/apple.
[[ -e "${REPO_ROOT}/backends/apple" ]] \
  || ln -sfn "${BACKEND}" "${REPO_ROOT}/backends/apple"

if [[ "${MODE}" == "ios-sim" ]]; then
  UDID="${2:?usage: measure.sh ios-sim <udid>}"
  read -r DEVNAME OSVER <<<"$(xcrun simctl list devices -j | python3 -c "
import json, re, sys
d = json.load(sys.stdin)['devices']
for os_key, devs in d.items():
    for dev in devs:
        if dev['udid'] == '${UDID}':
            m = re.search(r'SimRuntime\.iOS-(\d+)-(\d+)', os_key)
            ver = m.group(1) + '.' + m.group(2) if m else os_key
            print(dev.get('name', '?').replace(' ', '_'), ver); sys.exit(0)
")"
  case "${DEVNAME}" in
    iPad*) SCALE=2 ;; *) SCALE=3 ;;
  esac
  echo "device ${DEVNAME} ${UDID} ios${OSVER} -> scale ${SCALE}"

  DD="${TWINS_DIR}/.build-ios"
  case " ${SIDES:-swiftui waterui} " in *" swiftui "*)
    swiftui_ios "${UDID}" "${DD}" "${SCALE}" "${OSVER}" ;; esac
  case " ${SIDES:-swiftui waterui} " in *" waterui "*)
    echo "-- packaging waterui twin (ios-simulator) --"
    ARCHIVES="$(waterui_package ios-simulator | tail -n+1 | grep '\.a$')"
    waterui_run "platform=iOS Simulator,id=${UDID}" "${DD}-harness" \
      "${ARCHIVES}" "${RESULTS}/waterui/ios${OSVER}-s${SCALE}" ios "${UDID}" ;; esac
elif [[ "${MODE}" == "macos" ]]; then
  SCALE=1
  system_profiler SPDisplaysDataType 2>/dev/null | grep -qi retina && SCALE=2
  DD="${TWINS_DIR}/.build-macos"
  case " ${SIDES:-swiftui waterui} " in *" swiftui "*)
    swiftui_macos "${DD}" "${SCALE}" ;; esac
  case " ${SIDES:-swiftui waterui} " in *" waterui "*)
    echo "-- packaging waterui twin (macos) --"
    ARCHIVES="$(waterui_package macos | tail -n+1 | grep '\.a$')"
    waterui_run "platform=macOS" "${DD}-harness" \
      "${ARCHIVES}" "${RESULTS}/waterui/macos$(sw_vers -productVersion)-s${SCALE}" macos "" ;; esac
fi

echo "done — results under ${RESULTS}"

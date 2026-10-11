#!/bin/bash
# Builds the SwiftUI reference host as a minimal .app bundle — no Xcode project
# required, so it compiles in seconds with just the toolchain.
#
# Usage: build-reference-host.sh <macos|ios> <output-dir>
#
# Produces <output-dir>/E2EReference.app. Launch it with
#   -E2EExample <name> -E2ETitle <window title>
# via `open -W --args` (macOS) or `simctl launch` (iOS).

set -euo pipefail

PLATFORM="${1:?usage: build-reference-host.sh <macos|ios> <output-dir>}"
OUT_DIR="${2:?usage: build-reference-host.sh <macos|ios> <output-dir>}"
ROOT="$(cd "$(dirname "$0")" && pwd)"
APP="$OUT_DIR/E2EReference.app"

case "$PLATFORM" in
  macos)
    BIN_DIR="$APP/Contents/MacOS"
    PLIST="$APP/Contents/Info.plist"
    PLATFORMS=("MacOSX")
    MIN_KEY="LSMinimumSystemVersion"
    MIN_VERSION="$(sw_vers -productVersion)"
    ;;
  ios)
    BIN_DIR="$APP"
    PLIST="$APP/Info.plist"
    PLATFORMS=("iPhoneSimulator")
    MIN_KEY="MinimumOSVersion"
    # The app must install on the booted simulator, so the floor is the
    # device *runtime* version, not the toolchain SDK — the SDK can be newer
    # than the installed simulator runtime, and installd rejects a bundle
    # whose MinimumOSVersion exceeds the runtime ("Have 26.2; need 26.5").
    # Without a UDID, use the newest iOS runtime capped at the SDK version —
    # the same device a default `simctl` pick would boot. The value also
    # drives the swiftc deployment target, so it can never drop below what
    # the twins' newest APIs require.
    MIN_VERSION="$(python3 - "$(xcrun --sdk iphonesimulator --show-sdk-platform-version)" <<'PY'
import json, os, subprocess, sys
sdk_version = [int(x) for x in sys.argv[1].split(".")]
udid = os.environ.get("SIMULATOR_UDID", "")
devices = json.loads(subprocess.check_output(
    ["xcrun", "simctl", "list", "-j", "devices", "available"]))["devices"]
runtimes = {r["identifier"]: r.get("version", "")
            for r in json.loads(subprocess.check_output(
                ["xcrun", "simctl", "list", "-j", "runtimes", "available"]))["runtimes"]
            if r.get("isAvailable", True)}
version = ""
if udid:
    for runtime_id, group in devices.items():
        if any(d.get("udid") == udid for d in group):
            version = runtimes.get(runtime_id, "")
            break
if not version:
    candidates = [v for ident, v in runtimes.items()
                  if v and "iOS" in ident]
    if candidates:
        version = max(candidates, key=lambda v: [int(x) for x in v.split(".")])
if version and [int(x) for x in version.split(".")] > sdk_version:
    version = ".".join(str(x) for x in sdk_version)
print(version)
PY
)"
    [[ -z "${MIN_VERSION}" ]] && MIN_VERSION="$(xcrun --sdk iphonesimulator --show-sdk-platform-version)"
    ;;
  *)
    echo "unknown platform: $PLATFORM" >&2
    exit 1
    ;;
esac

rm -rf "$APP"
mkdir -p "$BIN_DIR"

SOURCES=()
while IFS= read -r file; do
  SOURCES+=("$file")
done < <(find "$ROOT/Sources" -name '*.swift' | sort)

if [[ "$PLATFORM" == "ios" ]]; then
  swiftc -O -o "$BIN_DIR/E2EReference" ${SOURCES[@]+"${SOURCES[@]}"} \
    -target "arm64-apple-ios${MIN_VERSION}-simulator" \
    -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)"
else
  swiftc -O -o "$BIN_DIR/E2EReference" ${SOURCES[@]+"${SOURCES[@]}"}
fi

mkdir -p "$(dirname "$PLIST")"
printf '<?xml version="1.0" encoding="UTF-8"?>\n<plist version="1.0"><dict></dict></plist>\n' > "$PLIST"
/usr/libexec/PlistBuddy \
  -c "Add :CFBundleExecutable string E2EReference" \
  -c "Add :CFBundleIdentifier string dev.waterui.E2EReference" \
  -c "Add :CFBundleName string E2EReference" \
  -c "Add :CFBundlePackageType string APPL" \
  -c "Add :CFBundleShortVersionString string 1.0" \
  -c "Add :CFBundleVersion string 1" \
  -c "Add :$MIN_KEY string $MIN_VERSION" \
  -c "Add :CFBundleSupportedPlatforms array" \
  -c "Add :CFBundleSupportedPlatforms:0 string ${PLATFORMS[0]}" \
  "$PLIST"

if [[ "$PLATFORM" == "ios" ]]; then
  # Without any launch-screen declaration iOS runs the app in compatibility
  # letterbox (a centered card with black bars) instead of fullscreen — the
  # parity capture would then never match the packaged example. An empty
  # UILaunchScreen dict opts into the system launch screen and full-bleed
  # sizing, exactly what the CLI's own scaffold emits.
  /usr/libexec/PlistBuddy \
    -c "Add :UILaunchScreen dict" \
    -c "Add :UIDeviceFamily array" \
    -c "Add :UIDeviceFamily:0 integer 1" \
    "$PLIST"
  # The scene manifest the CLI's scaffold declares: UIKit refuses to launch an
  # app built with the iOS 27 SDK that still creates its window from the app
  # delegate. The delegate is named by its Objective-C name (see
  # ReferenceHost.swift) so the manifest needs no module name.
  /usr/libexec/PlistBuddy \
    -c "Add :UIApplicationSceneManifest dict" \
    -c "Add :UIApplicationSceneManifest:UIApplicationSupportsMultipleScenes bool false" \
    -c "Add :UIApplicationSceneManifest:UISceneConfigurations dict" \
    -c "Add :UIApplicationSceneManifest:UISceneConfigurations:UIWindowSceneSessionRoleApplication array" \
    -c "Add :UIApplicationSceneManifest:UISceneConfigurations:UIWindowSceneSessionRoleApplication:0 dict" \
    -c "Add :UIApplicationSceneManifest:UISceneConfigurations:UIWindowSceneSessionRoleApplication:0:UISceneConfigurationName string Default" \
    -c "Add :UIApplicationSceneManifest:UISceneConfigurations:UIWindowSceneSessionRoleApplication:0:UISceneDelegateClassName string SceneDelegate" \
    "$PLIST"
fi

# Ad-hoc sign so the arm64 binary is launchable on Apple Silicon.
codesign --force --sign - "$APP" >/dev/null 2>&1 || codesign --force --sign - "$BIN_DIR/E2EReference"

echo "Built $APP"

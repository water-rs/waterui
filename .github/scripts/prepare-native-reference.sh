#!/usr/bin/env bash
# Produces the SwiftUI reference metrics the native suite's migration::uikit
# cases compare against. stdout is the JSON path to export as
# WATERUI_REFERENCE_METRICS; diagnostics use stderr.
#
#   WATERUI_REFERENCE_DESTINATION=simulator|catalyst   required
#   WATERUI_REFERENCE_STEP=all|build|run               default all
#   WATERUI_IOS_SIM_UDID=<udid>                        simulator run only
#
# usage: prepare-native-reference.sh <artifact-directory>
#
# `build` compiles and signs <artifact-directory>/NativeReference.app (the
# simulator build needs no device); `run` executes an existing bundle, so a
# Catalyst bundle built on one Mac can be copied to another and run there.
# Completion is the reference host's exit after all four native layout
# callbacks have produced their values. No run-loop sleeps or retry passes.
set -euo pipefail
destination="${WATERUI_REFERENCE_DESTINATION:?set WATERUI_REFERENCE_DESTINATION to simulator or catalyst}"
step="${WATERUI_REFERENCE_STEP:-all}"
output="${1:?usage: prepare-native-reference.sh <artifact-directory>}"
case "$destination" in
  simulator | catalyst) ;;
  *) echo "unknown WATERUI_REFERENCE_DESTINATION '$destination' (simulator or catalyst)" >&2; exit 2 ;;
esac
case "$step" in
  all | build | run) ;;
  *) echo "unknown WATERUI_REFERENCE_STEP '$step' (all, build or run)" >&2; exit 2 ;;
esac
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
host="$repo_root/backends/apple/Tests/ReferenceHost"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
app="$output/NativeReference.app"
# Keep --sdk on xcrun: without it xcrun exports the default macOS SDKROOT,
# which clang reads ahead of -sdk and warns -Wincompatible-sysroot.
swiftc_common=(-parse-as-library -swift-version 6)

build_simulator() {
  local sdk
  sdk="$(xcrun --sdk iphonesimulator --show-sdk-path)"
  mkdir -p "$app"
  xcrun --sdk iphonesimulator swiftc "${swiftc_common[@]}" -sdk "$sdk" \
    -target arm64-apple-ios26.0-simulator \
    "$host/ReferenceHost.swift" -o "$app/NativeReference" >&2
  cp "$host/Info.plist" "$app/Info.plist"
  codesign --force --sign - "$app" >&2
}

build_catalyst() {
  local sdk support plist="$app/Contents/Info.plist"
  sdk="$(xcrun --sdk macosx --show-sdk-path)"
  support="$sdk/System/iOSSupport"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  xcrun --sdk macosx swiftc "${swiftc_common[@]}" -sdk "$sdk" \
    -target arm64-apple-ios26.0-macabi \
    -F "$support/System/Library/Frameworks" -I "$support/usr/lib/swift" \
    "$host/ReferenceHost.swift" -o "$app/Contents/MacOS/NativeReference" >&2
  # Same plist as the simulator host, minus the iOS-only keys, plus the macOS
  # bundle keys a Catalyst app needs.
  cp "$host/Info.plist" "$plist"
  /usr/libexec/PlistBuddy \
    -c 'Delete :LSRequiresIPhoneOS' \
    -c 'Delete :UILaunchScreen' \
    -c 'Add :CFBundleSupportedPlatforms array' \
    -c 'Add :CFBundleSupportedPlatforms:0 string MacOSX' \
    -c 'Add :LSMinimumSystemVersion string 26.0' \
    -c 'Add :NSHighResolutionCapable bool true' \
    "$plist" >&2
  codesign --force --sign - "$app" >&2
}

run_simulator() {
  local device="${WATERUI_IOS_SIM_UDID:?set the owned simulator UDID}" container
  xcrun simctl bootstatus "$device" -b >&2
  xcrun simctl install "$device" "$app" >&2
  container="$(xcrun simctl get_app_container "$device" dev.waterui.NativeReference data)"
  rm -f "$container/Documents/reference.json"
  SIMCTL_CHILD_WATERUI_REFERENCE_OUTPUT="$container/Documents/reference.json" \
    xcrun simctl launch --terminate-running-process --console "$device" dev.waterui.NativeReference >&2
  test -s "$container/Documents/reference.json"
  cp "$container/Documents/reference.json" "$output/reference.json"
}

run_catalyst() {
  # Running the executable directly keeps this script attached to the host's
  # exit; it needs a macOS GUI session for the UIKit scene.
  rm -f "$output/reference.json"
  WATERUI_REFERENCE_OUTPUT="$output/reference.json" "$app/Contents/MacOS/NativeReference" >&2
  test -s "$output/reference.json"
}

if [ "$step" != run ]; then "build_$destination"; fi
if [ "$step" != build ]; then
  "run_$destination"
  printf '%s\n' "$output/reference.json"
fi

#!/usr/bin/env bash
# Build/run once per owned simulator before the Rust native suite. stdout is
# the JSON path to export as WATERUI_REFERENCE_METRICS; diagnostics use stderr.
set -euo pipefail
device="${WATERUI_IOS_SIM_UDID:?set the owned simulator UDID}"
output="${1:?usage: prepare-native-reference.sh <artifact-directory>}"
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$output/NativeReference.app"
output="$(cd "$output" && pwd)"
app="$output/NativeReference.app"
sdk="$(xcrun --sdk iphonesimulator --show-sdk-path)"
# Keep --sdk on xcrun: without it xcrun exports the default macOS SDKROOT,
# which clang reads ahead of -sdk and warns -Wincompatible-sysroot.
xcrun --sdk iphonesimulator swiftc -parse-as-library -swift-version 6 -sdk "$sdk" \
  -target arm64-apple-ios26.0-simulator \
  "$repo_root/backends/apple/Tests/ReferenceHost/ReferenceHost.swift" -o "$app/NativeReference" >&2
cp "$repo_root/backends/apple/Tests/ReferenceHost/Info.plist" "$app/Info.plist"
codesign --force --sign - "$app" >&2
xcrun simctl bootstatus "$device" -b >&2
xcrun simctl install "$device" "$app" >&2
container="$(xcrun simctl get_app_container "$device" dev.waterui.NativeReference data)"
rm -f "$container/Documents/reference.json"
# Completion is the reference host's exit after all four native layout
# callbacks have produced their values. No run-loop sleeps or retry passes.
xcrun simctl launch --terminate-running-process --console "$device" dev.waterui.NativeReference >&2
test -s "$container/Documents/reference.json"
cp "$container/Documents/reference.json" "$output/reference.json"
printf '%s\n' "$output/reference.json"

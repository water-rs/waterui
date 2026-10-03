#!/usr/bin/env bash
# Smoke-check the iOS half of cocoa-ui on a caller-owned simulator:
# build the smoke_ios example, wrap it in a minimal app bundle whose
# Info.plist names the kit's SceneDelegate class, install and launch it,
# streaming its output. The app exits itself after one layout pass.
set -euo pipefail

member_root="$(cd "$(dirname "$0")/.." && pwd)"

# The caller owns the booted simulator this script installs into — a bare
# `booted` or a discovered device could belong to someone else's run.
: "${WATERUI_IOS_SIM_UDID:?Set WATERUI_IOS_SIM_UDID to the UDID of a booted simulator owned by the caller (xcrun simctl list devices)}"

# The compiler-artifact receipt names the exact produced executable, so no
# target-dir layout is assumed: workspace-root, per-member and overridden
# CARGO_TARGET_DIR layouts all resolve through cargo's own record. `jq -s`
# consumes the complete stream and requires exactly one matching example
# artifact inside JSON — zero or several is a jq error, which pipefail
# turns into a failed build receipt.
executable="$(cargo build \
  --manifest-path "${member_root}/Cargo.toml" \
  --example smoke_ios \
  --target aarch64-apple-ios-sim \
  --message-format=json-render-diagnostics \
  | jq -s -e -r '
      [.[] | select(.reason == "compiler-artifact"
                    and (.target.kind | index("example") != null)
                    and .target.name == "smoke_ios"
                    and .executable != null)]
      | if length == 1 then .[0].executable
        else error("expected exactly one smoke_ios example executable, got \(length)")
        end')"

app_parent="$(mktemp -d)"
trap 'rm -rf "${app_parent}"' EXIT
APP="${app_parent}/Smoke.app"
mkdir -p "$APP"
cp "${executable}" "$APP/Smoke"
cat > "$APP/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>Smoke</string>
    <key>CFBundleIdentifier</key>
    <string>dev.cocoa-ui.smoke</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Smoke</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSRequiresIPhoneOS</key>
    <true/>
    <key>UILaunchScreen</key>
    <dict/>
    <key>UIRequiredDeviceCapabilities</key>
    <array>
        <string>arm64</string>
    </array>
    <key>UISupportedInterfaceOrientations</key>
    <array>
        <string>UIInterfaceOrientationPortrait</string>
    </array>
    <key>UIApplicationSceneManifest</key>
    <dict>
        <key>UIApplicationSupportsMultipleScenes</key>
        <false/>
        <key>UISceneConfigurations</key>
        <dict>
            <key>UIWindowSceneSessionRoleApplication</key>
            <array>
                <dict>
                    <key>UISceneConfigurationName</key>
                    <string>Default Configuration</string>
                    <key>UISceneDelegateClassName</key>
                    <string>SceneDelegate</string>
                </dict>
            </array>
        </dict>
    </dict>
</dict>
</plist>
EOF
codesign --force --sign - --timestamp=none "$APP"
xcrun simctl install "${WATERUI_IOS_SIM_UDID}" "$APP"
xcrun simctl launch --console-pty "${WATERUI_IOS_SIM_UDID}" dev.cocoa-ui.smoke

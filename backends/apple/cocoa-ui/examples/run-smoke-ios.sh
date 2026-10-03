#!/bin/sh
# Smoke-check the iOS half of cocoa-ui on the booted simulator:
# build the smoke_ios example, wrap it in a minimal app bundle whose
# Info.plist names the kit's SceneDelegate class, install and launch it,
# streaming its output. The app exits itself after one layout pass.
set -eu

cd "$(dirname "$0")/.."
cargo build --example smoke_ios --target aarch64-apple-ios-sim

APP="$(mktemp -d)/Smoke.app"
mkdir -p "$APP"
cp ../target/aarch64-apple-ios-sim/debug/examples/smoke_ios "$APP/Smoke"
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
xcrun simctl install booted "$APP"
xcrun simctl launch --console-pty booted dev.cocoa-ui.smoke

/// Scan for Android devices and emulators.
async fn scan_android_devices() -> Result<Vec<DeviceInfo>> {
    let mut results = Vec::new();

    // Scan connected devices via adb
    if let Some(adb) = find_android_tool("adb") {
        let output = tokio::process::Command::new(&adb)
            .args(["devices", "-l"])
            .output()
            .await
            .context("failed to execute adb devices")?;

        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines().skip(1) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let mut parts = trimmed.split_whitespace();
                let identifier = parts.next().unwrap_or_default();
                let state = parts.next().unwrap_or_default();
                if identifier.is_empty() || identifier == "offline" {
                    continue;
                }

                let mut name = identifier.to_string();
                let mut detail_segments = Vec::new();
                for token in parts {
                    detail_segments.push(token.to_string());
                    if let Some(rest) = token.strip_prefix("model:") {
                        if !rest.is_empty() {
                            name = rest.replace('_', " ");
                        }
                    }
                }

                let detail = if detail_segments.is_empty() {
                    None
                } else {
                    Some(detail_segments.join(" "))
                };

                results.push(DeviceInfo {
                    platform: "Android".to_string(),
                    raw_platform: Some("android-device".to_string()),
                    name,
                    identifier: identifier.to_string(),
                    kind: DeviceKind::Device,
                    state: Some(state.to_string()),
                    detail,
                });
            }
        }
    }

    // Scan emulators
    if let Some(emulator) = find_android_tool("emulator") {
        let output = tokio::process::Command::new(&emulator)
            .arg("-list-avds")
            .output()
            .await
            .context("failed to execute emulator -list-avds")?;

        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let name = line.trim();
                if name.is_empty() {
                    continue;
                }
                results.push(DeviceInfo {
                    platform: "Android".to_string(),
                    raw_platform: Some("android-emulator".to_string()),
                    name: name.to_string(),
                    identifier: name.to_string(),
                    kind: DeviceKind::Emulator,
                    state: Some("stopped".to_string()),
                    detail: None,
                });
            }
        }
    }

    Ok(results)
}

use anyhow::Result;
use clap::Args;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::process::Command;
use std::time::Duration;

#[derive(Args, Debug, Default)]
pub struct DoctorArgs {}

pub fn run(_args: DoctorArgs, pb: ProgressBar) -> Result<()> {
    pb.set_message(style("WaterUI Doctor: Checking your environment...").bold().to_string());

    let handles = vec![
        std::thread::spawn(check_rust),
        std::thread::spawn(check_swift),
        std::thread::spawn(check_android),
    ];

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.join().unwrap());
    }

    pb.finish_and_clear();
    println!("");

    for result_lines in results {
        for line in result_lines {
            println!("{}", line);
        }
    }

    println!("\n{}", style("Doctor check complete. If any checks failed, please follow the instructions to fix them.").green());
    Ok(())
}

fn check_command(name: &str, help: &str) -> String {
    match which::which(name) {
        Ok(path) => format!("  {} {}: Found at {}", style("✅").green(), name, path.display()),
        Err(_) => {
            format!("  {} {}: Not found\n    {}", style("❌").red(), name, style(help).yellow())
        }
    }
}

fn check_env_var(name: &str, help: &str) -> String {
    match std::env::var(name) {
        Ok(path) => format!("  {} {}: Set to {}", style("✅").green(), name, path),
        Err(_) => {
            format!("  {} {}: Not set\n    {}", style("❌").red(), name, style(help).yellow())
        }
    }
}

fn check_rust() -> Vec<String> {
    let mut lines = vec![format!("\n{}", style("[ Rust ]").bold())];
    lines.push(check_command("cargo", "Install Rust from https://rustup.rs"));
    lines.push(check_command("rustup", "Install Rust from https://rustup.rs"));

    if which::which("rustup").is_ok() {
        lines.push(format!("\n  {}", style("Checking for required Rust targets...").dim()));
        let output = Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output();

        if let Ok(output) = output {
            let installed_targets = String::from_utf8_lossy(&output.stdout);

            if cfg!(target_os = "macos") {
                let (required, optional) = if cfg!(target_arch = "aarch64") {
                    (
                        vec!["aarch64-apple-darwin", "aarch64-apple-ios", "aarch64-apple-ios-sim"],
                        vec![("x86_64-apple-darwin", "for cross-compiling to Intel Macs")],
                    )
                } else if cfg!(target_arch = "x86_64") {
                    (
                        vec!["x86_64-apple-darwin", "aarch64-apple-ios", "x86_64-apple-ios"],
                        vec![("aarch64-apple-darwin", "for cross-compiling to Apple Silicon Macs")],
                    )
                } else {
                    (vec![], vec![])
                };

                for target in required {
                    if installed_targets.contains(target) {
                        lines.push(format!("    {} {}", style("✅").green(), target));
                    } else {
                        lines.push(format!("    {} {}", style("❌").red(), target));
                        lines.push(format!("      {}", style(format!("Run `rustup target add {}`", target)).yellow()));
                    }
                }

                for (target, reason) in optional {
                    if installed_targets.contains(target) {
                        lines.push(format!("    {} {} (optional, {})", style("✅").green(), target, reason));
                    } else {
                        lines.push(format!("    {} {} (optional, {})", style("⚠️").yellow(), target, reason));
                    }
                }
            }

            let required_android_targets = [
                "aarch64-linux-android",
                "armv7-linux-androideabi",
                "i686-linux-android",
                "x86_64-linux-android",
            ];
            for target in required_android_targets {
                 if installed_targets.contains(target) {
                    lines.push(format!("    {} {} (for Android)", style("✅").green(), target));
                } else {
                    lines.push(format!("    {} {} (optional, for Android)", style("⚠️").yellow(), target));
                }
            }
        }
    }
    lines
}

fn check_swift() -> Vec<String> {
    if cfg!(not(target_os = "macos")) {
        return vec![];
    }
    let mut lines = vec![format!("\n{}", style("[ Swift (macOS) ]").bold())];
    lines.push(check_command("xcodebuild", "Install Xcode and command line tools (xcode-select --install)"));
    lines.push(check_command("xcrun", "Install Xcode and command line tools (xcode-select --install)"));
    lines
}

fn check_android() -> Vec<String> {
    let mut lines = vec![format!("\n{}", style("[ Android ]").bold())];
    lines.push(check_command("adb", "Install Android SDK Platform-Tools and add to PATH."));
    lines.push(check_command("emulator", "Install Android SDK command-line tools and add to PATH."));
    lines.push(check_env_var("ANDROID_HOME", "Set ANDROID_HOME environment variable to your Android SDK path."));
    lines.push(check_env_var("ANDROID_NDK_HOME", "Set ANDROID_NDK_HOME environment variable to your Android NDK path."));
    lines.push(check_env_var("JAVA_HOME", "Set JAVA_HOME environment variable to your JDK path (Java 17 or newer recommended)."));

    let java_version_cmd = if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let java_exe = std::path::Path::new(&java_home).join("bin/java");
        if java_exe.exists() {
            Some(Command::new(java_exe))
        } else {
            None
        }
    } else if which::which("java").is_ok() {
        Some(Command::new("java"))
    } else {
        None
    };

    if let Some(mut cmd) = java_version_cmd {
        let output = cmd.arg("-version").output();
        if let Ok(output) = output {
            let version_info = String::from_utf8_lossy(&output.stderr);
            if let Some(line) = version_info.lines().next() {
                lines.push(format!("  {} Java version: {}", style("✅").green(), line));
            } else {
                 lines.push(format!("  {} Could not determine Java version.", style("❌").red()));
            }
        }
    } else {
        lines.push(format!("  {} Java not found in JAVA_HOME or PATH.", style("❌").red()));
    }
    lines
}

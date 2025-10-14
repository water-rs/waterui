use std::process::Command;

fn main() {
    get_stable_version("waterui", "WATERUI_VERSION");
    get_stable_version("waterui-backend-swift", "WATERUI_BACKEND_SWIFT_VERSION");
}

fn get_stable_version(name: &str, env: &str) {
    let output = Command::new("git")
        .args([
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            &format!("{}-v*", name),
        ])
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cargo:warning=Failed to execute git describe for {}: {}. Setting empty version.", name, e);
            println!("cargo:rustc-env={}=", env);
            return;
        }
    };

    if output.status.success() {
        let version_tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let prefix = format!("{}-v", name);
        if let Some(version) = version_tag.strip_prefix(&prefix) {
            println!("cargo:rustc-env={}={}", env, version);
        } else {
            eprintln!("cargo:warning=Found tag '{}' for crate '{}' but it did not have the expected prefix '{}'. Setting empty version.", version_tag, name, prefix);
            println!("cargo:rustc-env={}=", env);
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No names found") {
            // This is an expected case when no tags are present.
            println!("cargo:warning=No stable tag found for crate '{}'. Defaulting to empty version. Use --dev for create command.", name);
        } else {
            eprintln!("cargo:warning=git describe for {} failed: {}. Setting empty version.", name, stderr);
        }
        println!("cargo:rustc-env={}=", env);
    }
}
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
            format!("{}-v*", name).as_str(),
        ])
        .output()
        .expect("Failed to get latest waterui version");

    output.status.success().then(|| {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!("cargo:rustc-env={}={}", env, version);
    });
}

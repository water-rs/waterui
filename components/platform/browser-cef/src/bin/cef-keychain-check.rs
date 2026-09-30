//! Prints whether this binary's code signature requires CEF's mock keychain.
//!
//! Inspect any signing identity without packaging an app:
//!
//! ```text
//! cargo build -p waterui-browser-cef --bin cef-keychain-check
//! codesign -s <identity> -f target/debug/cef-keychain-check
//! ./target/debug/cef-keychain-check
//! ```

#[cfg(target_os = "macos")]
fn main() {
    println!(
        "mock_keychain={}",
        waterui_browser_cef::signing::needs_mock_keychain()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("cef-keychain-check is only meaningful on macOS");
    std::process::exit(2);
}

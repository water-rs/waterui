//! Embeds the Windows icon resource when targeting Windows.
//!
//! `app-icon.ico` is staged next to this crate by the water CLI before the
//! build; the executable's taskbar and Explorer icon come from it.

fn main() {
    println!("cargo:rerun-if-changed=app-icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert!(
        std::path::Path::new("app-icon.ico").exists(),
        "app-icon.ico is missing; build through the water CLI so the app icon is staged first"
    );
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("app-icon.ico");
    resource
        .compile()
        .expect("failed to embed the Windows icon resource");
}

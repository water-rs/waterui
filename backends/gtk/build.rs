//! Build-time probes for optional GTK backend system libraries.

fn main() {
    println!("cargo:rustc-check-cfg=cfg(gtk_webkitgtk_link_available)");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_SYSROOT_DIR");

    if std::env::var_os("CARGO_FEATURE_WEBKITGTK").is_none() {
        return;
    }

    let required = ["webkitgtk-6.0", "javascriptcoregtk-6.0", "libsoup-3.0"];
    let link_ready = required.iter().all(|package| {
        pkg_config::Config::new()
            .cargo_metadata(false)
            .probe(package)
            .is_ok()
    });

    if link_ready {
        for package in required {
            let _ = pkg_config::Config::new().probe(package);
        }
        println!("cargo:rustc-cfg=gtk_webkitgtk_link_available");
    }
}

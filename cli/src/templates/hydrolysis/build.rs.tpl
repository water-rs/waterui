//! Embeds the Windows icon resource when targeting Windows.
//!
//! `app-icon.ico` is staged next to this crate by the water CLI before the
//! build; the executable's taskbar and Explorer icon come from it.

fn main() {
{% include "partials/build_script_i18n.rs.tpl" %}

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
    // The subclassing APIs hydrolysis uses (SetWindowSubclass,
    // GetWindowSubclass, RemoveWindowSubclass, DefSubclassProc) are only
    // exported by comctl32 v6, which the system binds only when the
    // executable declares the Common-Controls v6 dependency — without it
    // the loader resolves comctl32 v5.82 and the process cannot start.
    resource.set_manifest(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
          type="win32"
          name="Microsoft.Windows.Common-Controls"
          version="6.0.0.0"
          processorArchitecture="*"
          publicKeyToken="6595b64144ccf1df"
          language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>"#,
    );
    resource
        .compile()
        .expect("failed to embed the Windows icon resource");
}

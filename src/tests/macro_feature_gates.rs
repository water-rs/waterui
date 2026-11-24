//! Compile-level coverage verifying macro re-exports at `waterui` and
//! `waterui::prelude` resolve under their expected feature gates.

#![expect(
    unused_imports,
    reason = "verifies compile-time macro re-export visibility across feature combinations"
)]

#[test]
fn core_macros_resolve_from_root_and_prelude() {
    use waterui::prelude::{
        Identifiable as _, Project as _, bench as _, catalog as _, preview as _, state as _,
        text as _, ui_test as _, view as _, view_builder as _,
    };
    use waterui::{
        Identifiable as _, Project as _, bench as _, catalog as _, preview as _, state as _,
        test as _, text as _, view as _, view_builder as _,
    };
}

#[cfg(feature = "form")]
#[test]
fn form_macros_resolve_when_form_enabled() {
    use waterui::prelude::{FormBuilder as _, form as _};
    use waterui::{FormBuilder as _, form as _};
}

#[cfg(feature = "webview")]
#[test]
fn webview_macros_resolve_when_webview_enabled() {
    use waterui::prelude::{eval as _, exec as _, exec_file as _, js_api as _, js_file as _};
    use waterui::{eval as _, exec as _, exec_file as _, js_api as _, js_file as _};
}

#[cfg(feature = "ts")]
#[test]
fn ts_macros_resolve_when_ts_enabled() {
    use waterui::prelude::{TsProps as _, TsType as _, tsx as _};
    use waterui::{TsProps as _, TsType as _, tsx as _};
}

#[cfg(feature = "widgets")]
#[test]
fn widget_macros_resolve_when_widgets_enabled() {
    use waterui::{widget as _, widget_intent as _};
}

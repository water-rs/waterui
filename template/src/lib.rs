use waterui::{AnyView, Environment, View};
use waterui_ffi::{IntoFFI, WuiAnyView, WuiEnv};

pub fn init() -> Environment {
    todo!()
}

pub fn main() -> impl View {}

#[unsafe(no_mangle)]
extern "C" fn waterui_init() -> *mut WuiEnv {
    init().into_ffi()
}

#[unsafe(no_mangle)]
extern "C" fn waterui_main() -> *mut WuiAnyView {
    AnyView::new(main()).into_ffi()
}

use waterui::{
    component::{layout::stack::vstack, progress::loading},
    text,
    prelude::layout::padding::EdgeInsets,
    Environment,
    View,
};

pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {
    vstack((
        text!("Hello, __DISPLAY_NAME__!"),
        "Edit src/lib.rs and the view will hot reload",
        loading(),
    ))
    .padding_with(EdgeInsets::all(32.0))
}

waterui_ffi::export!();

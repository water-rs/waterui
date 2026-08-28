//! Generated shim for the WaterUI Inspector.
//!
//! The Inspector itself lives in `waterui-inspector-app`, a real crate in the
//! WaterUI workspace that is compiled, linted, and tested with everything else.
//! This file only wires it to the platform entry point, so there is no
//! meaningful logic here to drift out of date.

use waterui::app::App;
use waterui::prelude::*;

/// The Inspector's root view.
pub fn main() -> impl View {
    waterui_inspector_app::main()
}

/// Application entry point.
pub fn app(env: Environment) -> App {
    waterui_inspector_app::app(env)
}

waterui_ffi::export!();

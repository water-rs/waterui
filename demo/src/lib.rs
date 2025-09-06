use waterui::{Environment, View};

pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {}

waterui_ffi::export!();

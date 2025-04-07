use waterui_core::Str;
use waterui_core::{configurable, AnyView};
use waterui_reactive::Computed;

#[derive(Debug)]
pub struct LinkConfig {
    pub label: AnyView,
    pub url: Computed<Str>,
}

configurable!(Link, LinkConfig);

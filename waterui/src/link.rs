use waterui_core::AnyView;
use waterui_reactive::Computed;
use waterui_str::Str;

#[derive(Debug)]
pub struct LinkConfig {
    pub label: AnyView,
    pub url: Computed<Str>,
}

configurable!(Link, LinkConfig);

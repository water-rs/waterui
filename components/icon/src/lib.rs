//! Icon components for WaterUI.

use std::{cell::RefCell, collections::BTreeMap};

use waterui_core::{View, plugin::Plugin, raw_view};
use waterui_str::Str;
use waterui_url::Url;

/// Icon component representing an icon by name.
#[derive(Debug, Clone)]
pub struct Icon {
    namespace: Str,
    name: Str,
}

/// Plugin for providing icons from a specific provider.
pub struct IconProviderPlugin<T> {
    provider: T,
}

#[derive(Default)]
struct IconProviders {
    map: RefCell<BTreeMap<Str, Box<dyn IconProvider>>>,
}

impl<T: IconProvider> IconProviderPlugin<T> {
    /// Create a new IconProviderPlugin with the given provider.
    pub const fn new(provider: T) -> Self {
        Self { provider }
    }
}

impl<T: IconProvider> Plugin for IconProviderPlugin<T> {
    fn install(self, env: &mut waterui_core::Environment) {
        //let mut providers = env.get_or_insert_with(|| IconProviders::default())
        todo!()
    }
}

/// Trait for providing icons from a specific namespace.
pub trait IconProvider: 'static {
    /// Get the namespace of this icon provider.
    fn namespace(&self) -> Str;
    /// Get the URL of the icon with the given name in this namespace.
    fn get_icon(&self, name: &Str) -> Option<Url>;
}

impl View for Icon {
    fn body(self, _env: &waterui_core::Environment) -> impl View {}
}

/// SystemIcon component representing a system icon by name.
#[derive(Debug, Clone)]
pub struct SystemIcon {
    /// The name of the system icon.
    pub name: Str,
}

raw_view!(SystemIcon);

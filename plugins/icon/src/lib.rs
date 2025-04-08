#![doc=include_str!("../README.md")]
#![no_std]
extern crate alloc;

use alloc::{boxed::Box, collections::btree_map::BTreeMap};
use waterui::component::{Dynamic, Native};
use waterui::env::use_env;
use waterui::view::ConfigurableView;
use waterui::{Computed, compute::IntoComputed};
use waterui::{Environment, View, component::media::Photo};
use waterui::{Str, ViewExt, core::plugin::Plugin};
#[cfg(feature = "std")]
mod std_on;

/// Configuration for an icon component.
///
/// This struct holds the properties that define how an icon appears and behaves.
#[derive(Debug, Clone)]
pub struct IconConfig {
    /// The name or identifier of the icon.
    pub name: Computed<Str>,
    /// The size of the icon in pixels.
    pub size: Computed<f64>,
    /// The animation style for the icon.
    pub animation: IconAnimation,
}

/// A UI component that displays an icon.
///
/// Icons are visual elements that can be used to represent actions, items, or status.
#[derive(Debug)]
#[must_use]
pub struct Icon(IconConfig);

impl ConfigurableView for Icon {
    type Config = IconConfig;
    fn config(self) -> Self::Config {
        self.0
    }
}

impl Icon {
    /// Creates a new icon with the specified name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name or identifier of the icon.
    ///
    /// # Returns
    ///
    /// A new `Icon` instance with default size and animation.
    pub fn new(name: impl IntoComputed<Str>) -> Self {
        Self(IconConfig {
            name: name.into_computed(),
            size: f64::NAN.into_computed(),
            animation: IconAnimation::default(),
        })
    }

    /// Sets the animation style for the icon.
    ///
    /// # Arguments
    ///
    /// * `value` - The animation style to use.
    ///
    /// # Returns
    ///
    /// The updated `Icon` instance.
    pub fn animation(mut self, value: IconAnimation) -> Self {
        self.0.animation = value;
        self
    }
}

/// Creates a new icon with the specified name.
///
/// This is a convenience function that wraps `Icon::new`.
///
/// # Arguments
///
/// * `id` - The name or identifier of the icon.
///
/// # Returns
///
/// A new `Icon` instance with default size and animation.
pub fn icon(id: impl IntoComputed<Str>) -> Icon {
    Icon::new(id)
}

/// Defines the animation style for an icon.
///
/// This enum specifies how an icon will animate when its state changes.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub enum IconAnimation {
    /// Use the default animation style.
    #[default]
    Default,
    /// Replace the icon immediately without animation.
    Replace,
    /// No animation when the icon changes.
    None,
}

type Url = Str;

/// Manages icon resources and their mappings.
///
/// `IconManager` provides a central registry for icons, allowing them to be
/// referenced by name and providing URL mappings and aliasing capabilities.
#[derive(Debug, Default)]
pub struct IconManager {
    icons: BTreeMap<Str, Url>,
    alias: BTreeMap<Str, Str>,
}

impl Plugin for IconManager {}

impl IconManager {
    /// Creates a new, empty icon manager.
    ///
    /// # Returns
    ///
    /// A new `IconManager` instance with no registered icons.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieves the URL for an icon by its identifier.
    ///
    /// This method will check for direct matches and then try to resolve
    /// the name through aliases.
    ///
    /// # Arguments
    ///
    /// * `id` - The icon identifier to look up.
    ///
    /// # Returns
    ///
    /// The URL of the icon if found, or `None` if no matching icon exists.
    pub fn get(&self, id: &str) -> Option<&Url> {
        self.icons
            .get(id)
            .or_else(|| self.alias.get(id).and_then(|full| self.icons.get(full)))
    }

    /// Registers an icon with the manager.
    ///
    /// # Arguments
    ///
    /// * `id` - The primary identifier for the icon.
    /// * `alia` - An alias or secondary name for the icon.
    /// * `url` - The URL or path to the icon resource.
    pub fn insert(&mut self, id: Str, alia: Str, url: Url) {
        self.icons.insert(id.clone(), url);
        self.alias.insert(alia, id);
    }
}

/// Binary data type for icon content.
type Data = Box<[u8]>;

impl View for Icon {
    fn body(self, _env: &Environment) -> impl View {
        let config = self.config();
        Dynamic::watch(config.name.clone(), move |id| {
            let config = config.clone();
            use_env(move |env: Environment| {
                if let Some(manager) = env.get::<IconManager>() {
                    let icon = manager
                        .icons
                        .get(&id)
                        .or_else(|| manager.alias.get(&id).and_then(|v| manager.icons.get(v)))
                        .cloned();
                    if let Some(icon) = icon {
                        return Photo::new(icon).anyview();
                    }
                }

                Native(config).anyview()
            });
        })
    }
}

#![no_std]
extern crate alloc;

use alloc::{boxed::Box, collections::btree_map::BTreeMap};
use waterui::component::{Dynamic, Native};
use waterui::env::use_env;
use waterui::view::ConfigurableView;
use waterui::{component::media::Photo, Environment, View};
use waterui::{compute::IntoComputed, Computed};
use waterui::{core::plugin::Plugin, Str, ViewExt};
#[cfg(feature = "std")]
mod std_on;
#[derive(Debug, Clone)]
pub struct IconConfig {
    pub name: Computed<Str>,
    pub size: Computed<f64>,
    pub animation: IconAnimation,
}

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
    pub fn new(name: impl IntoComputed<Str>) -> Self {
        Self(IconConfig {
            name: name.into_computed(),
            size: f64::NAN.into_computed(),
            animation: IconAnimation::default(),
        })
    }

    pub fn animation(mut self, value: IconAnimation) -> Self {
        self.0.animation = value;
        self
    }
}

pub fn icon(id: impl IntoComputed<Str>) -> Icon {
    Icon::new(id)
}

#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub enum IconAnimation {
    #[default]
    Default,
    Replace,
    None,
}

type Url = Str;

#[derive(Debug, Default)]
pub struct IconManager {
    icons: BTreeMap<Str, Url>,
    alias: BTreeMap<Str, Str>,
}

impl Plugin for IconManager {}

impl IconManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<&Url> {
        self.icons
            .get(id)
            .or_else(|| self.alias.get(id).and_then(|full| self.icons.get(full)))
    }

    pub fn insert(&mut self, id: Str, alia: Str, url: Url) {
        self.icons.insert(id.clone(), url);
        self.alias.insert(alia, id);
    }
}

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

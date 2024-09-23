#![no_std]
extern crate alloc;
use core::ops::Deref;

use alloc::{boxed::Box, collections::btree_map::BTreeMap};
use waterui::component::{Dynamic, Image, Native};
use waterui::env::Plugin;
use waterui::view::ConfigurableView;
use waterui::{compute::ToComputed, Computed};
use waterui::{Environment, View};
use waterui::{Str, ViewExt};
#[cfg(feature = "std")]
mod std_on;
#[derive(Debug, Clone)]
pub struct IconConfig {
    pub id: Computed<Str>,
    pub animation: IconAnimation,
}

#[derive(Debug)]
pub struct Icon(IconConfig);

impl ConfigurableView for Icon {
    type Config = IconConfig;
    fn config(self) -> Self::Config {
        self.0
    }
}

impl Icon {
    pub fn new(id: impl ToComputed<Str>) -> Self {
        Self(IconConfig {
            id: id.to_computed(),
            animation: IconAnimation::default(),
        })
    }

    pub fn animation(mut self, value: IconAnimation) -> Self {
        self.0.animation = value;
        self
    }
}

#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub enum IconAnimation {
    #[default]
    Default,
    Replace,
    None,
}

#[derive(Debug, Default)]
pub struct IconManager {
    icons: BTreeMap<Str, Data>,
    alias: BTreeMap<Str, Str>,
}

impl Plugin for IconManager {}

impl IconManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<&[u8]> {
        self.icons
            .get(id)
            .or_else(|| self.alias.get(id).and_then(|full| self.icons.get(full)))
            .map(Deref::deref)
    }

    pub fn insert(&mut self, id: Str, alia: Str, data: Data) {
        self.icons.insert(id.clone(), data);
        self.alias.insert(alia, id);
    }
}

type Data = Box<[u8]>;

impl View for Icon {
    fn body(self, env: Environment) -> impl View {
        let config = self.config();
        Dynamic::watch(config.id.clone(), move |id| {
            let manager = env.get::<IconManager>();
            let icon = manager
                .icons
                .get(&id)
                .or_else(|| manager.alias.get(&id).and_then(|v| manager.icons.get(v)))
                .cloned();

            if let Some(icon) = icon {
                Image::new(icon).anyview()
            } else {
                Native(config.clone()).anyview()
            }
        })
    }
}

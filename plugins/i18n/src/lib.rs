#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "std")]
mod std_on;
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
use alloc::collections::BTreeMap;

use waterui_core::{
    ComputeExt, Environment, Str, extract::Extractor, plugin::Plugin, view::Modifier,
};
use waterui_text::{Text, locale::Locale};
#[derive(Debug, Default)]
pub struct I18n {
    map: BTreeMap<Str, BTreeMap<Str, Str>>,
}

impl I18n {
    pub const fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, locale: impl Into<Str>, key: impl Into<Str>, value: impl Into<Str>) {
        self.map
            .entry(locale.into())
            .or_default()
            .insert(key.into(), value.into());
    }

    pub fn get(&self, locale: &str, key: impl Into<Str>) -> Str {
        let key = key.into();
        self.try_get(locale, &key).cloned().unwrap_or(key)
    }

    pub fn try_get(&self, locale: &str, key: &str) -> Option<&Str> {
        self.map.get(locale).and_then(|map| map.get(key))
    }
}

impl Plugin for I18n {
    fn install(self, env: &mut Environment) {
        env.insert(self);

        env.insert(Modifier::<Text>::new(|env, mut config| {
            let Locale(locale) = Locale::extract(&env).unwrap();
            config.content = config
                .content
                .map(move |content| {
                    if let Some(i18n) = env.get::<I18n>() {
                        i18n.get(&locale, content)
                    } else {
                        content
                    }
                })
                .computed();
            Text::from(config)
        }))
    }
}

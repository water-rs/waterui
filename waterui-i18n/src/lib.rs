#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
use alloc::collections::BTreeMap;
use waterui::component::{locale::Locale, Text};
use waterui_core::{env::Plugin, extract::Extractor, view::Modifier, Environment};
use waterui_reactive::compute::ComputeExt;
use waterui_str::Str;
#[derive(Debug, Default)]
pub struct I18n {
    map: BTreeMap<Str, BTreeMap<Str, Str>>,
}

#[cfg(feature = "std")]
mod std_on {
    use alloc::{collections::btree_map::BTreeMap, string::ToString};
    use async_fs::{read_dir, read_to_string, write};
    use futures_lite::stream::StreamExt;
    use std::io;
    use toml::{from_str, to_string_pretty};
    use waterui_str::Str;

    use super::I18n;

    extern crate std;
    use core::ops::Deref;
    use std::path::Path;
    use thiserror::Error;
    #[derive(Debug, Error)]
    pub enum Error {
        #[error("Io Error {0}")]
        Io(#[from] io::Error),
        #[error("Deserialize error {0}")]
        Deserialize(#[from] toml::de::Error),
        #[error("Serialize error {0}")]
        Serialize(#[from] toml::ser::Error),
    }

    impl I18n {
        pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
            let mut dir = read_dir(path).await?;
            let mut i18n: BTreeMap<Str, BTreeMap<Str, Str>> = BTreeMap::new();
            while let Some(file) = dir.next().await {
                let file = file?;
                let path = file.path();
                if let Some(extension) = path.extension() {
                    if extension == "toml" {
                        let buf = read_to_string(&path).await?;
                        let map: BTreeMap<Str, Str> = from_str(&buf)?;
                        if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
                            i18n.insert(Str::from(name.to_string()), map);
                        }
                    }
                }
            }
            Ok(I18n { map: i18n })
        }

        pub async fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), Error> {
            let path = path.as_ref();
            for (locale, map) in self.map.iter() {
                let path = path.join(locale.deref()).with_extension("toml");
                write(path, to_string_pretty(&map)?).await?;
            }
            Ok(())
        }
    }
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

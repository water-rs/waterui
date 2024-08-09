#![no_std]
extern crate alloc;

use alloc::collections::btree_map::BTreeMap;
use waterui_core::{components::Text, env::Plugin, view::Modifier, Environment};
use waterui_reactive::compute::ComputeExt;
use waterui_str::Str;
#[derive(Debug)]
pub struct I18n {
    map: BTreeMap<Str, BTreeMap<Str, Str>>,
    locale: Str,
}

mod std_on {
    use alloc::{
        collections::btree_map::BTreeMap,
        string::{String, ToString},
    };
    use smol::{
        fs::{read_dir, File},
        io::{self, AsyncReadExt, AsyncWriteExt, BufReader},
        stream::StreamExt,
    };
    use sys_locale::get_locale;
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
                        let mut file = BufReader::new(File::open(&path).await?);
                        let mut buf = String::new();
                        file.read_to_string(&mut buf).await?;
                        let map: BTreeMap<Str, Str> = from_str(&buf)?;
                        if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
                            i18n.insert(Str::from(name.to_string()), map);
                        }
                    }
                }
            }
            Ok(I18n {
                map: i18n,
                locale: get_locale().unwrap_or("en-US".into()).into(),
            })
        }

        pub async fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), Error> {
            let path = path.as_ref();
            for (locale, map) in self.map.iter() {
                let path = path.join(locale.deref()).with_extension("toml");
                let mut file = File::create(path).await?;
                let buf = to_string_pretty(&map)?;
                file.write_all(buf.as_bytes()).await?;
            }
            Ok(())
        }
    }
}

impl I18n {
    pub fn new(locale: impl Into<Str>) -> Self {
        Self {
            map: BTreeMap::new(),
            locale: locale.into(),
        }
    }

    pub fn set_locale(&mut self, locale: impl Into<Str>) {
        self.locale = locale.into();
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub fn insert(&mut self, locale: impl Into<Str>, key: impl Into<Str>, value: impl Into<Str>) {
        self.map
            .entry(locale.into())
            .or_default()
            .insert(key.into(), value.into());
    }

    pub fn get(&self, key: impl Into<Str>) -> Str {
        let key = key.into();
        self.try_get(&key).cloned().unwrap_or(key)
    }

    pub fn try_get(&self, key: &str) -> Option<&Str> {
        self.map.get(&self.locale).and_then(|map| map.get(key))
    }
}

impl Plugin for I18n {
    fn install(self, env: &mut Environment) {
        env.insert(self);

        env.insert(Modifier::<Text>::new(|env, mut config| {
            config.content = config
                .content
                .map(move |content| {
                    if let Some(i18n) = env.try_get::<I18n>() {
                        i18n.get(content)
                    } else {
                        content
                    }
                })
                .computed();
            Text::from(config)
        }))
    }
}

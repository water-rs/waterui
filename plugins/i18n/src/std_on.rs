use alloc::{collections::btree_map::BTreeMap, string::ToString};
use async_fs::{read_dir, read_to_string, write};
use futures_lite::stream::StreamExt;
use std::io;
use toml::{from_str, to_string_pretty};
use waterui_core::Str;

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

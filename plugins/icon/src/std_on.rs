use crate::IconManager;

extern crate std;
use alloc::format;
use async_fs::{read_dir, read_to_string};
use futures_lite::StreamExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{io, path::Path};
use thiserror::Error;
use toml::from_str;
use waterui::Str;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IconToml {
    name: Str,
    alia: Str,
    author: Str,
}

impl IconManager {
    pub async fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref();
        let toml: IconToml = deserialize(dir.join("Icon.toml")).await?;

        let mut dir = read_dir(dir).await?;
        let mut manager = IconManager::new();
        while let Some(item) = dir.next().await {
            let item = item?;
            let id = Str::from_utf8(item.file_name().into_encoded_bytes())?;
            let ty = item.file_type().await?;
            if ty.is_file() {
                manager.insert(
                    id,
                    toml.alia.clone(),
                    format!("file://{}", item.path().display()).into(),
                )
            }
        }
        todo!()
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Io error: {0}")]
    Io(#[from] io::Error),
    #[error("Fail to parse toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Utf8 error {0}")]
    Utf8(#[from] alloc::string::FromUtf8Error),
}

async fn deserialize<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T, Error> {
    let string = read_to_string(path).await?;
    Ok(from_str(&string)?)
}

//! This module provides `std` feature-dependent functionality for the `I18n` struct,
//! primarily related to file system interactions for loading and saving translations.
//!
//! The methods here are only available when the `std` feature is enabled for the crate.
//!
//! # Features
//! - `std`: Enables `I18n::open` and `I18n::save` for TOML-based file I/O.

#![allow(clippy::future_not_send)]
use alloc::{collections::btree_map::BTreeMap, string::ToString};
use async_fs::{read_dir, read_to_string, write};
use futures_lite::stream::StreamExt;
use std::io;
use toml::{from_str, to_string_pretty};
use waterui_core::Str;

use super::I18n;

extern crate std;
use std::path::Path;
use thiserror::Error;

/// Represents errors that can occur during file-based I18n operations.
#[derive(Debug, Error)]
pub enum Error {
    /// An error occurred during file I/O.
    #[error("Io Error {0}")]
    Io(#[from] io::Error),
    /// An error occurred while deserializing TOML data.
    #[error("Deserialize error {0}")]
    Deserialize(#[from] toml::de::Error),
    /// An error occurred while serializing data to TOML format.
    #[error("Serialize error {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl I18n {
    /// Asynchronously loads translations from a directory of TOML files.
    ///
    /// This function scans the specified directory for files ending with the `.toml`
    /// extension. Each file is expected to correspond to a locale, with its name
    /// (sans extension) serving as the locale identifier (e.g., `en.toml` for "en").
    ///
    /// # Parameters
    ///
    /// - `path`: The path to the directory containing the translation files.
    ///
    /// # Returns
    ///
    /// A `Result` containing a new `I18n` instance populated with the loaded
    /// translations, or an `Error` if reading the directory or parsing the files fails.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The specified path does not exist or is not a directory.
    /// - There are I/O errors when reading the directory or the files within it.
    /// - A file's content is not valid TOML.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use waterui_i18n::I18n;
    /// # use std::error::Error;
    /// #
    /// # #[async_std::main]
    /// # async fn main() -> Result<(), Box<dyn Error>> {
    /// // Assuming a directory `locales/` with `en.toml` and `fr.toml`.
    /// let i18n = I18n::open("locales").await?;
    ///
    /// assert_eq!(i18n.get("en", "greeting"), "Hello!");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn open(path: impl AsRef<Path> + Send) -> Result<Self, Error> {
        let mut dir = read_dir(path).await?;
        let mut i18n: BTreeMap<Str, BTreeMap<Str, Str>> = BTreeMap::new();
        while let Some(file) = dir.next().await {
            let file = file?;
            let path = file.path();
            if let Some(extension) = path.extension()
                && extension == "toml" {
                    let buf = read_to_string(&path).await?;
                    let map: BTreeMap<Str, Str> = from_str(&buf)?;
                    if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
                        i18n.insert(Str::from(name.to_string()), map);
                    }
                }
        }
        Ok(Self { map: i18n })
    }

    /// Asynchronously saves all translations to a directory of TOML files.
    ///
    /// For each locale in the `I18n` instance, this function creates a corresponding
    /// `.toml` file (e.g., "en" -> `en.toml`) in the specified directory. The file
    /// will contain the key-value pairs for that locale, formatted as TOML.
    ///
    /// # Parameters
    ///
    /// - `path`: The path to the directory where the translation files will be saved.
    ///
    /// # Returns
    ///
    /// An empty `Result` indicating success, or an `Error` if writing the files fails.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - There are I/O errors when writing the files.
    /// - The translation data for a locale cannot be serialized to TOML format.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use waterui_i18n::I18n;
    /// # use std::error::Error;
    /// #
    /// # #[async_std::main]
    /// # async fn main() -> Result<(), Box<dyn Error>> {
    /// let mut i18n = I18n::new();
    /// i18n.insert("en", "greeting", "Hello!");
    ///
    /// // This will create a file `locales_backup/en.toml`.
    /// i18n.save("locales_backup").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn save(&self, path: impl AsRef<std::path::Path> + Send) -> Result<(), Error> {
        let path = path.as_ref();
        for (locale, map) in &self.map {
            let path = path.join(&**locale).with_extension("toml");
            write(path, to_string_pretty(&map)?).await?;
        }
        Ok(())
    }
}

# waterui-i18n - Internationalization Plugin for WaterUI

[![Crates.io](https://img.shields.io/crates/v/waterui-i18n)](https://crates.io/crates/waterui-i18n)
[![Documentation](https://docs.rs/waterui-i18n/badge.svg)](https://docs.rs/waterui-i18n)

A lightweight internationalization plugin for the WaterUI framework, providing text translation capabilities.

## Features

- Simple key-value based translation system
- Locale-specific text substitution
- `no_std` compatible (with optional `std` features)
- Async file I/O support (when `std` feature is enabled)
- TOML format for translation files
- Automatic text modification in WaterUI views

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-i18n = { version = "0.1", features = ["std"] }  # with file I/O support
# or for no_std environments:
waterui-i18n = "0.1"
```

## Basic Usage

```rust
use waterui_i18n::I18n;
use waterui_core::Environment;

let mut i18n = I18n::new();
i18n.insert("en", "greeting", "Hello, World!");
i18n.insert("fr", "greeting", "Bonjour le monde!");

let mut env = Environment::new();
i18n.install(&mut env);

// In your WaterUI views, text will be automatically translated
// based on the current locale
```

## File-based Usage (with `std` feature)

```rust
use waterui_i18n::I18n;

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load from directory of TOML files (en.toml, fr.toml, etc.)
    let i18n = I18n::open("locales").await?;

    // Save translations back to files
    i18n.save("locales_backup").await?;

    Ok(())
}
```

## File Format

Translation files should be named as `[locale].toml` (e.g., `en.toml`, `fr.toml`) and contain key-value pairs:

```toml
# en.toml
greeting = "Hello, World!"
welcome = "Welcome to our application!"
```

## Features

- `std` (default): Enables file I/O operations and error handling

For `no_std` environments, simply disable default features.

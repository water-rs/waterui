# waterui-locale

Localization support for WaterUI applications.

`waterui-locale` provides locale-aware text, plural selection, translation
catalogs, and regional formatting helpers used by WaterUI text and form
components. It integrates with WaterUI environments so applications can resolve
localized content from an explicit locale context or from runtime regional
settings.

## Features

- Locale identifiers and canonical locale tags.
- CLDR plural category selection.
- Translation catalogs for localized string lookup.
- Locale-aware date, list, length, mass, and temperature formatting.
- Runtime regional context extraction for WaterUI environments.

## Usage

```rust
use waterui_locale::{Locale, locales};

let locale: Locale = "en-US".parse().unwrap_or_else(|_| locales::EN_US);
assert_eq!(locale.canonical_tag(), "en-US");
```

Most applications use this crate through `waterui-text` or the main `waterui`
facade crate.

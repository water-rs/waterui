//! Type-safe Apple SF Symbols provided directly by `waterui-apple`.
//!
//! [`SfSymbol`] is realized natively through the Apple backend's
//! backend-provided component path (`AppleBackend::provided` /
//! `provided_dyn`), never through `AppleView` / `AppleContent` or a
//! cross-platform `SystemIcon` fallback. Every symbol in Apple's catalog is
//! exposed as an associated constant gated on the build's Apple deployment
//! target so nonexistent or too-new symbols fail to compile.

use alloc::string::String;
use core::fmt;

use waterui_core::{impl_constant, raw_view};

#[path = "sf_symbol_catalog.rs"]
mod catalog;
#[cfg(test)]
#[path = "sf_symbol_releases.rs"]
mod releases;

/// A type-safe SF Symbol from Apple's system symbol catalog.
///
/// Constants on `SfSymbol` (such as [`SfSymbol::HOUSE`],
/// [`SfSymbol::HOUSE_FILL`], or [`SfSymbol::SQUARE_AND_ARROW_UP`]) are generated
/// from Apple's `name_availability.plist` and gated against the active Apple
/// deployment target. There is no string constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SfSymbol {
    name: &'static str,
}

impl SfSymbol {
    const fn from_static(name: &'static str) -> Self {
        Self { name }
    }

    /// Returns the underlying SF Symbol catalog name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the underlying SF Symbol catalog name as a string slice.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.name
    }
}

impl AsRef<str> for SfSymbol {
    fn as_ref(&self) -> &str {
        self.name
    }
}

impl fmt::Display for SfSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

impl From<SfSymbol> for &'static str {
    fn from(symbol: SfSymbol) -> Self {
        symbol.name
    }
}

impl From<SfSymbol> for String {
    fn from(symbol: SfSymbol) -> Self {
        Self::from(symbol.name)
    }
}

raw_view!(SfSymbol);
impl_constant!(SfSymbol);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_constants_carry_catalog_names() {
        assert_eq!(SfSymbol::HOUSE.name(), "house");
        assert_eq!(SfSymbol::HOUSE_FILL.as_str(), "house.fill");
        assert_eq!(SfSymbol::_0_CIRCLE.to_string(), "0.circle");
        assert_eq!(String::from(SfSymbol::STAR_FILL), "star.fill");
    }

    #[test]
    fn release_table_starts_with_2019_and_reaches_2026() {
        assert_eq!(
            releases::SF_SYMBOL_RELEASES.first().map(|r| r.0),
            Some("2019")
        );
        assert_eq!(
            releases::SF_SYMBOL_RELEASES.last().map(|r| r.0),
            Some("2026")
        );
    }

    #[test]
    fn deployment_target_gates_releases() {
        const {
            assert!(cfg!(sf_symbol_release = "2019"));
            assert!(cfg!(sf_symbol_release = "2025"));
        }
    }
}

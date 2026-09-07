//! Text components and utilities for the `WaterUI` framework.
//!
//! This crate provides comprehensive text rendering and formatting capabilities,
//! including fonts, attributed text, and internationalization support.
//!
//! Note: The `Link` component has been moved to the main `waterui` crate
//! where it can use `robius-open` for URL handling.

#![no_std]

extern crate alloc;

#[cfg(feature = "highlight")]
pub mod code;
/// The one font collection an application shapes and typesets with.
#[cfg(feature = "font-collection")]
pub mod collection;
/// Font utilities and definitions.
pub mod font;
/// Syntax highlighting support.
pub mod highlight;
/// Styled text support for rich text formatting.
pub mod styled;

/// Core text component.
pub mod text;
#[cfg(feature = "highlight")]
pub use code::{Code, CodeConfig, OnCopied, code};
#[cfg(feature = "font-collection")]
pub use collection::FontCollection;
pub use styled::StyledStr;
pub use text::{Formatter, IntoText, Text, TextConfig, text};

/// Installs the system's fonts as the application's font collection, for a host
/// that owns no font stack of its own.
///
/// The native backends — Apple, Android, GTK — draw text with the platform's
/// own text engine and never build a `parley` collection, so a component that
/// typesets text itself (a formula, a canvas, a vector map) has no host
/// collection to share. This gives them one, discovered once for the
/// application rather than once per view.
///
/// It installs nothing at all when this build has no collection to install,
/// which is exactly when nothing in the dependency graph could read one: the
/// `system-fonts` feature is turned on by the components that shape text
/// themselves, so a build without them carries neither the collection type nor
/// the font stack behind it.
///
/// A collection already installed is left alone, so a host that owns its own
/// font stack may install that one and call this too, in either order.
#[allow(
    clippy::missing_const_for_fn,
    reason = "the body is empty only in the feature configuration being linted; a build that can consume a font collection installs one here"
)]
pub fn install_system_font_collection(env: &mut waterui_core::Environment) {
    #[cfg(feature = "system-fonts")]
    if env.get::<collection::FontCollection>().is_none() {
        collection::FontCollection::system().install(env);
    }
    #[cfg(not(feature = "system-fonts"))]
    let _ = env;
}

//! Web font registration: the page's manifest fonts registered,
//! classified and installed as the script fallbacks.
//!
//! Fetching stays with the web runner — it is the only code that can await
//! the browser's fetch — and this module does the registration half: the
//! `FontInfoOverride` naming, the resource-font classification, the
//! default-family assertion and the fallback install, ending in the
//! `FontCollection` the page's engine is built from.

use std::sync::Arc;

use parley::fontique::{Blob, FontInfoOverride};
use waterui_text::FontCollection;

use super::fonts::ResourceFontFamilies;

/// The collection a web session shapes with: every manifest font registered
/// under its declared family name, the recognized script fallbacks
/// installed, and the manifest's `default_family` asserted present.
pub fn web_collection<'a>(
    default_family: &str,
    fonts: impl IntoIterator<Item = (&'a str, Vec<u8>)>,
) -> FontCollection {
    let mut default_family_ids = Vec::new();
    let mut resource_fonts = ResourceFontFamilies::default();
    let mut font_cx = parley::FontContext::new();
    for (name, font_data) in fonts {
        let families = font_cx.collection.register_fonts(
            Blob::new(Arc::new(font_data)),
            Some(FontInfoOverride {
                family_name: Some(name),
                ..Default::default()
            }),
        );
        if name == default_family {
            default_family_ids.extend(families.iter().map(|(family_id, _)| *family_id));
        }
        resource_fonts.classify(name, &families);
    }

    assert!(
        !default_family_ids.is_empty(),
        "hydrolysis web font manifest default family `{default_family}` did not register any fonts",
    );
    resource_fonts.install(&mut font_cx.collection);
    FontCollection::new(font_cx)
}

//! Web font registration: the page's manifest fonts registered,
//! classified and installed as the script fallbacks.
//!
//! Fetching stays with the web runner — it is the only code that can await
//! the browser's fetch — and this module does the registration half: the
//! `FontInfoOverride` naming, the resource-font classification, the
//! default-family assertion and the fallback install. A page registers its
//! fonts in two steps: the faces its first frame needs, which build the
//! `FontCollection` the page's engine is built from, then each remaining
//! face as it arrives.

use std::sync::Arc;

use parley::fontique::{Blob, FontInfoOverride};
use waterui_text::FontCollection;

use super::fonts::ResourceFontFamilies;

/// A page's font collection and the fallback classification of every face
/// registered into it so far.
pub struct WebFonts {
    collection: FontCollection,
    families: ResourceFontFamilies,
}

impl WebFonts {
    /// The collection a web session's first frame shapes with: every face in
    /// `fonts` registered under its declared family name, the recognized
    /// script fallbacks installed, and `default_family` asserted present.
    ///
    /// The collection is shared (`fontique`'s shared store), so a face
    /// [`register`](Self::register)ed later reaches every copy of it — the
    /// text engine's shaping contexts included.
    ///
    /// # Panics
    ///
    /// Panics if no face registers under `default_family`.
    #[must_use]
    pub fn new<'a>(
        default_family: &str,
        fonts: impl IntoIterator<Item = (&'a str, Vec<u8>)>,
    ) -> Self {
        let mut font_cx = parley::FontContext::new();
        font_cx.collection.make_shared();
        let mut families = ResourceFontFamilies::default();
        let mut default_registered = false;
        for (name, font_data) in fonts {
            let registered = register_face(&mut font_cx.collection, &mut families, name, font_data);
            default_registered |= name == default_family && registered;
        }
        assert!(
            default_registered,
            "hydrolysis web font manifest default family `{default_family}` did not register any fonts",
        );
        families.install(&mut font_cx.collection);
        Self {
            collection: FontCollection::new(font_cx),
            families,
        }
    }

    /// The page's collection.
    #[must_use]
    pub const fn collection(&self) -> &FontCollection {
        &self.collection
    }

    /// Registers a face that arrived after the collection was built and
    /// reinstalls the fallbacks with it. The text engine still holds every
    /// shaping result made without it: the caller tells the renderer the
    /// fonts changed.
    ///
    /// # Panics
    ///
    /// Panics if `font_data` holds no face.
    pub fn register(&mut self, name: &str, font_data: Vec<u8>) {
        self.collection.use_fonts(|font_cx| {
            assert!(
                register_face(&mut font_cx.collection, &mut self.families, name, font_data),
                "hydrolysis web font `{name}` holds no face"
            );
            self.families.install(&mut font_cx.collection);
        });
    }
}

/// Registers `font_data` under the family `name` and classifies it; whether
/// any face registered.
fn register_face(
    collection: &mut parley::fontique::Collection,
    families: &mut ResourceFontFamilies,
    name: &str,
    font_data: Vec<u8>,
) -> bool {
    let registered = collection.register_fonts(
        Blob::new(Arc::new(font_data)),
        Some(FontInfoOverride {
            family_name: Some(name),
            ..Default::default()
        }),
    );
    families.classify(name, &registered);
    !registered.is_empty()
}

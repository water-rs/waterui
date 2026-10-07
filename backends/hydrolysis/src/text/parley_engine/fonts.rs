//! Resource font registration and locale-aware fallback installation.
//!
//! `WaterUI` ships a known set of resource fonts (Roboto plus script-specific
//! Noto Sans families). Classification and fallback installation are shared by the
//! native loader (which scans `resources/fonts` directories) and the web
//! loader in [`super::web_fonts`] (which registers fonts fetched from a
//! manifest).
//!
//! The result is built **once per application**, installed into the root
//! environment as the shared `FontCollection`, and every window's text engine
//! is constructed from it. Building it per window meant scanning
//! the resource directories and enumerating the system's fonts again for each
//! one, and a self-drawn component reading the environment would have had no
//! single collection to read.

#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
use parley::fontique::{Collection, FallbackKey, FamilyId, FontInfo, GenericFamily, Script};
use waterui_text::FontCollection;

#[cfg(not(target_arch = "wasm32"))]
use super::super::DeclaredFonts;

#[cfg(target_os = "android")]
pub use super::android_fonts::android_collection;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use super::web_fonts::web_collection;

/// Font-family buckets recognized from `WaterUI`'s bundled resource fonts.
#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
#[derive(Default)]
pub(super) struct ResourceFontFamilies {
    generic: Vec<FamilyId>,
    emoji: Vec<FamilyId>,
    hani_simplified: Vec<FamilyId>,
    hani_traditional: Vec<FamilyId>,
    hani_japanese: Vec<FamilyId>,
    hani_korean: Vec<FamilyId>,
    arabic: Vec<FamilyId>,
    hebrew: Vec<FamilyId>,
    thai: Vec<FamilyId>,
    devanagari: Vec<FamilyId>,
}

#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
fn extend_family_ids(target: &mut Vec<FamilyId>, families: &[(FamilyId, Vec<FontInfo>)]) {
    target.extend(families.iter().map(|(family_id, _)| *family_id));
}

#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
fn set_fallbacks(collection: &mut Collection, key: impl Into<FallbackKey>, families: &[FamilyId]) {
    if families.is_empty() {
        return;
    }
    assert!(
        collection.set_fallbacks(key, families.iter().copied()),
        "hydrolysis font loader attempted to install an untracked script fallback"
    );
}

#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
impl ResourceFontFamilies {
    /// Classify registered font families into fallback buckets by font name.
    ///
    /// The name is normalized (lowercased, spaces stripped) so the same rules
    /// cover native file names (`NotoSansCJKsc-Regular.otf`) and web manifest
    /// family names (`Noto Sans CJK SC`).
    pub(super) fn classify(&mut self, name: &str, families: &[(FamilyId, Vec<FontInfo>)]) {
        let key = name.to_ascii_lowercase().replace(' ', "");
        if key.contains("emoji") {
            extend_family_ids(&mut self.emoji, families);
        } else if key.contains("roboto") {
            extend_family_ids(&mut self.generic, families);
        } else if key.contains("notosanscjksc") {
            extend_family_ids(&mut self.hani_simplified, families);
        } else if key.contains("notosanscjktc") {
            extend_family_ids(&mut self.hani_traditional, families);
        } else if key.contains("notosanscjkjp") {
            extend_family_ids(&mut self.hani_japanese, families);
        } else if key.contains("notosanscjkkr") {
            extend_family_ids(&mut self.hani_korean, families);
        } else if key.contains("notosansarabic") {
            extend_family_ids(&mut self.arabic, families);
        } else if key.contains("notosanshebrew") {
            extend_family_ids(&mut self.hebrew, families);
        } else if key.contains("notosansthai") {
            extend_family_ids(&mut self.thai, families);
        } else if key.contains("notosansdevanagari") {
            extend_family_ids(&mut self.devanagari, families);
        }
    }

    /// Install the classified families as generic-family defaults and Han
    /// script fallbacks keyed by locale.
    pub(super) fn install(&self, collection: &mut Collection) {
        if !self.generic.is_empty() {
            collection.set_generic_families(GenericFamily::SansSerif, self.generic.iter().copied());
            collection
                .set_generic_families(GenericFamily::UiSansSerif, self.generic.iter().copied());
            collection.set_generic_families(GenericFamily::SystemUi, self.generic.iter().copied());
        }
        if !self.emoji.is_empty() {
            collection.set_generic_families(GenericFamily::Emoji, self.emoji.iter().copied());
        }

        let hani = Script::from_str_unchecked("Hani");
        set_fallbacks(collection, hani, &self.hani_simplified);
        for locale in ["zh", "zh-CN", "zh-SG"] {
            set_fallbacks(collection, (hani, locale), &self.hani_simplified);
        }
        for locale in ["zh-Hant", "zh-TW", "zh-HK", "zh-MO"] {
            set_fallbacks(collection, (hani, locale), &self.hani_traditional);
        }
        set_fallbacks(collection, (hani, "ja"), &self.hani_japanese);
        set_fallbacks(collection, (hani, "ko"), &self.hani_korean);
        // Hangul text selects the Hang script, not Hani: the KR face a host
        // bundles for Korean must answer that key too or it sits unreachable.
        set_fallbacks(
            collection,
            Script::from_str_unchecked("Hang"),
            &self.hani_korean,
        );
        set_fallbacks(collection, Script::from_str_unchecked("Arab"), &self.arabic);
        set_fallbacks(collection, Script::from_str_unchecked("Hebr"), &self.hebrew);
        set_fallbacks(collection, Script::from_str_unchecked("Thai"), &self.thai);
        set_fallbacks(
            collection,
            Script::from_str_unchecked("Deva"),
            &self.devanagari,
        );
    }
}

/// The system's fonts plus every `.ttf`/`.otf` under the application's staged
/// fonts directory and every file of the environment's [`DeclaredFonts`],
/// with the recognized script fallbacks installed.
///
/// The fonts directory is the one owned by the environment's required
/// [`waterui_core::ResourceContext`] and is never probed from the process's
/// working directory or executable location, so an embedded host's fonts come
/// from the roots it installed.
///
/// # Panics
///
/// Panics if the environment carries no `ResourceContext`, or if the fonts
/// directory or a declared font file cannot be read, naming the path.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn native_collection(env: &waterui_core::Environment) -> FontCollection {
    let root = waterui_core::ResourceContext::from_environment(env).fonts();
    let mut font_cx = parley::FontContext::new();
    let mut resource_fonts = ResourceFontFamilies::default();
    if root.is_dir() {
        let entries = std::fs::read_dir(root).unwrap_or_else(|error| {
            panic!(
                "hydrolysis native font loader failed to read `{}`: {error}",
                root.display()
            )
        });
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!(
                    "hydrolysis native font loader failed to read an entry in `{}`: {error}",
                    root.display()
                )
            });
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            if !extension.eq_ignore_ascii_case("ttf") && !extension.eq_ignore_ascii_case("otf") {
                continue;
            }
            register_font_file(&mut font_cx, &mut resource_fonts, &path);
        }
    }
    if let Some(declared) = env.get::<DeclaredFonts>() {
        for path in declared.paths() {
            register_font_file(&mut font_cx, &mut resource_fonts, path);
        }
    }
    resource_fonts.install(&mut font_cx.collection);
    FontCollection::new(font_cx)
}

/// Registers the font file at `path` into `font_cx` and classifies its
/// families into `resource_fonts` by file name.
#[cfg(not(target_arch = "wasm32"))]
fn register_font_file(
    font_cx: &mut parley::FontContext,
    resource_fonts: &mut ResourceFontFamilies,
    path: &std::path::Path,
) {
    use parley::fontique::Blob;
    use std::sync::Arc;

    let font_data = std::fs::read(path).unwrap_or_else(|error| {
        panic!(
            "hydrolysis native font loader failed to read `{}`: {error}",
            path.display()
        )
    });
    let families = font_cx
        .collection
        .register_fonts(Blob::new(Arc::new(font_data)), None);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| {
            panic!(
                "hydrolysis native font loader found a font path without UTF-8 file name: `{}`",
                path.display()
            )
        });
    resource_fonts.classify(file_name, &families);
    tracing::debug!(
        target: "waterui::hydrolysis::fonts",
        path = %path.display(),
        families = families.len(),
        "registered native Hydrolysis font"
    );
}

/// A collection carrying only the faces fontique discovers on the system —
/// for a wasm32 window without a font manifest, where there is no resource
/// directory to scan and no `web_collection` result to build on.
#[cfg(target_arch = "wasm32")]
pub fn system_collection() -> FontCollection {
    FontCollection::new(parley::FontContext::new())
}

/// Raw data of the default face of an installed font `family`, resolved
/// through system font discovery as a runtime resolves a named family. The
/// repository commits no fonts: `backends/hydrolysis/test-fonts/install.py`
/// installs the fixtures, and a family that is not installed fails naming it.
#[cfg(test)]
pub fn installed_font_bytes(family: &str) -> std::sync::Arc<[u8]> {
    let mut collection =
        parley::fontique::Collection::new(parley::fontique::CollectionOptions::default());
    let info = collection.family_by_name(family).unwrap_or_else(|| {
        panic!(
            "font family `{family}` is not installed; install the test fonts with \
             `uv run backends/hydrolysis/test-fonts/install.py`"
        )
    });
    let face = info
        .default_font()
        .unwrap_or_else(|| panic!("the installed `{family}` family carries no face"));
    std::sync::Arc::from(
        face.load(None)
            .unwrap_or_else(|| panic!("the installed `{family}` face failed to load"))
            .as_ref(),
    )
}

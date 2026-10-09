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
pub use super::web_fonts::WebFonts;

/// The fallback role a resource face plays, recognized from its name.
///
/// The name is normalized (lowercased, spaces stripped) so the same rules
/// cover native file names (`NotoSansCJKsc-Regular.otf`) and web manifest
/// family names (`Noto Sans CJK SC`).
#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FontRole {
    /// The generic sans-serif face (Roboto).
    Generic,
    /// The emoji face.
    Emoji,
    /// Han in its Simplified Chinese design.
    HaniSimplified,
    /// Han in its Traditional Chinese design.
    HaniTraditional,
    /// Han in its Japanese design.
    HaniJapanese,
    /// Han in its Korean design, and Hangul.
    HaniKorean,
    /// Arabic script.
    Arabic,
    /// Hebrew script.
    Hebrew,
    /// Thai script.
    Thai,
    /// Devanagari script.
    Devanagari,
}

#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
impl FontRole {
    /// The role the face `name` plays; `None` for a face that answers only
    /// to its own family name — an icon face, a text face a theme names.
    pub(super) fn of(name: &str) -> Option<Self> {
        let key = name.to_ascii_lowercase().replace(' ', "");
        let rules = [
            ("emoji", Self::Emoji),
            ("roboto", Self::Generic),
            ("notosanscjksc", Self::HaniSimplified),
            ("notosanscjktc", Self::HaniTraditional),
            ("notosanscjkjp", Self::HaniJapanese),
            ("notosanscjkkr", Self::HaniKorean),
            ("notosansarabic", Self::Arabic),
            ("notosanshebrew", Self::Hebrew),
            ("notosansthai", Self::Thai),
            ("notosansdevanagari", Self::Devanagari),
        ];
        rules
            .into_iter()
            .find(|(pattern, _)| key.contains(pattern))
            .map(|(_, role)| role)
    }
}

#[cfg(any(test, all(target_arch = "wasm32", feature = "web")))]
impl FontRole {
    /// Whether a reader whose language is `tag` — a BCP 47 tag, as
    /// `navigator.languages` lists them — reads text this role's face draws.
    ///
    /// The generic face draws Latin for every reader; the emoji face belongs
    /// to no language. Chinese reads the Traditional design under the `Hant`
    /// script or a Traditional region, and the Simplified one otherwise.
    pub(super) fn serves(self, tag: &str) -> bool {
        let mut subtags = tag.split(['-', '_']);
        let language = subtags.next().unwrap_or_default().to_ascii_lowercase();
        let mut script = None;
        let mut region = None;
        for subtag in subtags {
            if subtag.len() == 4 && subtag.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                script = Some(subtag.to_ascii_lowercase());
            } else if subtag.len() == 2 && subtag.bytes().all(|byte| byte.is_ascii_alphabetic())
                || subtag.len() == 3 && subtag.bytes().all(|byte| byte.is_ascii_digit())
            {
                region = Some(subtag.to_ascii_uppercase());
            }
        }
        let traditional_chinese = language == "zh"
            && (script.as_deref() == Some("hant")
                || script.is_none()
                    && region
                        .as_deref()
                        .is_some_and(|region| matches!(region, "TW" | "HK" | "MO")));
        match self {
            Self::Generic => true,
            Self::Emoji => false,
            Self::HaniSimplified => language == "zh" && !traditional_chinese,
            Self::HaniTraditional => traditional_chinese,
            Self::HaniJapanese => language == "ja",
            Self::HaniKorean => language == "ko",
            Self::Arabic => matches!(
                language.as_str(),
                "ar" | "fa" | "ur" | "ps" | "sd" | "ug" | "ckb"
            ),
            Self::Hebrew => matches!(language.as_str(), "he" | "iw" | "yi"),
            Self::Thai => language == "th",
            Self::Devanagari => matches!(
                language.as_str(),
                "hi" | "mr" | "ne" | "sa" | "mai" | "kok" | "bho" | "doi"
            ),
        }
    }
}

/// Whether the face `name` loads before a web page's first frame, for a
/// visitor whose preferred languages are `languages`.
///
/// The page's default family, every face with no fallback role (selected by
/// its own name, so the first frame may draw it) and the generic face load
/// first; a script face loads first when one of the visitor's languages reads
/// that script. Every other face — the scripts the visitor does not read, and
/// the emoji face — loads after the first frame.
#[cfg(any(test, all(target_arch = "wasm32", feature = "web")))]
#[must_use]
pub fn loads_before_first_frame(name: &str, default_family: &str, languages: &[String]) -> bool {
    name == default_family
        || FontRole::of(name).is_none_or(|role| {
            role == FontRole::Generic || languages.iter().any(|language| role.serves(language))
        })
}

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
    /// Classify registered font families into fallback buckets by font name
    /// (see [`FontRole::of`]).
    pub(super) fn classify(&mut self, name: &str, families: &[(FamilyId, Vec<FontInfo>)]) {
        let Some(role) = FontRole::of(name) else {
            return;
        };
        let bucket = match role {
            FontRole::Generic => &mut self.generic,
            FontRole::Emoji => &mut self.emoji,
            FontRole::HaniSimplified => &mut self.hani_simplified,
            FontRole::HaniTraditional => &mut self.hani_traditional,
            FontRole::HaniJapanese => &mut self.hani_japanese,
            FontRole::HaniKorean => &mut self.hani_korean,
            FontRole::Arabic => &mut self.arabic,
            FontRole::Hebrew => &mut self.hebrew,
            FontRole::Thai => &mut self.thai,
            FontRole::Devanagari => &mut self.devanagari,
        };
        bucket.extend(families.iter().map(|(family_id, _)| *family_id));
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

/// The collection this platform shapes text with: on Android the faces the
/// platform font directories ship, elsewhere [`native_collection`] — the
/// system's fonts plus the application's staged font resources.
///
/// Android has no resource-directory scan: `native_collection` would only
/// look where nothing on that platform stages fonts, so the collection the
/// Android session builds from `/system/fonts` and its overlay partitions
/// answers instead. The environment is unused there — Android collections
/// carry no `ResourceContext`-rooted files — and used everywhere else.
#[cfg(target_os = "android")]
#[must_use]
pub fn platform_collection(_env: &waterui_core::Environment) -> FontCollection {
    android_collection()
}

/// The collection this platform shapes text with — [`native_collection`] on
/// every non-Android, non-wasm target. See the Android variant above.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[must_use]
pub fn platform_collection(env: &waterui_core::Environment) -> FontCollection {
    native_collection(env)
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
/// directory to scan and no `WebFonts` collection to build on.
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

#[cfg(test)]
mod tests {
    use super::{FontRole, loads_before_first_frame};

    fn languages(tags: &[&str]) -> Vec<String> {
        tags.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn chinese_reads_the_simplified_or_traditional_design_by_script_and_region() {
        for tag in ["zh", "zh-CN", "zh-Hans", "zh-Hans-HK", "zh-SG"] {
            assert!(FontRole::HaniSimplified.serves(tag), "{tag}");
            assert!(!FontRole::HaniTraditional.serves(tag), "{tag}");
        }
        for tag in ["zh-TW", "zh-HK", "zh-Hant", "zh-Hant-CN", "zh-MO"] {
            assert!(FontRole::HaniTraditional.serves(tag), "{tag}");
            assert!(!FontRole::HaniSimplified.serves(tag), "{tag}");
        }
        assert!(FontRole::HaniJapanese.serves("ja-JP"));
        assert!(FontRole::HaniKorean.serves("ko"));
        assert!(!FontRole::HaniJapanese.serves("zh-CN"));
    }

    #[test]
    fn the_first_frame_waits_for_the_faces_the_visitor_reads() {
        let zh = languages(&["zh-CN", "en-US"]);
        assert!(loads_before_first_frame("Roboto", "Roboto", &zh));
        assert!(loads_before_first_frame("Noto Sans CJK SC", "Roboto", &zh));
        assert!(!loads_before_first_frame("Noto Sans CJK JP", "Roboto", &zh));
        assert!(!loads_before_first_frame("Noto Sans Arabic", "Roboto", &zh));
        assert!(!loads_before_first_frame("Noto Color Emoji", "Roboto", &zh));
        // A face with no fallback role is selected by name, so the first
        // frame may draw it.
        assert!(loads_before_first_frame("Material Icons", "Roboto", &zh));
        // The default family loads first whatever its role.
        assert!(loads_before_first_frame(
            "Noto Sans CJK JP",
            "Noto Sans CJK JP",
            &zh
        ));
        // With no preferred language only the generic face is needed.
        assert!(loads_before_first_frame("Roboto", "Inter", &[]));
        assert!(!loads_before_first_frame("Noto Sans Hebrew", "Roboto", &[]));
    }
}

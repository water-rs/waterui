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
    /// Noto Sans's Latin, Greek and Cyrillic fallback coverage.
    LatinGreekCyrillic,
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
        if key == "notosans" || key.starts_with("notosans-") {
            return Some(Self::LatinGreekCyrillic);
        }
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
            Self::Generic | Self::LatinGreekCyrillic => true,
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
            matches!(role, FontRole::Generic | FontRole::LatinGreekCyrillic)
                || languages.iter().any(|language| role.serves(language))
        })
}

/// Font-family buckets recognized from `WaterUI`'s bundled resource fonts.
#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
#[derive(Default)]
pub(super) struct ResourceFontFamilies {
    generic: Vec<FamilyId>,
    latin_greek_cyrillic: Vec<FamilyId>,
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

/// Install `families` for `key` ahead of any families a platform source
/// already registered: an application resource face outranks the platform
/// faces in its script's fallback list but never replaces them.
#[cfg(any(not(target_arch = "wasm32"), feature = "web"))]
fn set_fallbacks(
    collection: &mut Collection,
    key: impl Into<FallbackKey> + Copy,
    families: &[FamilyId],
) {
    if families.is_empty() {
        return;
    }
    let mut combined = families.to_vec();
    for family in collection.fallback_families(key) {
        if !combined.contains(&family) {
            combined.push(family);
        }
    }
    assert!(
        collection.set_fallbacks(key, combined.iter().copied()),
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
            FontRole::LatinGreekCyrillic => &mut self.latin_greek_cyrillic,
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

    /// Install generic-family defaults and script fallbacks, including
    /// locale-specific Han families.
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

        for script in [
            Script::from_bytes(*b"Latn"),
            Script::from_bytes(*b"Grek"),
            Script::from_bytes(*b"Cyrl"),
        ] {
            set_fallbacks(collection, script, &self.latin_greek_cyrillic);
        }

        let hani = Script::from_bytes(*b"Hani");
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
        set_fallbacks(collection, Script::from_bytes(*b"Hang"), &self.hani_korean);
        set_fallbacks(collection, Script::from_bytes(*b"Arab"), &self.arabic);
        set_fallbacks(collection, Script::from_bytes(*b"Hebr"), &self.hebrew);
        set_fallbacks(collection, Script::from_bytes(*b"Thai"), &self.thai);
        set_fallbacks(collection, Script::from_bytes(*b"Deva"), &self.devanagari);
    }
}

/// Whether `path` names a font file this loader registers.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn is_font_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ttf" | "otf" | "ttc" | "otc")
    )
}

/// The directory an application's staged fonts live in.
///
/// Everywhere but Android that is the fonts directory the host installed. An
/// Android package namespaces exactly one asset tree, the `waterui_assets`
/// bundle `HydrolysisEnvironment` syncs, and the host application wins every
/// path shared beside it (water-rs/waterui#2348), so the build stages the
/// faces inside that bundle and they are read from there.
#[cfg(not(target_arch = "wasm32"))]
fn staged_fonts_dir(resources: &waterui_core::ResourceContext) -> std::path::PathBuf {
    #[cfg(target_os = "android")]
    return resources.assets().join(waterui_assets::STAGED_FONTS_DIR);
    #[cfg(not(target_os = "android"))]
    resources.fonts().to_path_buf()
}

/// The platform's own fonts plus every font file under the application's
/// staged fonts directory and every file of the environment's
/// [`DeclaredFonts`], with the recognized script fallbacks installed.
///
/// The platform's own faces are the ones `fontique` enumerates, except on
/// Android: fontique's Android backend reads the deprecated
/// `/system/etc/fonts.xml` under `/system` only and keeps a single family
/// per script or locale, so `android_fonts::platform_font_context`
/// registers the faces `ASystemFontIterator` enumerates, keyed by the
/// locale the platform declares. The application's half is the same on
/// every target.
///
/// The fonts directory is `staged_fonts_dir` of the environment's required
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
    let root = staged_fonts_dir(waterui_core::ResourceContext::from_environment(env));
    let mut resource_fonts = ResourceFontFamilies::default();
    #[cfg(target_os = "android")]
    let mut font_cx = super::android_fonts::platform_font_context();
    #[cfg(not(target_os = "android"))]
    let mut font_cx = parley::FontContext::new();
    if root.is_dir() {
        let entries = std::fs::read_dir(&root).unwrap_or_else(|error| {
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
            if !is_font_file(&path) {
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
    FontCollection::from_parley(font_cx)
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
    FontCollection::from_parley(parley::FontContext::new())
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

    /// A staged resource root under `dir`: the fonts directory a non-Android
    /// host installs, carrying `files`, and the environment an application
    /// is constructed in.
    #[cfg(not(target_arch = "wasm32"))]
    fn staged_environment(
        dir: &std::path::Path,
        files: &[(&str, &[u8])],
    ) -> (waterui_core::Environment, std::path::PathBuf) {
        let fonts = dir.join("fonts");
        std::fs::create_dir_all(&fonts).expect("the staged fonts directory is created");
        for (name, bytes) in files {
            std::fs::write(fonts.join(name), bytes).expect("a staged file is written");
        }
        let mut env = waterui_core::Environment::new();
        env.insert(waterui_core::ResourceContext::new(
            dir.join("waterui_assets"),
            &fonts,
        ));
        (env, fonts)
    }

    /// The application's own faces reach the collection every native target
    /// builds, and reach it as the fallback role their file name names: a
    /// staged Roboto is the collection's sans-serif, which no platform's
    /// unconfigured collection resolves to a family of that name
    /// (water-rs/waterui#2433).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_staged_face_is_registered_with_its_fallback_role() {
        let roboto = super::installed_font_bytes("Roboto");
        let dir = tempfile::tempdir().expect("tempdir");
        let (env, _) = staged_environment(
            dir.path(),
            &[
                ("Roboto-Regular.ttf", &roboto),
                // Only font files are read: the loader panics on a file it
                // cannot parse, so a stray manifest beside the faces would
                // take the process down.
                ("waterui-fonts.json", b"{}"),
            ],
        );

        super::native_collection(&env).with_parley(|fonts| {
            let staged = fonts
                .collection
                .family_id("Roboto")
                .expect("the staged face's family is registered");
            assert!(
                fonts
                    .collection
                    .generic_families(parley::fontique::GenericFamily::SansSerif)
                    .any(|family| family == staged),
                "the staged generic face answers sans-serif"
            );
        });
    }

    /// A face a dependency declares is registered too, wherever its file
    /// sits — an icon crate's fetched face is never inside the staged tree.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_declared_face_is_registered_from_outside_the_staged_tree() {
        let bungee = super::installed_font_bytes("Bungee Color");
        let dir = tempfile::tempdir().expect("tempdir");
        let declared = dir.path().join("BungeeColor-Regular.ttf");
        std::fs::write(&declared, &bungee).expect("the declared face is written");
        let (mut env, _) = staged_environment(dir.path(), &[]);
        env.insert(crate::text::DeclaredFonts::new([declared]));

        super::native_collection(&env).with_parley(|fonts| {
            assert!(
                fonts.collection.family_id("Bungee Color").is_some(),
                "the declared face's family is registered"
            );
        });
    }

    /// An application that staged nothing still gets the platform's own
    /// faces — the collection is the platform's plus the application's,
    /// never one or the other.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_platform_s_own_faces_are_always_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut env = waterui_core::Environment::new();
        env.insert(waterui_core::ResourceContext::new(
            dir.path().join("waterui_assets"),
            dir.path().join("fonts"),
        ));

        super::native_collection(&env).with_parley(|fonts| {
            assert!(
                fonts.collection.family_names().next().is_some(),
                "the platform's collection enumerates its faces"
            );
        });
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
        assert!(loads_before_first_frame("Noto Sans", "Roboto", &[]));
        assert!(!loads_before_first_frame("Noto Sans Hebrew", "Roboto", &[]));
    }

    #[test]
    fn noto_sans_is_a_script_fallback_not_the_default_generic_face() {
        for name in ["Noto Sans", "NotoSans-Regular.ttf", "NotoSans-Bold.ttf"] {
            assert_eq!(FontRole::of(name), Some(FontRole::LatinGreekCyrillic));
        }
        assert_eq!(FontRole::of("Roboto"), Some(FontRole::Generic));
        assert_eq!(
            FontRole::of("Noto Sans CJK SC"),
            Some(FontRole::HaniSimplified)
        );
        assert_eq!(FontRole::of("Noto Sans Arabic"), Some(FontRole::Arabic));
        assert_eq!(FontRole::of("Noto Sans Cherokee"), None);
    }
}

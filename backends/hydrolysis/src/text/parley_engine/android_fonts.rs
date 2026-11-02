//! Android font discovery.
//!
//! fontique's Android backend is not the source here: it reads
//! `/system/etc/fonts.xml`, which AOSP marks deprecated and "no longer a
//! source of the font files installed", scans `/system/fonts` only, keeps
//! one family per script or locale, and resolves faces by PostScript name —
//! AOSP writes the same `NotoSansCJKJP-Regular` name for every CJK index,
//! so per-locale Han designs cannot come through it. The platform publishes
//! each face's file, collection index and BCP 47 locale through
//! `ASystemFontIterator`, which AOSP names as the supported channel and
//! `ndk::font` wraps; this module registers those faces itself, keyed by
//! the locale the platform declares.
//!
//! What this module supplies is only the platform's own faces:
//! `fonts::native_collection` adds the application's staged and declared
//! fonts on top, on Android exactly as on every other native target.

use icu_locale::{LanguageIdentifier, LocaleExpander};
use parley::fontique::{FallbackKey, Language, Script};

/// The scripts whose clusters a face for the ISO 15924 code `script`
/// answers. CLDR's likely-subtags data yields the composite codes `Jpan`
/// (Han, Hiragana and Katakana clusters) and `Kore` (Hangul and Han), and
/// a tag can spell out the Han-variant codes `Hans`/`Hant`, whose text
/// clusters all select `Hani`.
fn expanded_scripts(script: Script) -> Vec<Script> {
    match &script.to_bytes() {
        b"Hans" | b"Hant" => vec![Script::from_bytes(*b"Hani")],
        b"Jpan" => [*b"Hani", *b"Hira", *b"Kana"]
            .map(Script::from_bytes)
            .to_vec(),
        b"Kore" => [*b"Hang", *b"Hani"].map(Script::from_bytes).to_vec(),
        _ => vec![script],
    }
}

/// The fallback keys a platform face tagged `locale` answers, in install
/// order.
///
/// `und-Xxxx` is the platform's spelling of "a face for script `Xxxx`".
/// Any other tag keys the face under each bare script its language is
/// written in — CLDR likely-subtags data supplies the script when the tag
/// omits one — plus every locale-qualified pair fontique tracks for those
/// scripts: the Han designs differ per locale, so a `zh-Hant` face must
/// answer `(Hani, zh-Hant)`, not just bare `Hani`. A pair fontique does
/// not track resolves through the bare script, and a pair canonical to
/// the script's default locale is the bare script key itself.
pub(super) fn fallback_keys(locale: &str) -> Vec<FallbackKey> {
    let Ok(mut langid) = locale.parse::<LanguageIdentifier>() else {
        return Vec::new();
    };
    if langid.script.is_none() && !langid.language.is_unknown() {
        LocaleExpander::new_extended().maximize(&mut langid);
    }
    let Some(script) = langid.script else {
        return Vec::new();
    };
    let scripts = expanded_scripts(Script::from_bytes(script.into_raw()));
    if langid.language.is_unknown() {
        return scripts.into_iter().map(FallbackKey::from).collect();
    }
    let language = Language::parse(locale).ok();
    let mut keys: Vec<FallbackKey> = Vec::new();
    let mut push = |key: FallbackKey| {
        let key = if key.is_tracked() && !key.is_default() {
            key
        } else {
            FallbackKey::new(key.script(), None)
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    };
    for script in scripts {
        if let Some(language) = language {
            push(FallbackKey::new(script, Some(&language)));
        }
        push(FallbackKey::new(script, None));
    }
    keys
}

/// A font context holding the faces Android itself supplies: every face
/// [`SystemFontIterator`] enumerates, registered once per file and keyed
/// by the locale the platform declares for it. `system_fonts` stays off —
/// fontique's Android backend reads the deprecated `/system/etc/fonts.xml`
/// under `/system` only, keeps a single family per key, and resolves faces
/// by a PostScript name AOSP writes identically for every CJK index; the
/// per-face locale data the collection needs is only on the NDK iterator.
///
/// This is the platform half of [`super::fonts::native_collection`], which
/// adds the application's staged and declared fonts and installs their
/// fallbacks ahead of the platform lists; nothing else builds a
/// collection on Android.
#[cfg(target_os = "android")]
pub(super) fn platform_font_context() -> parley::FontContext {
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use ndk::font::SystemFontIterator;
    use parley::fontique::{Collection, CollectionOptions, FamilyId, SourceCache};

    let mut font_cx = parley::FontContext {
        collection: Collection::new(CollectionOptions {
            system_fonts: false,
            ..CollectionOptions::default()
        }),
        source_cache: SourceCache::default(),
    };
    let Some(iterator) = SystemFontIterator::new() else {
        tracing::warn!(
            target: "waterui::hydrolysis::android",
            "ASystemFontIterator could not be opened"
        );
        return font_cx;
    };
    // Faces registered per file, by collection index: `register_fonts`
    // runs once per path, so the `AFont`s of one file share the blob.
    let mut files: HashMap<PathBuf, Vec<(usize, FamilyId)>> = HashMap::new();
    // The `(key, family)` pairs already installed and the families
    // already appended to a generic list: the iterator lists a face per
    // style, and a family must not repeat in a fallback list.
    let mut installed: HashSet<(FallbackKey, FamilyId)> = HashSet::new();
    let mut generic_faces: HashSet<FamilyId> = HashSet::new();
    for font in iterator {
        let Some(faces) = file_faces(&mut font_cx, &mut files, font.path()) else {
            continue;
        };
        install_face(
            &mut font_cx,
            &font,
            faces,
            &mut installed,
            &mut generic_faces,
        );
    }
    font_cx
}

/// The faces of the font file at `path` — the family each collection
/// index registered as — registering the file's blob on first use.
#[cfg(target_os = "android")]
fn file_faces<'a>(
    font_cx: &mut parley::FontContext,
    files: &'a mut std::collections::HashMap<
        std::path::PathBuf,
        Vec<(usize, parley::fontique::FamilyId)>,
    >,
    path: &std::path::Path,
) -> Option<&'a mut Vec<(usize, parley::fontique::FamilyId)>> {
    use std::collections::hash_map::Entry;
    use std::sync::Arc;

    use parley::fontique::Blob;

    match files.entry(path.to_path_buf()) {
        Entry::Occupied(entry) => Some(entry.into_mut()),
        Entry::Vacant(entry) => {
            let Ok(bytes) = std::fs::read(path) else {
                tracing::warn!(
                    target: "waterui::hydrolysis::android",
                    path = %path.display(),
                    "font file could not be read — skipped"
                );
                return None;
            };
            let families = font_cx
                .collection
                .register_fonts(Blob::new(Arc::new(bytes)), None);
            Some(
                entry.insert(
                    families
                        .iter()
                        .flat_map(|(family, infos)| {
                            infos
                                .iter()
                                .map(move |info| (info.index() as usize, *family))
                        })
                        .collect(),
                ),
            )
        }
    }
}

/// Install `font`'s family under the fallback keys its platform-declared
/// locale maps to, or under the generic families when the platform tags
/// the face with no locale.
#[cfg(target_os = "android")]
fn install_face(
    font_cx: &mut parley::FontContext,
    font: &ndk::font::Font,
    faces: &[(usize, parley::fontique::FamilyId)],
    installed: &mut std::collections::HashSet<(FallbackKey, parley::fontique::FamilyId)>,
    generic_faces: &mut std::collections::HashSet<parley::fontique::FamilyId>,
) {
    use parley::fontique::GenericFamily;

    let path = font.path();
    let index = font.collection_index();
    let Some(&family) = faces
        .iter()
        .find(|(face_index, _)| *face_index == index)
        .map(|(_, family)| family)
    else {
        tracing::warn!(
            target: "waterui::hydrolysis::android",
            path = %path.display(),
            index,
            "platform font face has no registered family — skipped"
        );
        return;
    };
    let locale = match font.locale() {
        Some(locale) => match locale.to_str() {
            Ok(locale) => Some(locale),
            Err(error) => {
                tracing::warn!(
                    target: "waterui::hydrolysis::android",
                    path = %path.display(),
                    "platform font locale is not UTF-8 — {error}"
                );
                return;
            }
        },
        None => None,
    };
    let keys = locale.map(fallback_keys).unwrap_or_default();
    tracing::debug!(
        target: "waterui::hydrolysis::android",
        path = %path.display(),
        index,
        locale = locale.unwrap_or(""),
        keys = ?keys,
        generic = locale.is_none(),
        "registered platform font face"
    );
    match locale {
        Some(_) => {
            for key in keys {
                if installed.insert((key, family)) {
                    font_cx
                        .collection
                        .append_fallbacks(key, core::iter::once(family));
                    // Emoji clusters are shaped through the `emoji` generic
                    // family, not a script fallback, so the platform's
                    // emoji-script (ISO 15924 `Zsye`) faces answer it too.
                    if key.script() == Script::from_bytes(*b"Zsye") {
                        font_cx.collection.append_generic_families(
                            GenericFamily::Emoji,
                            core::iter::once(family),
                        );
                    }
                }
            }
        }
        // A face the platform tags with no locale is a default face,
        // answering the generic families.
        None => {
            if generic_faces.insert(family) {
                for generic in [
                    GenericFamily::SansSerif,
                    GenericFamily::UiSansSerif,
                    GenericFamily::SystemUi,
                ] {
                    font_cx
                        .collection
                        .append_generic_families(generic, core::iter::once(family));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use parley::fontique::{FallbackKey, Script};

    fn key(script: &str, locale: Option<&str>) -> FallbackKey {
        let script = Script::from_bytes(
            script
                .as_bytes()
                .try_into()
                .expect("an ISO 15924 script code is four bytes"),
        );
        locale.map_or_else(|| script.into(), |locale| (script, locale).into())
    }

    /// `und-Xxxx` keys the bare script; a language tag keys the bare
    /// scripts its language is written in plus the locale-qualified pair
    /// fontique tracks — `(Hani, zh-Hant)` is the Han design a
    /// Traditional-Chinese reader selects.
    #[test]
    fn a_platform_locale_maps_to_fallback_keys() {
        assert_eq!(super::fallback_keys("und-Arab"), [key("Arab", None)]);
        assert_eq!(super::fallback_keys("ar"), [key("Arab", None)]);
        assert_eq!(
            super::fallback_keys("ja"),
            [
                key("Hani", Some("ja")),
                key("Hani", None),
                key("Hira", None),
                key("Kana", None),
            ]
        );
        assert_eq!(
            super::fallback_keys("ko"),
            [
                key("Hang", None),
                key("Hani", Some("ko")),
                key("Hani", None)
            ]
        );
        assert_eq!(
            super::fallback_keys("zh-Hant"),
            [key("Hani", Some("zh-Hant")), key("Hani", None)]
        );
        assert_eq!(super::fallback_keys("zh-Hans"), [key("Hani", None)]);
        assert_eq!(super::fallback_keys("th"), [key("Thai", None)]);
        assert_eq!(super::fallback_keys("not-a-locale"), Vec::new());
    }
}

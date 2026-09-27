//! Resource font registration and locale-aware fallback installation.
//!
//! WaterUI ships a known set of resource fonts (Roboto plus script-specific
//! Noto Sans families). Classification and fallback installation are shared by the
//! native loader (which scans `resources/fonts` directories) and the web
//! loader in [`super::web_runner`] (which fetches fonts from a manifest).
//!
//! The result is built **once per application**, installed into the root
//! environment as the shared [`FontCollection`], and every window's renderer is
//! seeded from it by [`seed_renderer`]. Building it per window meant scanning
//! the resource directories and enumerating the system's fonts again for each
//! one, and a self-drawn component reading the environment would have had no
//! single collection to read.

use parley::fontique::{Collection, FallbackKey, FamilyId, FontInfo, GenericFamily, Script};
use waterui_text::FontCollection;

/// Font-family buckets recognized from WaterUI's bundled resource fonts.
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

fn extend_family_ids(target: &mut Vec<FamilyId>, families: &[(FamilyId, Vec<FontInfo>)]) {
    target.extend(families.iter().map(|(family_id, _)| *family_id));
}

fn set_fallbacks(collection: &mut Collection, key: impl Into<FallbackKey>, families: &[FamilyId]) {
    if families.is_empty() {
        return;
    }
    assert!(
        collection.set_fallbacks(key, families.iter().copied()),
        "hydrolysis font loader attempted to install an untracked script fallback"
    );
}

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

/// The faces a test host registers, and the ones it pins its generic families
/// to. Everything the renderer measures in a test is shaped through one of
/// these unless no bundled face maps the cluster.
#[cfg(any(test, feature = "testing"))]
const TEST_FONTS: &[(&str, &[u8])] = &[
    (
        "Roboto-Regular.ttf",
        include_bytes!("../../test-fonts/Roboto-Regular.ttf"),
    ),
    (
        "Roboto-Medium.ttf",
        include_bytes!("../../test-fonts/Roboto-Medium.ttf"),
    ),
    (
        "Roboto-Bold.ttf",
        include_bytes!("../../test-fonts/Roboto-Bold.ttf"),
    ),
    (
        "Roboto-Italic.ttf",
        include_bytes!("../../test-fonts/Roboto-Italic.ttf"),
    ),
];

/// The faces that answer what the bundled Roboto cannot cover, subsetted from
/// the Noto Sans families the runner ships to applications and classified
/// through the same [`ResourceFontFamilies`] path — so a cluster Roboto has no
/// glyph for resolves to a known face on every host, not to whatever the
/// platform happened to install.
///
/// Each file is a `pyftsubset` of the corresponding full face kept to the
/// sample strings the suite shapes: a host with no CJK, Hangul, Thai, or
/// Devanagari faces installed — a clean CI image counts — still answers every
/// script a `WaterUI` application is expected to draw. They are registered
/// like any other resource font but never pinned to a generic family, which
/// is what keeps them "not bundled" for the fallback assertions.
#[cfg(any(test, feature = "testing"))]
const TEST_FALLBACK_FONTS: &[(&str, &[u8])] = &[
    (
        "NotoSansCJKsc-Regular.otf",
        include_bytes!("../../test-fonts/NotoSansCJKsc-Regular.otf"),
    ),
    (
        "NotoSansCJKkr-Regular.otf",
        include_bytes!("../../test-fonts/NotoSansCJKkr-Regular.otf"),
    ),
    (
        "NotoSansArabic-Regular.ttf",
        include_bytes!("../../test-fonts/NotoSansArabic-Regular.ttf"),
    ),
    (
        "NotoSansHebrew-Regular.ttf",
        include_bytes!("../../test-fonts/NotoSansHebrew-Regular.ttf"),
    ),
    (
        "NotoSansThai-Regular.ttf",
        include_bytes!("../../test-fonts/NotoSansThai-Regular.ttf"),
    ),
    (
        "NotoSansDevanagari-Regular.ttf",
        include_bytes!("../../test-fonts/NotoSansDevanagari-Regular.ttf"),
    ),
    // The colour-emoji face a host carries: classifies into the `emoji`
    // generic family so emoji-presentation clusters shape — and rasterize
    // through the bitmap image atlas — the way they do in production.
    (
        "NotoColorEmojiSubset.ttf",
        include_bytes!("../../test-fonts/NotoColorEmojiSubset.ttf"),
    ),
];

/// The collection a test host shapes with: the bundled Roboto for everything it
/// covers, the bundled Noto subsets for everything it does not.
///
/// Test text used to shape against whatever the host OS discovered first, so a
/// layout assertion tuned on one platform's metrics failed on another's fonts.
/// Pinning the generic families to exactly the Roboto files bundled with this
/// crate is what fixed the half of that about selection: Latin and Cyrillic
/// measure identically on every runner because a family the collection
/// registered itself is matched ahead of any system family of the same generic.
///
/// The fallback half had the same disease one layer down. System discovery
/// stays on so a cluster the pinned face has no glyph for can be answered, but
/// the host's font set is not deterministic either — a clean Linux image
/// carries no CJK, Hangul, Thai, or Devanagari face at all, so every script
/// outside Roboto's coverage shaped to `.notdef` and drew as tofu, which is how
/// the avatar gallery came to render `山田 太郎`'s monogram as two empty boxes
/// while `Ольга Ладыженская`'s read correctly.
///
/// So the test host installs the same kind of fallback an application does:
/// [`TEST_FALLBACK_FONTS`] registers through the same `ResourceFontFamilies`
/// classify/install path the shipping runner's own collection takes, which is
/// what makes a script exercised here evidence about the renderer rather than
/// about the test host. Only a cluster no bundled face maps reaches the
/// platform — the last resort, as in the shipping runner.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn deterministic_test_fonts() -> parley::FontContext {
    use parley::fontique::{Blob, CollectionOptions};
    use std::sync::Arc;

    let mut font_cx = parley::FontContext {
        collection: Collection::new(CollectionOptions {
            // On for fallback, not for selection: `ResourceFontFamilies::install`
            // below pins the generic families to the bundled Roboto, so a
            // system face is only ever reached for a cluster Roboto cannot map.
            system_fonts: true,
            ..CollectionOptions::default()
        }),
        source_cache: parley::fontique::SourceCache::default(),
    };
    let mut resource_fonts = ResourceFontFamilies::default();
    for (name, bytes) in TEST_FONTS.iter().chain(TEST_FALLBACK_FONTS.iter()) {
        let families = font_cx
            .collection
            .register_fonts(Blob::new(Arc::new(*bytes)), None);
        resource_fonts.classify(name, &families);
    }
    resource_fonts.install(&mut font_cx.collection);
    font_cx
}

/// The system's fonts plus every `.ttf`/`.otf` under the app's `resources/fonts`
/// directories, with the recognized script fallbacks installed.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn native_resource_fonts() -> parley::FontContext {
    use parley::fontique::Blob;
    use std::sync::Arc;

    let mut roots = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        roots.push(current_dir.join("resources").join("fonts"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        roots.push(exe_dir.join("resources").join("fonts"));
        if let Some(contents_dir) = exe_dir.parent()
            && contents_dir
                .file_name()
                .is_some_and(|name| name == "Contents")
        {
            roots.push(
                contents_dir
                    .join("Resources")
                    .join("resources")
                    .join("fonts"),
            );
        }
    }

    let mut font_cx = parley::FontContext::new();
    let mut resource_fonts = ResourceFontFamilies::default();
    for root in roots {
        if !root.exists() {
            continue;
        }
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
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            if !extension.eq_ignore_ascii_case("ttf") && !extension.eq_ignore_ascii_case("otf") {
                continue;
            }
            let font_data = std::fs::read(&path).unwrap_or_else(|error| {
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
    }
    resource_fonts.install(&mut font_cx.collection);
    font_cx
}

/// Gives `core` the application's fonts to shape with.
///
/// Every window shapes against the one collection the runner installed, so a
/// popup opened later measures text exactly as the window that opened it does.
/// The renderer keeps its own copy because it shapes across worker threads and
/// `parley`'s contexts are not `Sync`; the faces in it are the same ones.
pub(super) fn seed_core(core: &mut crate::renderer::SemanticCore, fonts: &FontCollection) {
    *core.state_mut().text_fonts_mut() = fonts.use_fonts(|fonts| fonts.clone());
}

#[cfg(test)]
mod tests {
    use parley::PositionedLayoutItem;
    use waterui_core::Environment;
    use waterui_core::layout::HorizontalAlignment;
    use waterui_text::styled::StyledStr;

    use super::{TEST_FONTS, deterministic_test_fonts};
    use crate::renderer::{TextMeasureService, resolve_text_layout_input};

    /// One sample per script a `WaterUI` application is expected to draw.
    ///
    /// The first two are what the bundled Roboto covers and what every layout
    /// assertion in the suite is written against; the rest are the scripts that
    /// can only be answered by the platform's fallback, and each of them drew
    /// as a row of tofu boxes before this collection asked for one.
    const SAMPLES: &[(&str, &str)] = &[
        ("Latin", "Ada Lovelace"),
        ("Cyrillic", "Ольга Ладыженская"),
        ("Han", "山田 太郎"),
        ("Hangul", "안녕하세요"),
        ("Arabic", "مرحبا بالعالم"),
        ("Hebrew", "שלום עולם"),
        ("Thai", "สวัสดี"),
        ("Devanagari", "नमस्ते"),
    ];

    /// A shaping service seeded exactly as a headless test host seeds it.
    fn test_host_service() -> TextMeasureService {
        let mut service = TextMeasureService::new();
        *service.fonts_mut() = deterministic_test_fonts();
        service
    }

    /// Shapes `text` through the renderer's own service and reports how many
    /// glyphs came back, how many of them are `.notdef`, and whether every run
    /// resolved to one of the bundled faces.
    ///
    /// `.notdef` is glyph 0 by definition, and glyph 0 is the box a reader sees.
    /// Counting it is the same observation as reading the image, made where it
    /// cannot be argued with.
    fn shaped(service: &TextMeasureService, text: &'static str) -> (usize, usize, bool) {
        let mut env = Environment::new();
        crate::testing::install_theme(&mut env);
        let input =
            resolve_text_layout_input(&StyledStr::from(text), HorizontalAlignment::Leading, &env);
        let layout = service.shape(&input, None);
        let mut glyphs = 0;
        let mut missing = 0;
        let mut all_bundled = true;
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let face = run.run().font().data.data();
                all_bundled &= TEST_FONTS.iter().any(|(_, bundled)| face == *bundled);
                for glyph in run.glyphs() {
                    glyphs += 1;
                    missing += usize::from(glyph.id == 0);
                }
            }
        }
        (glyphs, missing, all_bundled)
    }

    /// The defect this collection was fixed for: a cluster the bundled face
    /// cannot map must reach a face that can, on every script, not just the
    /// ones Roboto happens to carry.
    #[test]
    fn no_script_shapes_to_a_missing_glyph() {
        let service = test_host_service();
        for (script, text) in SAMPLES {
            let (glyphs, missing, _) = shaped(&service, text);
            assert!(glyphs > 0, "{script} sample `{text}` produced no glyphs");
            assert_eq!(
                missing, 0,
                "{missing} of {glyphs} glyphs in the {script} sample `{text}` are `.notdef`, \
                 which is the tofu box: the collection found no face covering the script"
            );
        }
    }

    /// ...and the half that must not move while it does: pinning the generic
    /// families to the bundled Roboto is what keeps a layout assertion tuned on
    /// one runner true on the next, so the scripts Roboto covers have to keep
    /// resolving to Roboto rather than to whatever the host installed.
    #[test]
    fn the_scripts_roboto_covers_still_shape_through_roboto() {
        let service = test_host_service();
        for (script, text) in &SAMPLES[..2] {
            let (glyphs, _, all_bundled) = shaped(&service, text);
            assert!(glyphs > 0, "{script} sample `{text}` produced no glyphs");
            assert!(
                all_bundled,
                "the {script} sample `{text}` reached a system face; the bundled Roboto \
                 covers it and must be matched first, or every metric in the suite \
                 becomes host-dependent"
            );
        }
    }

    /// Long enough that a narrow proposal can only answer it with many lines,
    /// and mixing Japanese kana and kanji with Chinese hanzi so every CJ
    /// complex-script classification passes through the word segmenter.
    const CJK_PARAGRAPH: &str = "こんにちは世界。これは日本語のテキストです。雨にも負けず風にも負けず。中文也可以排版，天涯海角任我行。";

    /// UAX #14 already allows a line break between adjacent ideographs, so a
    /// narrow proposal must wrap a CJK paragraph into several lines. And with
    /// the bundled segmentation dictionaries the segmenter answers every
    /// complex-script run with a model, so the layout pass must stay silent:
    /// `No segmentation model for complex script` was the warn/debug record a
    /// missing model emitted once per CJK run per layout.
    #[test]
    fn a_cjk_paragraph_wraps_at_ideographic_boundaries_and_logs_nothing() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct LogBuffer(Arc<Mutex<Vec<u8>>>);
        impl Write for LogBuffer {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .expect("log capture buffer mutex poisoned")
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
            type Writer = LogBuffer;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_max_level(tracing::Level::DEBUG)
            .without_time()
            .with_ansi(false)
            .finish();
        // ICU4X data warnings travel through `log`; `LogTracer` lands them in
        // the same subscriber, so a missing model could not hide behind the
        // `eprintln` path debug builds take without `icu_provider/logging`.
        tracing_log::LogTracer::init().ok();

        let mut lines = 0;
        tracing::subscriber::with_default(subscriber, || {
            let service = test_host_service();
            let mut env = Environment::new();
            crate::testing::install_theme(&mut env);
            let input = resolve_text_layout_input(
                &StyledStr::from(CJK_PARAGRAPH),
                HorizontalAlignment::Leading,
                &env,
            );
            let layout = service.shape(&input, Some(80.0));
            lines = layout.lines().count();
        });

        assert!(
            lines >= 3,
            "a CJK paragraph in an 80px width must wrap at ideographic boundaries; got {lines} line(s)"
        );
        let logged = String::from_utf8(
            buffer
                .0
                .lock()
                .expect("log capture buffer mutex poisoned")
                .clone(),
        )
        .expect("captured log output must be UTF-8");
        assert!(
            !logged.contains("segmentation model") && !logged.contains("icu_segmenter"),
            "laying out CJK text must not log segmentation-model misses: {logged}"
        );
    }

    /// A DejaVu-Sans subset kept to ASCII and the emoji codepoints the
    /// emoji-family test exercises — including the ones that *also* live in a
    /// colour face. DejaVu covering `U+1F600` at all is the whole point of
    /// <https://github.com/water-rs/hydrolysis/issues/119>: a desktop sans
    /// carries monochrome glyphs for emoji-presentation codepoints, so which
    /// family answers the cluster decides whether it draws colour or flat
    /// text.
    const TEXT_WITH_EMOJI_COVERAGE: &[u8] =
        include_bytes!("../../test-fonts/DejaVuSansEmojiCoverage.ttf");

    /// A Noto Color Emoji subset over the same codepoints, playing the
    /// `emoji` generic family.
    const EMOJI_FACE: &[u8] = include_bytes!("../../test-fonts/NotoColorEmojiSubset.ttf");

    /// A shaping service whose collection holds exactly the two fixture
    /// faces, each pinned to its generic family: the text face at
    /// `sans-serif`, the colour face at `emoji`. Both cover every emoji
    /// codepoint the test shapes, so which blob a run resolves to reports
    /// which family list the cluster consulted first — host fonts cannot
    /// leak in because system discovery is off.
    fn emoji_fixture_service() -> TextMeasureService {
        use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily};
        use std::sync::Arc;

        let mut font_cx = parley::FontContext {
            collection: Collection::new(CollectionOptions {
                system_fonts: false,
                ..CollectionOptions::default()
            }),
            source_cache: parley::fontique::SourceCache::default(),
        };
        let text_families = font_cx
            .collection
            .register_fonts(Blob::new(Arc::new(TEXT_WITH_EMOJI_COVERAGE)), None);
        let emoji_families = font_cx
            .collection
            .register_fonts(Blob::new(Arc::new(EMOJI_FACE)), None);
        font_cx.collection.set_generic_families(
            GenericFamily::SansSerif,
            text_families.iter().map(|(family_id, _)| *family_id),
        );
        font_cx.collection.set_generic_families(
            GenericFamily::Emoji,
            emoji_families.iter().map(|(family_id, _)| *family_id),
        );
        let mut service = TextMeasureService::new();
        *service.fonts_mut() = font_cx;
        service
    }

    /// <https://github.com/water-rs/hydrolysis/issues/119>: a cluster whose
    /// presentation is emoji — an `Emoji_Presentation=Yes` codepoint, or an
    /// `Emoji=Yes` base followed by U+FE0F — must resolve through the `emoji`
    /// generic family *before* the text families. `parley` instead appends
    /// the emoji fallback after the requested family, so a text face that
    /// covers the codepoint wins and the cluster draws monochrome.
    #[test]
    fn emoji_presentation_clusters_resolve_through_the_emoji_family_first() {
        let service = emoji_fixture_service();
        let mut env = Environment::new();
        crate::testing::install_theme(&mut env);

        let faces_of = |text: &'static str| -> Vec<Vec<u8>> {
            let input = resolve_text_layout_input(
                &StyledStr::from(text),
                HorizontalAlignment::Leading,
                &env,
            );
            let layout = service.shape(&input, None);
            layout
                .lines()
                .flat_map(|line| line.items())
                .filter_map(|item| {
                    let PositionedLayoutItem::GlyphRun(run) = item else {
                        return None;
                    };
                    Some(run.run().font().data.data().to_vec())
                })
                .collect()
        };

        // Emoji presentation: the bare `Emoji_Presentation=Yes` codepoints
        // and the `Emoji=Yes` bases followed by U+FE0F must all come back in
        // the face pinned at `emoji`, never in the text face that also
        // covers them.
        for text in [
            "\u{1F600}",
            "\u{2764}\u{FE0F}",
            "\u{2615}",
            "\u{26A0}\u{FE0F}",
            "\u{26A1}",
        ] {
            let faces = faces_of(text);
            assert!(!faces.is_empty(), "`{text}` produced no glyph runs");
            for face in &faces {
                assert_eq!(
                    face.as_slice(),
                    EMOJI_FACE,
                    "emoji-presentation `{text}` resolved to the text face"
                );
            }
        }

        // Text presentation keeps the text face: Latin, bare `Emoji=Yes`
        // codepoints whose `Emoji_Presentation` is `No`, and one followed by
        // U+FE0E all stay monochrome.
        for text in ["plain", "\u{2764}", "\u{26A0}", "\u{2764}\u{FE0E}"] {
            let faces = faces_of(text);
            assert!(!faces.is_empty(), "`{text}` produced no glyph runs");
            for face in &faces {
                assert_eq!(
                    face.as_slice(),
                    TEXT_WITH_EMOJI_COVERAGE,
                    "text-presentation `{text}` left the text face"
                );
            }
        }

        // Mixed text: the emoji cluster inside Latin text takes the emoji
        // face while its ASCII neighbours keep the text face.
        let text = "a\u{1F600}b";
        let input =
            resolve_text_layout_input(&StyledStr::from(text), HorizontalAlignment::Leading, &env);
        let layout = service.shape(&input, None);
        let (mut saw_emoji, mut saw_text) = (false, false);
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let face = run.run().font().data.data();
                for cluster in run.run().clusters() {
                    if text[cluster.text_range()].contains('\u{1F600}') {
                        saw_emoji = true;
                        assert_eq!(
                            face, EMOJI_FACE,
                            "the emoji cluster in `{text}` resolved to the text face"
                        );
                    } else {
                        saw_text = true;
                        assert_eq!(
                            face, TEXT_WITH_EMOJI_COVERAGE,
                            "a text cluster in `{text}` left the text face"
                        );
                    }
                }
            }
        }
        assert!(
            saw_emoji && saw_text,
            "`{text}` produced no runs to assert on"
        );
    }

    /// And the same statement from the other side: a script Roboto does not
    /// carry must be answered by a face other than the pinned Roboto. Without this the
    /// test above could pass on a collection that had quietly stopped
    /// registering anything at all.
    #[test]
    fn a_script_roboto_lacks_is_answered_by_a_fallback_face() {
        let service = test_host_service();
        for (script, text) in &SAMPLES[2..] {
            let (_, _, all_bundled) = shaped(&service, text);
            assert!(
                !all_bundled,
                "the {script} sample `{text}` claims to shape through the bundled Roboto, \
                 which has no glyph for it"
            );
        }
    }
}

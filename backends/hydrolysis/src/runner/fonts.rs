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

use crate::renderer::HydrolysisRenderer;

/// Font-family buckets recognized from WaterUI's bundled resource fonts.
#[derive(Default)]
pub(super) struct ResourceFontFamilies {
    generic: Vec<FamilyId>,
    hani_simplified: Vec<FamilyId>,
    hani_traditional: Vec<FamilyId>,
    hani_japanese: Vec<FamilyId>,
    hani_korean: Vec<FamilyId>,
    arabic: Vec<FamilyId>,
    hebrew: Vec<FamilyId>,
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
        if key.contains("roboto") {
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
        set_fallbacks(collection, Script::from_str_unchecked("Arab"), &self.arabic);
        set_fallbacks(collection, Script::from_str_unchecked("Hebr"), &self.hebrew);
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

/// The collection a test host shapes with: the bundled Roboto for everything it
/// covers, the platform's own faces for everything it does not.
///
/// Test text used to shape against whatever the host OS discovered first, so a
/// layout assertion tuned on one platform's metrics failed on another's fonts.
/// Pinning the generic families to exactly the Roboto files bundled with this
/// crate is what fixed that, and it is why Latin and Cyrillic still measure
/// identically on every runner: a family the collection registered itself is
/// matched ahead of any system family of the same generic.
///
/// Turning system discovery off on top of that pinning did not help and cost
/// the platform *fallback*, which is the only thing that can answer a cluster
/// the pinned face has no glyph for. Every script outside Roboto's coverage —
/// Han, Hangul, Arabic, Hebrew, Thai, Devanagari — therefore shaped to
/// `.notdef` and drew as tofu, which is how the avatar gallery came to render
/// `山田 太郎`'s monogram as two empty boxes while `Ольга Ладыженская`'s read
/// correctly, and why no non-Latin text could be tested or reviewed by eye
/// anywhere in the framework.
///
/// So discovery stays on and the pinning does the deterministic half of the
/// job by itself. Only what Roboto cannot cover reaches the platform's
/// fallback — the same path the shipping runner's own collection takes, which
/// is what makes a script exercised here evidence about the renderer rather
/// than about the test host.
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
    for (name, bytes) in TEST_FONTS {
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

/// Gives `renderer` the application's fonts to shape with.
///
/// Every window shapes against the one collection the runner installed, so a
/// popup opened later measures text exactly as the window that opened it does.
/// The renderer keeps its own copy because it shapes across worker threads and
/// `parley`'s contexts are not `Sync`; the faces in it are the same ones.
pub(super) fn seed_renderer(renderer: &mut HydrolysisRenderer, fonts: &FontCollection) {
    *renderer.state_mut().text_fonts_mut() = fonts.use_fonts(|fonts| fonts.clone());
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

    /// And the same statement from the other side: a script Roboto does not
    /// carry must be answered by a face that is *not* bundled. Without this the
    /// test above could pass on a collection that had quietly stopped
    /// registering anything at all.
    #[test]
    fn a_script_roboto_lacks_is_answered_by_a_platform_face() {
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

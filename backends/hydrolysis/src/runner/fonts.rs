//! Resource font registration and locale-aware fallback installation.
//!
//! WaterUI ships a known set of resource fonts (Roboto plus script-specific
//! Noto Sans families). Classification and fallback installation are shared by the
//! native loader (which scans `resources/fonts` directories) and the web
//! loader in [`super::web_runner`] (which fetches fonts from a manifest).

use parley::fontique::{Collection, FallbackKey, FamilyId, FontInfo, GenericFamily, Script};

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

/// Replaces the renderer's font collection with the bundled deterministic set.
///
/// Test text used to shape against whatever the host OS discovered, so a
/// layout assertion tuned on one platform's metrics failed on another's fonts
/// and every snapshot golden was platform-specific. The test hosts shape with
/// exactly the Roboto files under `testing/fonts` instead — system discovery
/// off, identical metrics on every runner. Characters outside Roboto's
/// coverage shape as missing glyphs on purpose: a test that needs another
/// script should say so loudly rather than silently depending on the host's
/// fallback set.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn install_deterministic_test_fonts(renderer: &mut crate::renderer::HydrolysisRenderer) {
    use parley::fontique::{Blob, CollectionOptions};
    use std::sync::Arc;

    const FONTS: &[(&str, &[u8])] = &[
        (
            "Roboto-Regular.ttf",
            include_bytes!("../../../../testing/fonts/Roboto-Regular.ttf"),
        ),
        (
            "Roboto-Medium.ttf",
            include_bytes!("../../../../testing/fonts/Roboto-Medium.ttf"),
        ),
        (
            "Roboto-Bold.ttf",
            include_bytes!("../../../../testing/fonts/Roboto-Bold.ttf"),
        ),
        (
            "Roboto-Italic.ttf",
            include_bytes!("../../../../testing/fonts/Roboto-Italic.ttf"),
        ),
    ];

    let font_cx = renderer.state_mut().text_fonts_mut();
    font_cx.collection = Collection::new(CollectionOptions {
        system_fonts: false,
        ..CollectionOptions::default()
    });
    let mut resource_fonts = ResourceFontFamilies::default();
    for (name, bytes) in FONTS {
        let families = font_cx
            .collection
            .register_fonts(Blob::new(Arc::new(*bytes)), None);
        resource_fonts.classify(name, &families);
    }
    resource_fonts.install(&mut font_cx.collection);
}

/// Register every `.ttf`/`.otf` under the app's `resources/fonts` directories
/// and install the recognized script fallbacks.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn load_native_resource_fonts(renderer: &mut crate::renderer::HydrolysisRenderer) {
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

    let font_cx = renderer.state_mut().text_fonts_mut();
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
}

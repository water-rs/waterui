//! Android font discovery.
//!
//! `fontique` has no Android system-font backend, so the host scans the
//! platform font directories itself and registers each face as a blob —
//! the same `ResourceFontFamilies` classify/install path the native
//! runner's bundled-resource fonts take, so a cluster a system face has
//! no glyph for still answers through the script fallbacks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parley::fontique::{Blob, Collection, CollectionOptions};

use crate::runner::fonts::ResourceFontFamilies;

/// The font directories AOSP documents and OEMs ship — `/system/fonts` is
/// always present; the rest are partition overlays that may not exist.
const FONT_DIRS: &[&str] = &[
    "/system/fonts",
    "/product/fonts",
    "/system_ext/fonts",
    "/vendor/fonts",
];

fn is_font_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("ttf" | "otf" | "ttc" | "otc")
    )
}

/// One directory of font files; a `read_dir` failure is a missing
/// partition, not an error — OEMs drop several of these.
fn scan_dir(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, out);
        } else if is_font_file(&path) {
            out.push(path);
        }
    }
}

/// The collection an Android session shapes with: every face under the
/// platform font directories, classified into the generic/script
/// fallbacks. `system_fonts` stays off — `fontique` knows no Android
/// source and would only double-register the same faces.
pub(crate) fn android_fonts() -> parley::FontContext {
    let mut font_cx = parley::FontContext {
        collection: Collection::new(CollectionOptions {
            system_fonts: false,
            ..CollectionOptions::default()
        }),
        source_cache: parley::fontique::SourceCache::default(),
    };
    let mut files = Vec::new();
    for dir in FONT_DIRS {
        scan_dir(Path::new(dir), &mut files);
    }
    let mut resource_fonts = ResourceFontFamilies::default();
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else {
            tracing::warn!(
                target: "waterui::hydrolysis::android",
                path = %path.display(),
                "font file could not be read — skipped"
            );
            continue;
        };
        let families = font_cx
            .collection
            .register_fonts(Blob::new(Arc::new(bytes)), None);
        // Classification keys on the file stem: `NotoSansCJKtc-Regular.ttc`
        // keys the traditional-Han fallback the same way the resource-font
        // names do on the native path.
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            resource_fonts.classify(stem, &families);
        }
    }
    resource_fonts.install(&mut font_cx.collection);
    font_cx
}

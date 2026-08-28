//! Build-time font subsetting for flash-bundled faces.
//!
//! A whole TTF embeds hundreds of kilobytes of glyphs a firmware image will
//! never draw — a full Latin face is ~770 KB, of which an ASCII UI uses a few
//! dozen kilobytes. `[backends.esp32] font_ranges` opts a project into
//! subsetting: every configured font is reduced to the requested Unicode
//! ranges before it is embedded, the way LVGL and Slint prepare their
//! offline fonts, while the runtime keeps consuming ordinary TTF bytes.
//!
//! Subsetting is explicit rather than a default because it trades away
//! glyphs silently: a codepoint outside the configured ranges simply does
//! not render on the device. The ranges are part of the subset file's name,
//! so changing them regenerates the harness instead of reusing stale bytes.
//!
//! The subsetter is `fontcull-klippa`, a Rust port of `HarfBuzz`'s
//! `hb-subset`: it computes the composite-glyph and GSUB closures and keeps
//! a working `cmap` for the retained codepoints, which is exactly what
//! dew's parley/fontique/skrifa stack needs at runtime. Layout scripts,
//! features, and name records are all retained so kerning and ligatures
//! inside the kept ranges behave identically to the full font.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr, eyre};
use fontcull_klippa::{Plan, SubsetFlags, parse_unicodes, subset_font};
use fontcull_write_fonts::read::FontRef;
use fontcull_write_fonts::read::collections::IntSet;
use fontcull_write_fonts::types::{GlyphId, NameId, Tag};

/// Subsets `source` to `ranges`, writing the result under `output_dir`.
///
/// Returns the subset file's path. The output name carries a hash of the
/// ranges, so a changed configuration produces a new file (and therefore a
/// harness regeneration) instead of silently reusing the old subset. An
/// up-to-date output — newer than the source font — is reused as is.
///
/// # Errors
///
/// Returns an error when the source font cannot be read or parsed, when the
/// ranges are invalid, or when the subset cannot be written.
pub fn subset_into(source: &Path, ranges: &str, output_dir: &Path) -> eyre::Result<PathBuf> {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| eyre!("font path {} has no UTF-8 file stem", source.display()))?;
    let output = output_dir.join(format!("{stem}-{:08x}.subset.ttf", ranges_key(ranges)));

    if is_fresh(source, &output) {
        return Ok(output);
    }

    let data = std::fs::read(source)
        .wrap_err_with(|| format!("failed to read font {}", source.display()))?;
    let subset = subset_bytes(&data, ranges)
        .wrap_err_with(|| format!("failed to subset font {}", source.display()))?;

    std::fs::create_dir_all(output_dir)
        .wrap_err_with(|| format!("failed to create {}", output_dir.display()))?;
    std::fs::write(&output, &subset)
        .wrap_err_with(|| format!("failed to write {}", output.display()))?;
    tracing::info!(
        "subset {} ({} KiB) to {} ({} KiB) for ranges {ranges}",
        source.display(),
        data.len() / 1024,
        output.display(),
        subset.len() / 1024,
    );
    Ok(output)
}

/// Reduces raw font bytes to the glyphs reachable from `ranges`.
///
/// # Errors
///
/// Returns an error when the font or the ranges fail to parse, or when the
/// subsetter rejects the font.
pub fn subset_bytes(data: &[u8], ranges: &str) -> eyre::Result<Vec<u8>> {
    let font = FontRef::new(data).map_err(|error| eyre!("font does not parse: {error}"))?;
    let unicodes = parse_unicodes(ranges)
        .map_err(|error| eyre!("invalid font_ranges {ranges:?}: {error:?}"))?;
    if unicodes.is_empty() {
        return Err(eyre!("font_ranges {ranges:?} selects no codepoints"));
    }
    // Everything except the glyph set is retained: all layout scripts and
    // features (kerning/ligatures within the kept ranges stay intact), all
    // name records, no extra table drops. The savings come from the glyph
    // outlines, which dominate the file.
    let plan = Plan::new(
        &IntSet::<GlyphId>::empty(),
        &unicodes,
        &font,
        SubsetFlags::default(),
        &IntSet::<Tag>::empty(),
        &IntSet::<Tag>::all(),
        &IntSet::<Tag>::all(),
        &IntSet::<NameId>::all(),
        &IntSet::<u16>::all(),
    );
    subset_font(&font, &plan).map_err(|error| eyre!("subsetting failed: {error:?}"))
}

/// Whether `output` exists and is at least as new as `source`.
fn is_fresh(source: &Path, output: &Path) -> bool {
    let (Ok(source_meta), Ok(output_meta)) = (source.metadata(), output.metadata()) else {
        return false;
    };
    match (source_meta.modified(), output_meta.modified()) {
        (Ok(source_time), Ok(output_time)) => output_time >= source_time,
        _ => false,
    }
}

/// A stable key for the ranges string, used in the subset file name.
fn ranges_key(ranges: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    ranges.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The font these tests subset.
    ///
    /// It is the one this repository ships for testing rather than whatever
    /// the host has installed: a list of absolute paths only ever covers the
    /// platforms someone remembered, and Windows was not one of them, so both
    /// tests here failed on it outright (part of #152). A committed font also
    /// makes the size assertion below mean something — it compares against a
    /// known font rather than whichever one the machine happened to offer.
    const TEST_FONT: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../testing/fonts/Roboto-Regular.ttf"
    ));

    fn host_font() -> Vec<u8> {
        TEST_FONT.to_vec()
    }

    /// The subset must shrink dramatically while keeping a working cmap for
    /// the retained range — the property dew's text stack depends on.
    #[test]
    fn ascii_subset_shrinks_and_keeps_cmap() {
        use fontcull_skrifa::MetadataProvider as _;

        let full = host_font();
        let subset = subset_bytes(&full, "20-7E").expect("ASCII subset must succeed");
        assert!(
            subset.len() * 4 < full.len(),
            "an ASCII subset should be under a quarter of the full font \
             ({} vs {} bytes)",
            subset.len(),
            full.len()
        );

        let font = fontcull_skrifa::FontRef::new(&subset).expect("subset must parse as a font");
        let charmap = font.charmap();
        for ch in ['A', 'z', '0', ' ', '~'] {
            let glyph = charmap
                .map(ch)
                .unwrap_or_else(|| panic!("subset cmap must map {ch:?}"));
            assert_ne!(glyph.to_u32(), 0, "{ch:?} must not map to .notdef");
        }
        assert!(
            charmap.map('中').is_none() || charmap.map('中').unwrap().to_u32() == 0,
            "codepoints outside the ranges must not survive"
        );
    }

    #[test]
    fn empty_ranges_fail_fast() {
        let full = host_font();
        assert!(subset_bytes(&full, "").is_err());
    }
}

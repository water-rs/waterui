//! Glyph runs through Core Text: each glyph's outline from the run's font
//! at the run's variation, placed at the glyph's origin.
//!
//! A run carries normalized variation coordinates (`F2Dot14`, after the
//! font's `avar` mapping, as the engine applies them); Core Text takes
//! user-space axis values and applies `avar` itself. The coordinates are
//! taken back through the font's own `avar` segment maps and `fvar` axis
//! ranges, so Core Text lands on the same instance.
//!
//! # Safety
//!
//! The `unsafe` calls hand Core Text a font descriptor, attribute
//! dictionaries and transforms that outlive each call, and read path
//! elements Core Graphics hands a block for the duration of the call.

use core::cell::RefCell;
use core::ptr::{self, NonNull};
use std::rc::Rc;

use block2::RcBlock;
use cocoa_ui::objc2_core_foundation::{
    CFDictionary, CFNumber, CFRetained, CFString, CGAffineTransform, CGPoint,
};
use cocoa_ui::objc2_core_graphics::{CGPath, CGPathElement, CGPathElementType};
use cocoa_ui::objc2_core_text::{CTFont, CTFontTableOptions, kCTFontVariationAttribute};
use kurbo::{Affine, BezPath, Point};
use skrifa::raw::tables::avar::{Avar, SegmentMaps};
use skrifa::raw::tables::fvar::Fvar;
use skrifa::raw::types::{F2Dot14, Fixed, Tag};
use skrifa::raw::{FontData, FontRead};
use waterui_graphics::draw::GlyphRun;

use super::resources::Registry;

/// Bisection steps that take an `avar` coordinate back to below `Fixed`'s
/// resolution: the range `-1..=1` halves to `2^-31`.
const INVERSE_STEPS: u32 = 32;

fn table(font: &CTFont, tag: [u8; 4]) -> Option<Vec<u8>> {
    // SAFETY: the font is live; a table tag is any four bytes.
    unsafe { font.table(u32::from_be_bytes(tag), CTFontTableOptions::NoOptions) }
        .map(|data| data.to_vec())
}

/// The input of `map` — the font's `avar` segment map for one axis —
/// that maps to `normalized`. The map is non-decreasing, so bisection over
/// read-fonts' own forward mapping finds it exactly as Core Text will map
/// it forward again.
fn unmap(map: &SegmentMaps<'_>, normalized: f64) -> f64 {
    let target = Fixed::from_f64(normalized);
    let (mut low, mut high) = (-1.0_f64, 1.0_f64);
    for _ in 0..INVERSE_STEPS {
        let middle = f64::midpoint(low, high);
        if map.apply(Fixed::from_f64(middle)) < target {
            low = middle;
        } else {
            high = middle;
        }
    }
    f64::midpoint(low, high)
}

/// The user-space value of each axis `coords` sets, keyed by axis tag.
///
/// # Panics
///
/// When the font has no `fvar` but the run carries coordinates, has fewer
/// axes than coordinates, or maps its coordinates with an `avar` version 2
/// item variation store, which depends on every axis at once and has no
/// per-axis inverse to reach Core Text's user space.
fn user_values(font: &CTFont, coords: &[i16]) -> Vec<(Tag, f64)> {
    let fvar_bytes =
        table(font, *b"fvar").expect("a run with variation coordinates names a variable font");
    let fvar = Fvar::read(FontData::new(&fvar_bytes)).expect("the font's fvar parses");
    let axes = fvar.axes().expect("the font's fvar lists its axes");
    assert!(
        coords.len() <= axes.len(),
        "a glyph run has {} variation coordinates for a font with {} axes",
        coords.len(),
        axes.len()
    );
    let avar_bytes = table(font, *b"avar");
    let avar = avar_bytes
        .as_deref()
        .map(|bytes| Avar::read(FontData::new(bytes)).expect("the font's avar parses"));
    if let Some(avar) = &avar {
        assert!(
            avar.var_store().is_none(),
            "the font maps variation coordinates with an avar version 2 variation store, which has no per-axis inverse for Core Text's user-space values"
        );
    }
    let maps: Vec<SegmentMaps<'_>> = avar.as_ref().map_or_else(Vec::new, |avar| {
        avar.axis_segment_maps()
            .iter()
            .map(|map| map.expect("the font's avar segment maps parse"))
            .collect()
    });
    coords
        .iter()
        .zip(axes)
        .enumerate()
        .map(|(index, (&coord, axis))| {
            let normalized = F2Dot14::from_bits(coord).to_f64();
            let unmapped = maps
                .get(index)
                .map_or(normalized, |map| unmap(map, normalized));
            let (min, default, max) = (
                axis.min_value().to_f64(),
                axis.default_value().to_f64(),
                axis.max_value().to_f64(),
            );
            let user = if unmapped < 0.0 {
                unmapped.mul_add(default - min, default)
            } else {
                unmapped.mul_add(max - default, default)
            };
            (axis.axis_tag(), user)
        })
        .collect()
}

/// The run's font: its registered face at its size and variation.
///
/// # Panics
///
/// When the run names a font that is not registered, or carries
/// coordinates [`user_values`] cannot take back to user space.
#[must_use]
pub fn font(registry: &Registry, run: &GlyphRun) -> CFRetained<CTFont> {
    let descriptor = registry.font(run.font);
    let size = f64::from(run.size);
    // SAFETY: the descriptor is live and a null matrix is the identity.
    let face = unsafe { CTFont::with_font_descriptor(&descriptor, size, ptr::null()) };
    if run.coords.is_empty() {
        return face;
    }
    let values = user_values(&face, &run.coords);
    let keys: Vec<CFRetained<CFNumber>> = values
        .iter()
        .map(|(tag, _)| CFNumber::new_i64(i64::from(u32::from_be_bytes(tag.to_be_bytes()))))
        .collect();
    let numbers: Vec<CFRetained<CFNumber>> = values
        .iter()
        .map(|&(_, value)| CFNumber::new_f64(value))
        .collect();
    let keys: Vec<&CFNumber> = keys.iter().map(|key| &**key).collect();
    let numbers: Vec<&CFNumber> = numbers.iter().map(|value| &**value).collect();
    let variation = CFDictionary::from_slices(&keys, &numbers);
    // SAFETY: the static is a `CFString` constant Core Text exports.
    let key: &CFString = unsafe { kCTFontVariationAttribute };
    let attributes = CFDictionary::from_slices(&[key], &[variation.as_opaque()]);
    // SAFETY: the descriptor is live and `kCTFontVariationAttribute` takes
    // a dictionary of axis identifiers to values.
    let varied = unsafe { descriptor.copy_with_attributes(attributes.as_opaque()) };
    // SAFETY: the descriptor is live and a null matrix is the identity.
    unsafe { CTFont::with_font_descriptor(&varied, size, ptr::null()) }
}

/// Appends `path`'s elements, moved by `placement`, to `out`.
fn append(out: &mut BezPath, path: &CGPath, placement: Affine) {
    let collected = Rc::new(RefCell::new(BezPath::new()));
    let at = move |point: CGPoint| placement * Point::new(point.x, point.y);
    let sink = Rc::clone(&collected);
    let block = RcBlock::new(move |element: NonNull<CGPathElement>| {
        // SAFETY: Core Graphics hands a live element for the call, whose
        // `points` holds as many points as its type uses.
        let element = unsafe { element.as_ref() };
        let point = |index: usize| {
            // SAFETY: `index` is below the point count of the element's
            // type, which `points` holds.
            at(unsafe { *element.points.as_ptr().add(index) })
        };
        let mut collected = sink.borrow_mut();
        match element.r#type {
            CGPathElementType::MoveToPoint => collected.move_to(point(0)),
            CGPathElementType::AddLineToPoint => collected.line_to(point(0)),
            CGPathElementType::AddQuadCurveToPoint => collected.quad_to(point(0), point(1)),
            CGPathElementType::AddCurveToPoint => {
                collected.curve_to(point(0), point(1), point(2));
            }
            CGPathElementType::CloseSubpath => collected.close_path(),
            other => unreachable!("Core Graphics has no path element {other:?}"),
        }
    });
    // SAFETY: the path is live and the block lives across the call.
    unsafe { path.apply_with_block(RcBlock::as_ptr(&block)) };
    out.extend(collected.take());
}

/// The outline of every glyph of `run`, in the canvas's y-down space.
///
/// # Panics
///
/// As [`font`] does.
#[must_use]
pub fn outline(registry: &Registry, run: &GlyphRun) -> BezPath {
    let font = font(registry, run);
    let mut combined = BezPath::new();
    for glyph in run.glyphs.iter() {
        let id = u16::try_from(glyph.id).expect("a Core Text glyph index fits sixteen bits");
        // SAFETY: the font is live and a null matrix is the identity.
        let Some(outline) = (unsafe { font.path_for_glyph(id, ptr::null::<CGAffineTransform>()) })
        else {
            continue;
        };
        // Outlines are y-up; the canvas is y-down.
        let placement = Affine::translate((f64::from(glyph.x), f64::from(glyph.y)))
            * glyph.transform.unwrap_or(Affine::IDENTITY)
            * Affine::FLIP_Y;
        append(&mut combined, &outline, placement);
    }
    combined
}

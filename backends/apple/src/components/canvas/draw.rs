//! Paints drawn by Core Graphics: gradients and image patterns, into the
//! layer a shape masks.
//!
//! The working space interpolates in linear light, so a gradient is a
//! `CGGradient` in extended linear Display P3 and needs no subdivided
//! stops; a gradient interpolating in encoded sRGB converts its stops into
//! extended sRGB instead.
//!
//! # Safety
//!
//! The `unsafe` calls hand Core Graphics component and location arrays
//! that live across the call, in the counts the gradient was sized for.

use core::f64::consts::TAU;

use cocoa_ui::objc2_core_foundation::{CFData, CFRetained};
use cocoa_ui::objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGGradient,
    CGGradientDrawingOptions, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
    CGImageComponentInfo, CGInterpolationQuality, kCGColorSpaceExtendedSRGB,
};
use cocoa_ui::patch;
use kurbo::{Affine, Point, Rect};
use waterui_graphics::backend::draw::{
    ColorSpace, ColorStop, Extend, ImagePattern, Interpolation, LinearDisplayP3, LinearGradient,
    MeshColorInterpolation, MeshGradient, Paint, RadialGradient, Sampling, Srgb, SweepGradient,
};

use super::cg;
use super::plan;
use super::resources::Registry;

/// The most periods a repeating gradient expands into stops for: a
/// gradient that needs more is thinner than a pixel.
const MAX_PERIODS: f64 = 1024.0;

/// The most tiles an image pattern draws.
const MAX_TILES: f64 = 65_536.0;

/// The whole periods `first..last` a repeating paint spans, held as the
/// integral `f64`s every use of them (offsets, scales, parity) needs.
#[derive(Clone, Copy)]
struct Periods {
    first: f64,
    last: f64,
}

impl Periods {
    const fn new(first: f64, last: f64) -> Self {
        Self { first, last }
    }

    /// The periods covering the parameter range `low..=high`, at least one.
    fn covering(low: f64, high: f64) -> Self {
        let first = low.floor();
        Self::new(first, high.ceil().max(first + 1.0))
    }

    fn count(self) -> f64 {
        self.last - self.first
    }

    /// `self`, after checking it is a count the lowering expands. A larger
    /// count is a contract violation of the paint's author; the expansion
    /// stops at the most periods the lowering takes.
    fn limited(self) -> Self {
        if self.count() > MAX_PERIODS {
            waterui::contract_violation!(
                "a repeating gradient needs {} periods across its extent; the lowering expands at most {MAX_PERIODS}, \
                 and the rest is not drawn",
                self.count()
            );
            return Self::new(self.first, self.first + MAX_PERIODS);
        }
        self
    }

    fn iter(self) -> impl Iterator<Item = f64> {
        core::iter::successors(Some(self.first), move |period| {
            Some(period + 1.0).filter(|next| *next < self.last)
        })
    }
}

/// Whether `period` runs backwards in a reflecting paint.
fn is_mirrored(period: f64) -> bool {
    period.rem_euclid(2.0) >= 0.5
}

/// A stop's position and straight-alpha components in the gradient's space.
type Stop = (f64, [f64; 4]);

fn components(stop: &ColorStop, interpolation: Interpolation) -> [f64; 4] {
    let [red, green, blue, alpha] = stop.color.components;
    let [red, green, blue] = match interpolation {
        Interpolation::Working => [red, green, blue],
        Interpolation::SrgbEncoded => {
            <LinearDisplayP3 as ColorSpace>::convert::<Srgb>([red, green, blue])
        }
    };
    [red, green, blue, alpha].map(f64::from)
}

fn stops(stops: &[ColorStop], interpolation: Interpolation) -> Vec<Stop> {
    if stops.is_empty() {
        waterui::contract_violation!(
            "a gradient has at least one stop; a gradient with none paints transparent"
        );
        return vec![(0.0, [0.0; 4])];
    }
    stops
        .iter()
        .map(|stop| (f64::from(stop.offset), components(stop, interpolation)))
        .collect()
}

/// `stops` repeated over `periods`, rescaled into `0..=1`; a reflecting
/// period runs backwards.
fn expand(stops: &[Stop], periods: Periods, reflect: bool) -> Vec<Stop> {
    let count = periods.count();
    let mut expanded = Vec::new();
    for period in periods.iter() {
        let mirrored = reflect && is_mirrored(period);
        let base = period - periods.first;
        if mirrored {
            expanded.extend(
                stops
                    .iter()
                    .rev()
                    .map(|&(offset, color)| ((base + 1.0 - offset) / count, color)),
            );
        } else {
            expanded.extend(
                stops
                    .iter()
                    .map(|&(offset, color)| ((base + offset) / count, color)),
            );
        }
    }
    expanded
}

fn gradient(stops: &[Stop], interpolation: Interpolation) -> CFRetained<CGGradient> {
    let space = match interpolation {
        Interpolation::Working => cg::working_space(),
        Interpolation::SrgbEncoded => {
            // SAFETY: the static is a `CFString` constant Core Graphics exports.
            CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedSRGB }))
                .expect("Core Graphics provides extended sRGB")
        }
    };
    let flat: Vec<f64> = stops.iter().flat_map(|(_, color)| *color).collect();
    let locations: Vec<f64> = stops
        .iter()
        .map(|(offset, _)| offset.clamp(0.0, 1.0))
        .collect();
    // SAFETY: four components and one location per stop, both alive for the
    // call.
    unsafe {
        CGGradient::with_color_components(
            Some(&space),
            flat.as_ptr(),
            locations.as_ptr(),
            locations.len(),
        )
    }
    .expect("Core Graphics makes a gradient from validated stops")
}

const fn options(extend: Extend) -> CGGradientDrawingOptions {
    match extend {
        Extend::Pad => CGGradientDrawingOptions(
            CGGradientDrawingOptions::DrawsBeforeStartLocation.bits()
                | CGGradientDrawingOptions::DrawsAfterEndLocation.bits(),
        ),
        // Repeating gradients are expanded into stops across the extent.
        Extend::None | Extend::Repeat | Extend::Reflect => CGGradientDrawingOptions(0),
    }
}

const fn corners(extent: Rect) -> [Point; 4] {
    [
        Point::new(extent.x0, extent.y0),
        Point::new(extent.x1, extent.y0),
        Point::new(extent.x0, extent.y1),
        Point::new(extent.x1, extent.y1),
    ]
}

fn linear(context: &CGContext, gradient_paint: &LinearGradient, extent: Rect) {
    let base = stops(&gradient_paint.stops, gradient_paint.interpolation);
    let (start, end) = (gradient_paint.start, gradient_paint.end);
    let axis = end - start;
    match gradient_paint.extend {
        Extend::Pad | Extend::None => {
            CGContext::draw_linear_gradient(
                Some(context),
                Some(&gradient(&base, gradient_paint.interpolation)),
                cg::point(start),
                cg::point(end),
                options(gradient_paint.extend),
            );
        }
        repeat @ (Extend::Repeat | Extend::Reflect) => {
            let length = axis.hypot2();
            let at = |p: Point| (p - start).dot(axis) / length;
            let (low, high) = corners(extent)
                .map(at)
                .into_iter()
                .fold((f64::MAX, f64::MIN), |(low, high), t| {
                    (low.min(t), high.max(t))
                });
            let periods = Periods::covering(low, high).limited();
            let expanded = expand(&base, periods, repeat == Extend::Reflect);
            CGContext::draw_linear_gradient(
                Some(context),
                Some(&gradient(&expanded, gradient_paint.interpolation)),
                cg::point(start + axis * periods.first),
                cg::point(start + axis * periods.last),
                options(Extend::None),
            );
        }
    }
}

fn radial(context: &CGContext, paint: &RadialGradient, extent: Rect) {
    let base = stops(&paint.stops, paint.interpolation);
    if paint.has_identical_circles() {
        // A hard edge: the first colour inside the circle, the last outside.
        let (first, last) = (base[0].1, base[base.len() - 1].1);
        let hard = [(0.0, first), (0.5, first), (0.5, last), (1.0, last)];
        CGContext::draw_radial_gradient(
            Some(context),
            Some(&gradient(&hard, paint.interpolation)),
            cg::point(paint.start_center),
            0.0,
            cg::point(paint.start_center),
            2.0 * paint.start_radius,
            options(paint.extend),
        );
        return;
    }
    match paint.extend {
        Extend::Pad | Extend::None => {
            CGContext::draw_radial_gradient(
                Some(context),
                Some(&gradient(&base, paint.interpolation)),
                cg::point(paint.start_center),
                paint.start_radius,
                cg::point(paint.end_center),
                paint.end_radius,
                options(paint.extend),
            );
        }
        repeat @ (Extend::Repeat | Extend::Reflect) => {
            let delta = paint.end_radius - paint.start_radius;
            if delta <= 0.0 {
                waterui::contract_violation!(
                    "a repeating radial gradient grows outward; got radii {} to {}; \
                     the gradient is not drawn",
                    paint.start_radius,
                    paint.end_radius
                );
                return;
            }
            let shift = paint.end_center - paint.start_center;
            let circle = |t: f64| {
                (
                    paint.start_center + shift * t,
                    delta.mul_add(t, paint.start_radius),
                )
            };
            let first = -(paint.start_radius / delta).floor();
            let mut periods = Periods::new(first, first + 1.0);
            while !corners(extent).into_iter().all(|corner| {
                let (center, radius) = circle(periods.last);
                (corner - center).hypot() <= radius
            }) {
                periods.last += 1.0;
                if periods.count() > MAX_PERIODS {
                    periods = periods.limited();
                    break;
                }
            }
            let expanded = expand(&base, periods, repeat == Extend::Reflect);
            let (start_center, start_radius) = circle(periods.first);
            let (end_center, end_radius) = circle(periods.last);
            CGContext::draw_radial_gradient(
                Some(context),
                Some(&gradient(&expanded, paint.interpolation)),
                cg::point(start_center),
                start_radius,
                cg::point(end_center),
                end_radius,
                options(Extend::None),
            );
        }
    }
}

/// The colour at `offset` along `stops`, which ascend.
fn color_at(stops: &[Stop], offset: f64) -> [f64; 4] {
    let after = stops.partition_point(|&(at, _)| at <= offset);
    match (after.checked_sub(1).map(|i| stops[i]), stops.get(after)) {
        (Some((from, a)), Some(&(to, b))) if to > from => {
            let t = (offset - from) / (to - from);
            core::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
        }
        (Some((_, color)), _) | (None, Some(&(_, color))) => color,
        (None, None) => unreachable!("a gradient has at least one stop"),
    }
}

fn sweep(context: &CGContext, paint: &SweepGradient) {
    let span = paint.end_angle - paint.start_angle;
    if !(span > 0.0 && span <= f64::from(core::f32::consts::TAU)) {
        waterui::contract_violation!(
            "an angular gradient sweeps 0 < span <= TAU, got {span}; the gradient is not drawn"
        );
        return;
    }
    let turn = (span / TAU).min(1.0);
    let base = stops(&paint.stops, paint.interpolation);
    let scaled: Vec<Stop> = match paint.extend {
        // The parameter past the last stop pads, as the engine's does.
        Extend::Pad | Extend::None => base
            .iter()
            .map(|&(offset, color)| (offset * turn, color))
            .collect(),
        repeat @ (Extend::Repeat | Extend::Reflect) => {
            let periods = (1.0 / turn).ceil();
            let mut expanded = Vec::new();
            for period in Periods::new(0.0, periods).iter() {
                let mirrored = repeat == Extend::Reflect && is_mirrored(period);
                let local: Vec<Stop> = if mirrored {
                    base.iter().rev().map(|&(o, c)| (1.0 - o, c)).collect()
                } else {
                    base.clone()
                };
                for (offset, color) in &local {
                    let at = (period + offset) * turn;
                    if at < 1.0 {
                        expanded.push((at, *color));
                    }
                }
            }
            // The last period is cut by the turn: close it at its colour.
            let cut = periods * turn;
            if cut > 1.0 {
                let local = (1.0 / turn) - (periods - 1.0);
                let mirrored = repeat == Extend::Reflect && is_mirrored(periods - 1.0);
                let offset = if mirrored { 1.0 - local } else { local };
                expanded.push((1.0, color_at(&base, offset)));
            }
            expanded
        }
    };
    context.draw_conic_gradient(
        Some(&gradient(&scaled, paint.interpolation)),
        cg::point(paint.center),
        paint.start_angle,
    );
}

/// Draws `image` into `rect`, upright in a y-down context.
pub fn draw_image_upright(context: &CGContext, image: &CGImage, rect: Rect, sampling: Sampling) {
    CGContext::save_g_state(Some(context));
    CGContext::set_interpolation_quality(
        Some(context),
        match sampling {
            Sampling::Nearest => CGInterpolationQuality::None,
            Sampling::Linear => CGInterpolationQuality::High,
        },
    );
    CGContext::translate_ctm(Some(context), rect.x0, rect.y1);
    CGContext::scale_ctm(Some(context), 1.0, -1.0);
    CGContext::draw_image(
        Some(context),
        cg::rect(Rect::new(0.0, 0.0, rect.width(), rect.height())),
        Some(image),
    );
    CGContext::restore_g_state(Some(context));
}

/// One stretch of an image pattern along one axis: the texels it reads
/// and where they land, in texels of the pattern's space.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    /// First texel read.
    from: f64,
    /// Texels read.
    texels: f64,
    /// Where the read texels start.
    at: f64,
    /// How far they are stretched.
    length: f64,
    /// Whether they run backwards.
    mirrored: bool,
}

/// The spans covering `low..high` of an axis `size` texels long under
/// `extend`. A padded axis stretches its first texel over everything
/// before the image and its last over everything after: clamp to edge.
fn spans(extend: Extend, low: f64, high: f64, size: f64) -> Vec<Span> {
    let whole = |at: f64, mirrored: bool| Span {
        from: 0.0,
        texels: size,
        at,
        length: size,
        mirrored,
    };
    match extend {
        Extend::None => vec![whole(0.0, false)],
        repeat @ (Extend::Repeat | Extend::Reflect) => Periods::covering(low / size, high / size)
            .limited()
            .iter()
            .map(|period| {
                whole(
                    period * size,
                    repeat == Extend::Reflect && is_mirrored(period),
                )
            })
            .collect(),
        Extend::Pad => {
            let before = (low < 0.0).then_some(Span {
                from: 0.0,
                texels: 1.0,
                at: low,
                length: -low,
                mirrored: false,
            });
            let after = (high > size).then_some(Span {
                from: size - 1.0,
                texels: 1.0,
                at: size,
                length: high - size,
                mirrored: false,
            });
            before
                .into_iter()
                .chain([whole(0.0, false)])
                .chain(after)
                .collect()
        }
    }
}

fn pattern(context: &CGContext, pattern: &ImagePattern, extent: Rect, registry: &Registry) {
    let image = registry.image(pattern.image);
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel dimensions are far below 2^52"
    )]
    let (width, height) = (
        CGImage::width(Some(&image)) as f64,
        CGImage::height(Some(&image)) as f64,
    );
    let inverse = pattern.transform.inverse();
    let bounds = corners(extent)
        .map(|corner| inverse * corner)
        .into_iter()
        .fold(
            Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |bounds, p| {
                Rect::new(
                    bounds.x0.min(p.x),
                    bounds.y0.min(p.y),
                    bounds.x1.max(p.x),
                    bounds.y1.max(p.y),
                )
            },
        );
    let columns = spans(pattern.extend_x, bounds.x0, bounds.x1, width);
    let rows = spans(pattern.extend_y, bounds.y0, bounds.y1, height);
    #[expect(
        clippy::cast_precision_loss,
        reason = "tile counts are bounded by MAX_PERIODS squared"
    )]
    let tiles = (columns.len() * rows.len()) as f64;
    if tiles > MAX_TILES {
        waterui::contract_violation!(
            "an image pattern tiles at most {MAX_TILES} times across its extent, got {tiles}; \
             the pattern is not drawn"
        );
        return;
    }
    CGContext::save_g_state(Some(context));
    CGContext::concat_ctm(Some(context), cg::transform(pattern.transform));
    for row in &rows {
        for column in &columns {
            let source = if column.texels < width || row.texels < height {
                CGImage::with_image_in_rect(
                    Some(&image),
                    cg::rect(Rect::new(
                        column.from,
                        row.from,
                        column.from + column.texels,
                        row.from + row.texels,
                    )),
                )
                .expect("a texel span lies inside its image")
            } else {
                image.clone()
            };
            let cell = Rect::new(
                column.at,
                row.at,
                column.at + column.length,
                row.at + row.length,
            );
            let mirror = Affine::translate(cell.center().to_vec2())
                * Affine::scale_non_uniform(
                    if column.mirrored { -1.0 } else { 1.0 },
                    if row.mirrored { -1.0 } else { 1.0 },
                )
                * Affine::translate(-cell.center().to_vec2());
            CGContext::save_g_state(Some(context));
            CGContext::concat_ctm(Some(context), cg::transform(mirror));
            draw_image_upright(context, &source, cell, pattern.sampling);
            CGContext::restore_g_state(Some(context));
        }
    }
    CGContext::restore_g_state(Some(context));
}

/// Draws `mesh`, placed by `transform` into the context's space, over the
/// whole of the context's clip.
///
/// The mesh is rasterized on the CPU, a pixel of the context's resolution
/// per pixel, by the pure inversion the Metal pass solves
/// ([`cocoa_ui::patch`]); a run Core Graphics draws cannot take the pass's
/// GPU pixels without reading them back.
pub fn mesh(context: &CGContext, mesh: &MeshGradient, transform: Affine) {
    let visible = CGContext::clip_bounding_box(Some(context));
    let ctm = CGContext::ctm(Some(context));
    let scale = Affine::new([ctm.a, ctm.b, ctm.c, ctm.d, 0.0, 0.0])
        .determinant()
        .abs()
        .sqrt();
    if scale == 0.0 || visible.is_empty() {
        return;
    }
    // The pixel grid of the context's resolution, so that the image lands
    // one to one when the context only scales and translates.
    let (x0, y0) = (
        (visible.origin.x * scale).floor() / scale,
        (visible.origin.y * scale).floor() / scale,
    );
    let (x1, y1) = (
        ((visible.origin.x + visible.size.width) * scale).ceil() / scale,
        ((visible.origin.y + visible.size.height) * scale).ceil() / scale,
    );
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a clip's pixel size is a small non-negative integer"
    )]
    let (width, height) = (
        ((x1 - x0) * scale).round() as usize,
        ((y1 - y0) * scale).round() as usize,
    );
    if width == 0 || height == 0 {
        return;
    }
    let (points, colors) = plan::grid(mesh);
    let patches = patch::patches(
        mesh.columns() as usize,
        mesh.rows() as usize,
        &points,
        &colors,
    );
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the inverse is solved in f32, as the shader's is"
    )]
    let pixel_to_mesh =
        (transform.inverse() * Affine::translate((x0, y0)) * Affine::scale(scale.recip()))
            .as_coeffs()
            .map(|coefficient| coefficient as f32);
    let pixels = patch::raster(
        &patches,
        mesh.interpolation_mode() == MeshColorInterpolation::Smoothstep,
        width,
        height,
        pixel_to_mesh,
    );
    let bytes: Vec<u8> = pixels
        .iter()
        .flatten()
        .flat_map(|component| component.to_le_bytes())
        .collect();
    let data = CFData::from_bytes(&bytes);
    let provider = CGDataProvider::with_cf_data(Some(&data))
        .expect("Core Graphics takes the `CFData` made a line above");
    let space = cg::working_space();
    // SAFETY: a null decode array is the identity; every other argument
    // describes the 128-bit premultiplied float pixels `raster` made.
    let image = unsafe {
        CGImage::new(
            width,
            height,
            32,
            128,
            width * 16,
            Some(&space),
            CGBitmapInfo(
                CGImageAlphaInfo::PremultipliedLast.0
                    | CGImageComponentInfo::Float.0
                    | CGImageByteOrderInfo::Order32Little.0,
            ),
            Some(&provider),
            core::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    };
    let Some(image) = image else {
        tracing::error!(
            "Core Graphics could not make a {width}x{height} float image of a mesh; \
             the mesh is not drawn"
        );
        return;
    };
    draw_image_upright(context, &image, Rect::new(x0, y0, x1, y1), Sampling::Linear);
}

/// Draws `paint` across `extent` in canvas coordinates.
///
/// A shader paint, which only an engine surface draws, is reported and
/// draws nothing. A mesh is drawn by [`mesh`], never here.
///
/// # Panics
///
/// On a mesh paint, which `plan::classify` routes to its own layer.
pub fn paint(context: &CGContext, paint: &Paint, extent: Rect, registry: &Registry) {
    match paint {
        Paint::Solid(color) => {
            CGContext::set_fill_color_with_color(Some(context), Some(&cg::color(*color)));
            CGContext::fill_rect(Some(context), cg::rect(extent));
        }
        Paint::Linear(gradient) => linear(context, gradient, extent),
        Paint::Radial(gradient) => radial(context, gradient, extent),
        Paint::Sweep(gradient) => sweep(context, gradient),
        Paint::Image(image) => pattern(context, image, extent, registry),
        Paint::Transformed(transformed) => {
            CGContext::save_g_state(Some(context));
            CGContext::concat_ctm(Some(context), cg::transform(transformed.transform));
            // The extent seen from the paint's own coordinates.
            let inverse = transformed.transform.inverse();
            let inner = corners(extent)
                .map(|corner| inverse * corner)
                .into_iter()
                .fold(
                    Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN),
                    |bounds, p| {
                        Rect::new(
                            bounds.x0.min(p.x),
                            bounds.y0.min(p.y),
                            bounds.x1.max(p.x),
                            bounds.y1.max(p.y),
                        )
                    },
                );
            self::paint(context, &transformed.paint, inner, registry);
            CGContext::restore_g_state(Some(context));
        }
        Paint::Mesh(_) => unreachable!("`plan::classify` routes a mesh gradient to its own layer"),
        Paint::Shader(_) => {
            waterui::contract_violation!(
                "a shader paint needs an engine surface, which a canvas has none of; \
                 the paint is not drawn"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a repeating gradient needs 2000 periods")
    )]
    fn periods_beyond_the_limit_are_clamped() {
        let limited = Periods::new(0.0, 2000.0).limited();
        assert_eq!(limited.count(), MAX_PERIODS);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a gradient has at least one stop")
    )]
    fn a_gradient_without_stops_paints_transparent() {
        assert_eq!(stops(&[], Interpolation::Working), vec![(0.0, [0.0; 4])]);
    }
}

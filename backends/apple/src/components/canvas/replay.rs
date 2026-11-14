//! Runs of recorded commands drawn by Core Graphics.
//!
//! A group whose blend mode composites against the content beneath it in
//! the canvas cannot be a layer ([`super::plan`], "Group compositing"); the
//! lowering draws such a group and the content of its level beneath it
//! into one bitmap here, and a layer shows it ([`super::layers`]). Groups
//! blend with Core Graphics blend modes in the extended sRGB encoding the
//! bitmap holds, whatever their recorded `blend_space`: the encoded-space
//! deviation the canvas design states for Apple. Every other command draws
//! as its layer does: the same paths, paints, glyph outlines and spreads.
//!
//! # Safety
//!
//! The `unsafe` calls open transparency layers on a live context with no
//! auxiliary information.

use core::ops::Range;

use cocoa_ui::objc2_core_foundation::CGSize;
use cocoa_ui::objc2_core_graphics::{CGBlendMode, CGContext};
use kurbo::{Affine, BezPath, PathEl, Rect, Shape as _, Stroke, Vec2};
use waterui_graphics::backend::draw::{
    BlendMode, Command, FillRule, GlyphStyle, Paint, Shadow, ShapeData, WorkingColor,
};

use super::cg;
use super::draw;
use super::glyphs;
use super::plan::{self, Composite, PaintClass};
use super::resources::Registry;

/// How far a Gaussian shadow reaches, in standard deviations.
pub const SHADOW_REACH_SIGMAS: f64 = 3.0;

/// Core Graphics' shadow `blur` per standard deviation of the Gaussian it
/// draws. Measured on a straight edge: a `blur` of 4, 10 and 20 pixels
/// draws a standard deviation of 1.98, 4.77 and 9.43 pixels.
const BLUR_PER_SIGMA: f64 = 2.0;

/// The Core Graphics blend mode of `mode`; `None` for `Dst`, which draws
/// nothing of the source.
#[must_use]
pub const fn blend_mode(mode: BlendMode) -> Option<CGBlendMode> {
    Some(match mode {
        BlendMode::Normal => CGBlendMode::Normal,
        BlendMode::Multiply => CGBlendMode::Multiply,
        BlendMode::Screen => CGBlendMode::Screen,
        BlendMode::Overlay => CGBlendMode::Overlay,
        BlendMode::Darken => CGBlendMode::Darken,
        BlendMode::Lighten => CGBlendMode::Lighten,
        BlendMode::ColorDodge => CGBlendMode::ColorDodge,
        BlendMode::ColorBurn => CGBlendMode::ColorBurn,
        BlendMode::HardLight => CGBlendMode::HardLight,
        BlendMode::SoftLight => CGBlendMode::SoftLight,
        BlendMode::Difference => CGBlendMode::Difference,
        BlendMode::Exclusion => CGBlendMode::Exclusion,
        BlendMode::Hue => CGBlendMode::Hue,
        BlendMode::Saturation => CGBlendMode::Saturation,
        BlendMode::Color => CGBlendMode::Color,
        BlendMode::Luminosity => CGBlendMode::Luminosity,
        BlendMode::Clear => CGBlendMode::Clear,
        BlendMode::Src => CGBlendMode::Copy,
        BlendMode::Dst => return None,
        BlendMode::DestOver => CGBlendMode::DestinationOver,
        BlendMode::SrcIn => CGBlendMode::SourceIn,
        BlendMode::DestIn => CGBlendMode::DestinationIn,
        BlendMode::SrcOut => CGBlendMode::SourceOut,
        BlendMode::DestOut => CGBlendMode::DestinationOut,
        BlendMode::SrcAtop => CGBlendMode::SourceAtop,
        BlendMode::DestAtop => CGBlendMode::DestinationAtop,
        BlendMode::Xor => CGBlendMode::XOR,
        BlendMode::PlusLighter => CGBlendMode::PlusLighter,
    })
}

/// The indices of the commands directly inside `range` of `commands`.
fn items(commands: &[Command], range: Range<usize>) -> impl Iterator<Item = usize> + '_ {
    let end = range.end;
    core::iter::successors(
        Some(range.start).filter(|&start| start < end),
        move |&index| {
            Some(plan::scope_end(&commands[index]).map_or(index + 1, |close| close + 1))
                .filter(|&next| next < end)
        },
    )
}

/// The outline a fill of `elements` covers, and how it fills: a stroke
/// covers its outline under the nonzero rule.
fn covered(elements: Vec<PathEl>, rule: FillRule, stroke: Option<&Stroke>) -> (BezPath, FillRule) {
    match stroke {
        Some(stroke) => (cg::stroke_outline(elements, stroke), FillRule::NonZero),
        None => (BezPath::from_vec(elements), rule),
    }
}

/// The shapes a command covers with a paint, if it is a fill or stroke.
fn painted(command: &Command, registry: &Registry) -> Option<(BezPath, FillRule, Paint)> {
    let (elements, rule, stroke, paint) = match command {
        Command::Fill { shape, paint } => (cg::elements(shape), cg::fill_rule(shape), None, paint),
        Command::Stroke {
            shape,
            stroke,
            paint,
        } => (
            cg::elements(shape),
            cg::fill_rule(shape),
            Some(stroke),
            paint,
        ),
        Command::Glyphs { run, paint } => (
            glyphs::outline(registry, run).into_elements(),
            FillRule::NonZero,
            match &run.style {
                GlyphStyle::Fill => None,
                GlyphStyle::Stroke(stroke) => Some(stroke),
            },
            paint,
        ),
        _ => return None,
    };
    let (outline, rule) = covered(elements, rule, stroke);
    Some((outline, rule, paint.clone()))
}

/// The box a shadow of `shape` darkens, if any.
fn shadow_extent(shape: &ShapeData, shadow: &Shadow) -> Option<Rect> {
    let reach = shadow.sigma.mul_add(SHADOW_REACH_SIGMAS, 1.0);
    cg::bounds(&cg::spread(shape, shadow.spread))
        .map(|bounds| (bounds + shadow.offset).inflate(reach, reach))
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

/// The box, in the space `range` is recorded in, that its commands draw
/// into; `None` when they draw nothing.
#[must_use]
pub fn extent(commands: &[Command], range: Range<usize>, registry: &Registry) -> Option<Rect> {
    items(commands, range).fold(None, |bounds, index| {
        union(bounds, item_extent(commands, index, registry))
    })
}

fn item_extent(commands: &[Command], index: usize, registry: &Registry) -> Option<Rect> {
    let inside = |end: u32| extent(commands, index + 1..end as usize, registry);
    match &commands[index] {
        command @ (Command::Fill { .. } | Command::Stroke { .. } | Command::Glyphs { .. }) => {
            let (outline, _, _) = painted(command, registry).expect("a fill or stroke");
            (!outline.is_empty()).then(|| outline.bounding_box().inflate(1.0, 1.0))
        }
        Command::Image { dst, .. } => Some(*dst),
        Command::Shadow { shape, shadow } => shadow_extent(shape, shadow),
        Command::Picture { picture, transform } => {
            let inner = picture.display_list().commands();
            extent(inner, 0..inner.len(), registry).map(|r| transform.transform_rect_bbox(r))
        }
        Command::BeginClip { shape, end, .. } => inside(*end).and_then(|r| {
            let clipped = r.intersect(BezPath::from_vec(cg::elements(shape)).bounding_box());
            (clipped.width() > 0.0 && clipped.height() > 0.0).then_some(clipped)
        }),
        Command::BeginTransform { transform, end } => {
            inside(*end).map(|r| transform.transform_rect_bbox(r))
        }
        Command::BeginGroup { group, end } => match plan::composite(group.blend) {
            Composite::Hidden => None,
            Composite::Normal | Composite::Filter(_) | Composite::Backdrop => inside(*end),
        },
        Command::Text {
            layout,
            transform,
            paint,
        } => {
            let text = registry.text(*layout, paint.as_ref());
            let inner = text.display_list().commands();
            extent(inner, 0..inner.len(), registry).map(|r| transform.transform_rect_bbox(r))
        }
        Command::End => unreachable!("an item is never a scope's End"),
    }
}

/// What drawing reads besides the commands.
#[derive(Clone, Copy, Debug)]
pub struct Target<'a> {
    /// The context, its CTM mapping the recording's space to its pixels.
    pub context: &'a CGContext,
    /// The registered images and fonts.
    pub registry: &'a Registry,
    /// The bitmap's larger side in pixels: a shape drawn that far away
    /// lands outside it.
    pub reach: f64,
}

/// Draws `range` of `commands` into `target`.
pub fn draw(target: Target<'_>, commands: &[Command], range: Range<usize>) {
    for index in items(commands, range) {
        draw_item(target, commands, index);
    }
}

fn draw_item(target: Target<'_>, commands: &[Command], index: usize) {
    let context = Some(target.context);
    let inside = |end: u32| draw(target, commands, index + 1..end as usize);
    match &commands[index] {
        command @ (Command::Fill { .. } | Command::Stroke { .. } | Command::Glyphs { .. }) => {
            let (outline, rule, paint) =
                painted(command, target.registry).expect("a fill or stroke");
            cover(target, &outline, rule, &paint);
        }
        Command::Image {
            image,
            dst,
            sampling,
        } => draw::draw_image_upright(
            target.context,
            &target.registry.image(*image),
            *dst,
            *sampling,
        ),
        Command::Shadow { shape, shadow } => cast(target, shape, shadow),
        Command::Picture { picture, transform } => {
            let inner = picture.display_list().commands();
            CGContext::save_g_state(context);
            CGContext::concat_ctm(context, cg::transform(*transform));
            draw(target, inner, 0..inner.len());
            CGContext::restore_g_state(context);
        }
        Command::BeginClip { shape, end, .. } => {
            CGContext::save_g_state(context);
            CGContext::add_path(context, Some(&cg::path(shape)));
            clip(target.context, cg::fill_rule(shape));
            inside(*end);
            CGContext::restore_g_state(context);
        }
        Command::BeginTransform { transform, end } => {
            CGContext::save_g_state(context);
            CGContext::concat_ctm(context, cg::transform(*transform));
            inside(*end);
            CGContext::restore_g_state(context);
        }
        Command::BeginGroup { group, end } => {
            plan::reject_group_filter(group);
            let Some(mode) = blend_mode(group.blend) else {
                // `Dst`: the destination stays as it is.
                return;
            };
            CGContext::save_g_state(context);
            CGContext::set_alpha(context, f64::from(group.opacity));
            CGContext::set_blend_mode(context, mode);
            // SAFETY: the context is live; no auxiliary information.
            unsafe { CGContext::begin_transparency_layer(context, None) };
            inside(*end);
            CGContext::end_transparency_layer(context);
            CGContext::restore_g_state(context);
        }
        Command::Text {
            layout,
            transform,
            paint,
        } => {
            let text = target.registry.text(*layout, paint.as_ref());
            let inner = text.display_list().commands();
            CGContext::save_g_state(context);
            CGContext::concat_ctm(context, cg::transform(*transform));
            draw(target, inner, 0..inner.len());
            CGContext::restore_g_state(context);
        }
        Command::End => unreachable!("an item is never a scope's End"),
    }
}

fn clip(context: &CGContext, rule: FillRule) {
    match rule {
        FillRule::NonZero => CGContext::clip(Some(context)),
        FillRule::EvenOdd => CGContext::eo_clip(Some(context)),
    }
}

/// Fills `outline` under `rule` with `paint`.
fn cover(target: Target<'_>, outline: &BezPath, rule: FillRule, paint: &Paint) {
    let context = Some(target.context);
    let path = cg::path_of(outline.elements().iter().copied());
    match plan::classify(paint) {
        PaintClass::Solid(color) => {
            CGContext::set_fill_color_with_color(context, Some(&cg::color(color)));
            CGContext::add_path(context, Some(&path));
            match rule {
                FillRule::NonZero => CGContext::fill_path(context),
                FillRule::EvenOdd => CGContext::eo_fill_path(context),
            }
        }
        PaintClass::Drawn => {
            let Some(bounds) = cg::bounds(&path) else {
                return;
            };
            CGContext::save_g_state(context);
            CGContext::add_path(context, Some(&path));
            clip(target.context, rule);
            draw::paint(target.context, paint, bounds, target.registry);
            CGContext::restore_g_state(context);
        }
        PaintClass::Mesh { mesh, transform } => {
            CGContext::save_g_state(context);
            CGContext::add_path(context, Some(&path));
            clip(target.context, rule);
            draw::mesh(target.context, mesh, transform);
            CGContext::restore_g_state(context);
        }
    }
}

/// Draws only the shadow of `shape`: the shape is filled a whole bitmap
/// away, outside it, and its shadow is offset back. Core Graphics takes
/// shadow offsets and blur in the bitmap's pixels, not the CTM's units,
/// so both pass through the CTM here.
fn cast(target: Target<'_>, shape: &ShapeData, shadow: &Shadow) {
    let path = cg::spread(shape, shadow.spread);
    let Some(bounds) = cg::bounds(&path) else {
        return;
    };
    let context = Some(target.context);
    let ctm = CGContext::ctm(context);
    let linear = Affine::new([ctm.a, ctm.b, ctm.c, ctm.d, 0.0, 0.0]);
    let scale = linear.determinant().abs().sqrt();
    let reach = shadow.sigma.mul_add(SHADOW_REACH_SIGMAS, 1.0);
    let away = Vec2::new(
        2.0f64
            .mul_add(reach, bounds.width() + bounds.height())
            .mul_add(scale, 2.0 * target.reach),
        0.0,
    );
    let offset = (linear * shadow.offset.to_point()).to_vec2() - away;
    CGContext::save_g_state(context);
    CGContext::set_shadow_with_color(
        context,
        CGSize::new(offset.x, offset.y),
        shadow.sigma * scale * BLUR_PER_SIGMA,
        Some(&cg::color(shadow.color)),
    );
    let away = (linear.inverse() * away.to_point()).to_vec2();
    CGContext::concat_ctm(context, cg::transform(Affine::translate(away)));
    CGContext::set_fill_color_with_color(context, Some(&cg::color(WorkingColor::BLACK)));
    CGContext::add_path(context, Some(&path));
    CGContext::fill_path(context);
    CGContext::restore_g_state(context);
}

/// Whether anything `range` draws needs more than SDR.
#[must_use]
pub fn extended(commands: &[Command], range: Range<usize>) -> bool {
    commands[range].iter().any(|command| match command {
        Command::Fill { paint, .. }
        | Command::Stroke { paint, .. }
        | Command::Glyphs { paint, .. } => cg::paint_extended(paint),
        Command::Shadow { shadow, .. } => cg::color_extended(shadow.color),
        Command::Picture { picture, .. } => {
            let inner = picture.display_list().commands();
            extended(inner, 0..inner.len())
        }
        _ => false,
    })
}

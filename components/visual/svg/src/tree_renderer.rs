//! Draws a parsed SVG document through the engine-independent [`Scene2D`].
//!
//! `vello_svg` renders into a `vello::Scene` and nothing else, which ties every
//! icon in an application to one rendering engine. That engine's compute
//! pipeline needs indirect execution, which the iOS Simulator's Metal does not
//! have — so an icon aborts the process there rather than drawing. Going
//! through [`Scene2D`] instead lets the same document render on whichever
//! engine the device can actually run.
//!
//! The translation follows `vello_svg`'s own renderer, which is where the
//! handling of paint order, clip paths, nested documents and flattened text
//! comes from.

use kurbo::{Affine, BezPath, Rect, Shape as _, Stroke};
use peniko::{BlendMode, Brush, Fill, Mix};
use waterui_graphics::Scene2D;

use crate::usvg;

/// Draws a whole document into `scene`, positioned by `transform`.
pub fn render_tree(scene: &mut dyn Scene2D, tree: &usvg::Tree, transform: Affine) {
    render_group(scene, tree.root(), transform);
}

fn render_group(scene: &mut dyn Scene2D, group: &usvg::Group, transform: Affine) {
    for node in group.children() {
        let transform = transform * to_affine(&node.abs_transform());
        match node {
            usvg::Node::Group(group) => render_nested_group(scene, group, transform),
            usvg::Node::Path(path) => render_path(scene, path, transform),
            usvg::Node::Image(image) => render_image(scene, image, transform),
            // Text is drawn from its flattened outlines, so it needs no font
            // machinery of its own here.
            usvg::Node::Text(text) => render_group(scene, text.flattened(), transform),
        }
    }
}

fn render_nested_group(scene: &mut dyn Scene2D, group: &usvg::Group, transform: Affine) {
    let alpha = group.opacity().get();
    let blend = to_blend_mode(group.blend_mode());

    // A clip path with a single path clips to it; anything else clips to the
    // group's bounding box, which is what `vello_svg` does and keeps the layer
    // stack balanced either way.
    let clip = group
        .clip_path()
        .and_then(|path| path.root().children().first())
        .and_then(|node| match node {
            usvg::Node::Path(path) => Some(to_bez_path(path)),
            _ => None,
        })
        .unwrap_or_else(|| {
            let bounds = group.layer_bounding_box();
            let rect = Rect::from_origin_size(
                (f64::from(bounds.x()), f64::from(bounds.y())),
                (f64::from(bounds.width()), f64::from(bounds.height())),
            );
            let mut path = BezPath::new();
            path.extend(rect.path_elements(0.1));
            path
        });

    scene.push_layer(Fill::NonZero, blend, alpha, transform, &clip);
    // The children carry absolute transforms of their own, so the group's is
    // not applied twice.
    render_group(scene, group, Affine::IDENTITY);
    scene.pop_layer();
}

fn render_path(scene: &mut dyn Scene2D, path: &usvg::Path, transform: Affine) {
    if !path.is_visible() {
        return;
    }
    let outline = to_bez_path(path);

    let fill = || {
        let Some(fill) = path.fill() else { return None };
        let brush = to_brush(fill.paint(), fill.opacity())?;
        let rule = match fill.rule() {
            usvg::FillRule::NonZero => Fill::NonZero,
            usvg::FillRule::EvenOdd => Fill::EvenOdd,
        };
        Some((rule, brush))
    };
    let stroke = || {
        let Some(stroke) = path.stroke() else {
            return None;
        };
        let brush = to_brush(stroke.paint(), stroke.opacity())?;
        Some((to_stroke(stroke), brush))
    };

    let draw_fill = |scene: &mut dyn Scene2D| {
        if let Some((rule, brush)) = fill() {
            scene.fill(rule, transform, &brush, &outline);
        }
    };
    let draw_stroke = |scene: &mut dyn Scene2D| {
        if let Some((stroke, brush)) = stroke() {
            scene.stroke(&stroke, transform, &brush, &outline);
        }
    };

    match path.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            draw_fill(scene);
            draw_stroke(scene);
        }
        usvg::PaintOrder::StrokeAndFill => {
            draw_stroke(scene);
            draw_fill(scene);
        }
    }
}

fn render_image(scene: &mut dyn Scene2D, image: &usvg::Image, transform: Affine) {
    if !image.is_visible() {
        return;
    }
    match image.kind() {
        // A nested document is just more of the same tree.
        usvg::ImageKind::SVG(svg) => render_group(scene, svg.root(), transform),
        // Raster images inside an SVG are not decoded here. An icon set does not
        // use them, and pulling an image decoder into every application that
        // draws an icon is not worth the one case that would.
        _ => {
            tracing::debug!(
                target: "waterui::svg",
                "Raster image inside an SVG is not drawn"
            );
        }
    }
}

fn to_blend_mode(mode: usvg::BlendMode) -> BlendMode {
    match mode {
        usvg::BlendMode::Normal => Mix::Normal.into(),
        usvg::BlendMode::Multiply => Mix::Multiply.into(),
        usvg::BlendMode::Screen => Mix::Screen.into(),
        usvg::BlendMode::Overlay => Mix::Overlay.into(),
        usvg::BlendMode::Darken => Mix::Darken.into(),
        usvg::BlendMode::Lighten => Mix::Lighten.into(),
        usvg::BlendMode::ColorDodge => Mix::ColorDodge.into(),
        usvg::BlendMode::ColorBurn => Mix::ColorBurn.into(),
        usvg::BlendMode::HardLight => Mix::HardLight.into(),
        usvg::BlendMode::SoftLight => Mix::SoftLight.into(),
        usvg::BlendMode::Difference => Mix::Difference.into(),
        usvg::BlendMode::Exclusion => Mix::Exclusion.into(),
        usvg::BlendMode::Hue => Mix::Hue.into(),
        usvg::BlendMode::Saturation => Mix::Saturation.into(),
        usvg::BlendMode::Color => Mix::Color.into(),
        usvg::BlendMode::Luminosity => Mix::Luminosity.into(),
    }
}

fn to_affine(transform: &usvg::Transform) -> Affine {
    Affine::new([
        f64::from(transform.sx),
        f64::from(transform.ky),
        f64::from(transform.kx),
        f64::from(transform.sy),
        f64::from(transform.tx),
        f64::from(transform.ty),
    ])
}

fn to_bez_path(path: &usvg::Path) -> BezPath {
    let mut local = BezPath::new();
    for segment in path.data().segments() {
        match segment {
            usvg::tiny_skia_path::PathSegment::MoveTo(point) => {
                local.move_to((f64::from(point.x), f64::from(point.y)));
            }
            usvg::tiny_skia_path::PathSegment::LineTo(point) => {
                local.line_to((f64::from(point.x), f64::from(point.y)));
            }
            usvg::tiny_skia_path::PathSegment::QuadTo(control, end) => {
                local.quad_to(
                    (f64::from(control.x), f64::from(control.y)),
                    (f64::from(end.x), f64::from(end.y)),
                );
            }
            usvg::tiny_skia_path::PathSegment::CubicTo(first, second, end) => {
                local.curve_to(
                    (f64::from(first.x), f64::from(first.y)),
                    (f64::from(second.x), f64::from(second.y)),
                    (f64::from(end.x), f64::from(end.y)),
                );
            }
            usvg::tiny_skia_path::PathSegment::Close => local.close_path(),
        }
    }
    local
}

fn to_stroke(stroke: &usvg::Stroke) -> Stroke {
    Stroke::new(f64::from(stroke.width().get()))
        .with_caps(match stroke.linecap() {
            usvg::LineCap::Butt => kurbo::Cap::Butt,
            usvg::LineCap::Round => kurbo::Cap::Round,
            usvg::LineCap::Square => kurbo::Cap::Square,
        })
        .with_join(match stroke.linejoin() {
            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => kurbo::Join::Miter,
            usvg::LineJoin::Round => kurbo::Join::Round,
            usvg::LineJoin::Bevel => kurbo::Join::Bevel,
        })
        .with_miter_limit(f64::from(stroke.miterlimit().get()))
        .with_dashes(
            f64::from(stroke.dashoffset()),
            stroke
                .dasharray()
                .map(|array| array.iter().copied().map(f64::from))
                .into_iter()
                .flatten(),
        )
}

/// The paint a fill or stroke uses, when it is one this draws.
///
/// Gradients and patterns resolve to their own paint servers, which an icon set
/// does not use; a document that does gets nothing drawn for that shape rather
/// than a wrong colour.
fn to_brush(paint: &usvg::Paint, opacity: usvg::Opacity) -> Option<Brush> {
    match paint {
        usvg::Paint::Color(color) => Some(Brush::Solid(peniko::Color::from_rgba8(
            color.red,
            color.green,
            color.blue,
            opacity.to_u8(),
        ))),
        _ => None,
    }
}

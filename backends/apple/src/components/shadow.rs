//! The `shadow` metadata: `Metadata<Shadow>` wrapped around a child.
//!
//! Mirrors `WuiShadow`: a transparent `HostView` container that installs a
//! drop shadow on its own layer — offset, radius and opacity on the layer,
//! plus a `shadowPath` resolved from the silhouette on every layout so Core
//! Animation never has to derive the caster shape from the rendered
//! content. The color resolves through the environment, so every change
//! lands as an imperative setter inside a watcher, then
//! `invalidateCapturedRendering` fires so a cached capture re-renders.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::Cell;

use cocoa_ui::layer;
use cocoa_ui::path::ShapeKind as CocoaShapeKind;
use cocoa_ui::{PlatformView, Rect, Retained, Size, view};
use waterui::graphics::color::WorkingColor;
use waterui::reactive::{Computed, Signal};
use waterui::shape::{PathCommand, ShapeKind};
use waterui::style::Shadow;
use waterui_core::Metadata;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};

use crate::animation::with_platform_animation;
use crate::contract::{Held, Mounted, NativeLeaf};
use crate::proposal;

use crate::contract::RenderContext;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::colors;
    pub(super) use cocoa_ui::objc2_app_kit::NSColor as PlatformColor;
}
#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor as PlatformColor;
    pub(super) use cocoa_ui::uikit::colors;
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object — the
/// same conversion `resolved_color` applies.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<platform::PlatformColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object, with HDR
/// headroom applied as a content-headroom multiplier — the `AppKit` variant.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor) -> Retained<platform::PlatformColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

fn corner_radius(radius: waterui::shape::CornerRadius) -> cocoa_ui::path::CornerRadius {
    match (radius.as_points(), radius.as_fraction()) {
        (Some(points), None) => cocoa_ui::path::CornerRadius::Points(f64::from(points)),
        (None, Some(fraction)) => cocoa_ui::path::CornerRadius::Fraction(f64::from(fraction)),
        _ => unreachable!("CornerRadius is either points or fraction"),
    }
}

/// The silhouette as its kit kind — `ShapeKind` answers what the shape *is*
/// so a normalized radius resolves against the shorter side.
fn shape_kind(kind: ShapeKind) -> cocoa_ui::path::ShapeKind {
    match kind {
        ShapeKind::Rect => cocoa_ui::path::ShapeKind::Rect,
        ShapeKind::Circle => cocoa_ui::path::ShapeKind::Circle,
        ShapeKind::Ellipse => cocoa_ui::path::ShapeKind::Ellipse,
        ShapeKind::RoundedRect { radii } => cocoa_ui::path::ShapeKind::RoundedRect {
            radii: [
                corner_radius(radii.top_leading()),
                corner_radius(radii.top_trailing()),
                corner_radius(radii.bottom_trailing()),
                corner_radius(radii.bottom_leading()),
            ],
        },
        ShapeKind::Capsule => cocoa_ui::path::ShapeKind::Capsule,
        ShapeKind::CustomPath => cocoa_ui::path::ShapeKind::CustomPath,
    }
}

/// The silhouette's unit-space commands as kit commands.
fn shape_commands(commands: &[PathCommand]) -> Vec<cocoa_ui::path::Command> {
    commands
        .iter()
        .map(|command| match *command {
            PathCommand::MoveTo { x, y } => cocoa_ui::path::Command::MoveTo {
                x: f64::from(x),
                y: f64::from(y),
            },
            PathCommand::LineTo { x, y } => cocoa_ui::path::Command::LineTo {
                x: f64::from(x),
                y: f64::from(y),
            },
            PathCommand::QuadTo { cx, cy, x, y } => cocoa_ui::path::Command::QuadTo {
                cx: f64::from(cx),
                cy: f64::from(cy),
                x: f64::from(x),
                y: f64::from(y),
            },
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => cocoa_ui::path::Command::CubicTo {
                c1x: f64::from(c1x),
                c1y: f64::from(c1y),
                c2x: f64::from(c2x),
                c2y: f64::from(c2y),
                x: f64::from(x),
                y: f64::from(y),
            },
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => cocoa_ui::path::Command::Arc {
                cx: f64::from(cx),
                cy: f64::from(cy),
                rx: f64::from(rx),
                ry: f64::from(ry),
                start: f64::from(start),
                sweep: f64::from(sweep),
            },
            PathCommand::Close => cocoa_ui::path::Command::Close,
        })
        .collect()
}

/// The leaf's live state: the mounted child and reactive geometry metrics.
struct ShadowState {
    /// The mounted content.
    child: Mounted,
    offset_x: Cell<f32>,
    offset_y: Cell<f32>,
    radius: Cell<f32>,
}

impl core::fmt::Debug for ShadowState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShadowState").finish_non_exhaustive()
    }
}

/// `applyShadowColor`: the resolved color lands as `shadowColor` at full
/// opacity plus `shadowOpacity` carrying the alpha, then any captured
/// rendering is invalidated.
fn apply_shadow_color(color: &WorkingColor, host: &PlatformView) {
    let platform = platform_color(color);
    let opaque = platform::colors::with_alpha(&platform, 1.0);
    if let Some(layer) = layer::layer_of(host) {
        layer::set_shadow_color(&layer, Some(&platform::colors::cg(&opaque)));
        layer::set_shadow_opacity(&layer, color.components[3]);
    }
    view::invalidate_captured_rendering(host);
}

fn apply_shadow_geometry(host: &PlatformView, state: &ShadowState) {
    if let Some(host_layer) = layer::layer_of(host) {
        layer::set_shadow(
            &host_layer,
            Size::new(
                f64::from(state.offset_x.get()),
                f64::from(state.offset_y.get()),
            ),
            f64::from(state.radius.get().max(0.0)),
        );
    }
    view::invalidate_captured_rendering(host);
}

fn watch_shadow_metric(
    leaf: &mut NativeLeaf,
    signal: &Computed<f32>,
    slot: impl Fn(&ShadowState) -> &Cell<f32> + 'static,
    state: &Rc<ShadowState>,
    host: &Retained<HostView>,
) {
    let state = Rc::clone(state);
    let host = host.clone();
    leaf.watch(signal, move |wctx| {
        slot(&state).set(*wctx.value());
        let state = Rc::clone(&state);
        let host = host.clone();
        with_platform_animation(wctx.metadata(), move || {
            apply_shadow_geometry(&host, &state);
        });
    });
}

/// `updateShadowPath`: the silhouette resolved against the host's bounds —
/// skipped on empty bounds like the Swift `guard`.
fn update_shadow_path(
    host: &PlatformView,
    kind: cocoa_ui::path::ShapeKind,
    commands: &[cocoa_ui::path::Command],
) {
    let bounds = view::bounds(host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return;
    }
    let path = cocoa_ui::path::shape_path(kind, commands, bounds);
    if let Some(layer) = layer::layer_of(host) {
        layer::set_shadow_path(&layer, Some(&path));
    }
}

/// The wrapper's layout face: the content's answers everywhere.
struct ShadowSubView {
    /// The leaf's state.
    state: Rc<ShadowState>,
}

impl core::fmt::Debug for ShadowSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShadowSubView").finish_non_exhaustive()
    }
}

impl SubView for ShadowSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.state.child.layout().resolve_extent(bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

/// The `shadow` realization.
pub fn realize_shadow_metadata(
    metadata: Metadata<Shadow, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let shadow = metadata.value;

    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    layer::ensure_layer(&host);
    let child_leaf = ctx.realize(metadata.content);
    let inherited_clip = child_leaf.clip_shape_handle();
    let mounted = child_leaf.mount(&host);
    crate::primary_content::forward(&host, mounted.view());
    view::set_translates_autoresizing(mounted.view(), true);

    // `configureShadow`: offset and radius on the host's layer; the
    // layer must not mask the shadow it draws.
    let initial_offset_x = shadow.offset_x().snapshot();
    let initial_offset_y = shadow.offset_y().snapshot();
    let initial_radius = shadow.radius().snapshot();
    if let Some(host_layer) = layer::layer_of(&host) {
        layer::set_shadow(
            &host_layer,
            Size::new(f64::from(initial_offset_x), f64::from(initial_offset_y)),
            f64::from(initial_radius.max(0.0)),
        );
        layer::set_masks_to_bounds(&host_layer, false);
    }

    // The silhouette: explicit `.silhouette(...)` override wins; otherwise follow
    // the caster's `.clip(...)` shape, or fall back to a rectangle when unclipped.
    let (kind, commands) = shadow
        .get_silhouette()
        .or(inherited_clip.as_deref())
        .map_or_else(
            || (CocoaShapeKind::Rect, Vec::new()),
            |silhouette| {
                (
                    shape_kind(silhouette.kind()),
                    shape_commands(silhouette.commands()),
                )
            },
        );

    let state = Rc::new(ShadowState {
        child: mounted,
        offset_x: Cell::new(initial_offset_x),
        offset_y: Cell::new(initial_offset_y),
        radius: Cell::new(initial_radius),
    });

    // The content fills the wrapper; the shadow path follows its bounds.
    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host_view| {
            view::set_frame(state.child.view(), view::bounds(host_view));
            update_shadow_path(host_view, kind, &commands);
        }
    });

    // `setPlacementProposal`: the proposal selected for this wrapper is
    // the proposal its content was negotiated with.
    let sink_guard = proposal::register_sink(&host, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        &*host,
        ShadowSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.set_clip_shape_handle(inherited_clip);

    let resolved = shadow.get_color().resolve(ctx.env());
    apply_shadow_color(&resolved.snapshot(), &host);
    leaf.watch(&resolved, {
        let host = host.clone();
        move |wctx| {
            let color = *wctx.value();
            let host = host.clone();
            with_platform_animation(wctx.metadata(), move || {
                apply_shadow_color(&color, &host);
            });
        }
    });
    watch_shadow_metric(&mut leaf, shadow.offset_x(), |s| &s.offset_x, &state, &host);
    watch_shadow_metric(&mut leaf, shadow.offset_y(), |s| &s.offset_y, &state, &host);
    watch_shadow_metric(&mut leaf, shadow.radius(), |s| &s.radius, &state, &host);
    leaf.keep(state);
    leaf
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::shape::ShapeKind as WaterShapeKind;

    #[test]
    fn kind_conversion_preserves_per_corner_radii() {
        use waterui::shape::{CornerRadii, CornerRadius};

        assert_eq!(
            shape_kind(WaterShapeKind::RoundedRect {
                radii: CornerRadii::new(
                    CornerRadius::fraction(0.25),
                    CornerRadius::fraction(0.5),
                    CornerRadius::fraction(0.375),
                    CornerRadius::fraction(0.125),
                ),
            }),
            cocoa_ui::path::ShapeKind::RoundedRect {
                radii: [
                    cocoa_ui::path::CornerRadius::Fraction(0.25),
                    cocoa_ui::path::CornerRadius::Fraction(0.5),
                    cocoa_ui::path::CornerRadius::Fraction(0.125),
                    cocoa_ui::path::CornerRadius::Fraction(0.375),
                ],
            }
        );
        assert_eq!(
            shape_kind(WaterShapeKind::RoundedRect {
                radii: CornerRadii::uniform(CornerRadius::points(12.0)),
            }),
            cocoa_ui::path::ShapeKind::RoundedRect {
                radii: [cocoa_ui::path::CornerRadius::Points(12.0); 4],
            }
        );
        assert_eq!(
            shape_kind(WaterShapeKind::CustomPath),
            cocoa_ui::path::ShapeKind::CustomPath
        );
    }

    #[test]
    fn command_conversion_carries_every_field() {
        let commands = shape_commands(&[
            PathCommand::MoveTo { x: 1.0, y: 2.0 },
            PathCommand::QuadTo {
                cx: 0.25,
                cy: 0.5,
                x: 0.75,
                y: 1.0,
            },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.25,
                ry: 0.5,
                start: 0.0,
                sweep: 1.5,
            },
            PathCommand::Close,
        ]);
        assert_eq!(
            commands,
            vec![
                cocoa_ui::path::Command::MoveTo { x: 1.0, y: 2.0 },
                cocoa_ui::path::Command::QuadTo {
                    cx: 0.25,
                    cy: 0.5,
                    x: 0.75,
                    y: 1.0,
                },
                cocoa_ui::path::Command::Arc {
                    cx: 0.5,
                    cy: 0.5,
                    rx: 0.25,
                    ry: 0.5,
                    start: 0.0,
                    sweep: 1.5,
                },
                cocoa_ui::path::Command::Close,
            ]
        );
    }
}

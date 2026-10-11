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

use cocoa_ui::layer;
use cocoa_ui::{PlatformView, Rect, Retained, Size, view};
use waterui::graphics::color::WorkingColor;
use waterui::shape::{ClipShape, PathCommand, ShapeKind};
use waterui::style::Shadow;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

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

/// The silhouette as its kit kind — `ShapeKind` answers what the shape *is*
/// so a normalized radius resolves against the shorter side.
fn shape_kind(kind: ShapeKind) -> cocoa_ui::path::ShapeKind {
    match kind {
        ShapeKind::Rect => cocoa_ui::path::ShapeKind::Rect,
        ShapeKind::Circle => cocoa_ui::path::ShapeKind::Circle,
        ShapeKind::Ellipse => cocoa_ui::path::ShapeKind::Ellipse,
        ShapeKind::RoundedRect { corner_radius } => cocoa_ui::path::ShapeKind::RoundedRect {
            corner_radius: f64::from(corner_radius),
        },
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => cocoa_ui::path::ShapeKind::UnevenRoundedRect {
            top_left: f64::from(top_left),
            top_right: f64::from(top_right),
            bottom_right: f64::from(bottom_right),
            bottom_left: f64::from(bottom_left),
        },
        ShapeKind::Capsule => cocoa_ui::path::ShapeKind::Capsule,
        ShapeKind::FixedRoundedRect { corner_radius } => {
            cocoa_ui::path::ShapeKind::FixedRoundedRect {
                corner_radius: f64::from(corner_radius),
            }
        }
        ShapeKind::FixedUnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => cocoa_ui::path::ShapeKind::FixedUnevenRoundedRect {
            top_left: f64::from(top_left),
            top_right: f64::from(top_right),
            bottom_right: f64::from(bottom_right),
            bottom_left: f64::from(bottom_left),
        },
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

/// The leaf's live state: the mounted child.
struct ShadowState {
    /// The mounted content.
    child: Mounted,
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

/// Installs the `shadow` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Shadow>>(|metadata, ctx| {
        let shadow = metadata.value;

        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        layer::ensure_layer(&host);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        // `configureShadow`: offset and radius on the host's layer; the
        // layer must not mask the shadow it draws.
        if let Some(host_layer) = layer::layer_of(&host) {
            layer::set_shadow(
                &host_layer,
                Size::new(f64::from(shadow.offset.x), f64::from(shadow.offset.y)),
                f64::from(shadow.radius),
            );
            layer::set_masks_to_bounds(&host_layer, false);
        }

        // The silhouette: `WuiShapePath.commands(from:)`'s pair — the
        // structured kind first, the unit-space commands as the custom-path
        // fallback.
        let silhouette: &ClipShape = &shadow.silhouette;
        let kind = shape_kind(silhouette.kind());
        let commands = shape_commands(silhouette.commands());

        let state = Rc::new(ShadowState { child: mounted });

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

        let resolved = shadow.color.resolve(ctx.env());
        leaf.bind(&resolved, {
            move |color| {
                apply_shadow_color(&color, &host);
            }
        });
        leaf.keep(state);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::shape::ShapeKind as WaterShapeKind;

    #[test]
    fn kind_conversion_preserves_per_corner_radii() {
        assert_eq!(
            shape_kind(WaterShapeKind::UnevenRoundedRect {
                top_left: 0.25,
                top_right: 0.5,
                bottom_left: 0.375,
                bottom_right: 0.125,
            }),
            cocoa_ui::path::ShapeKind::UnevenRoundedRect {
                top_left: 0.25,
                top_right: 0.5,
                bottom_right: 0.125,
                bottom_left: 0.375,
            }
        );
        assert_eq!(
            shape_kind(WaterShapeKind::RoundedRect { corner_radius: 0.5 }),
            cocoa_ui::path::ShapeKind::RoundedRect { corner_radius: 0.5 }
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

//! Native Apple realization of [`Avatar`](waterui::widget::avatar::Avatar).

use alloc::rc::Rc;
use alloc::vec::Vec;

use cocoa_ui::accessibility::{self as a11y, Role};
use cocoa_ui::{PlatformView, Rect, Retained, layer, shape, view};
use objc2_foundation::NSString;
use objc2_quartz_core::CAShapeLayer;
#[cfg(waterui_feature = "image")]
use waterui::layout::ContentMode;
#[cfg(waterui_feature = "image")]
use waterui::media::Photo;
use waterui::reactive::Signal as _;
use waterui::shape::{Circle, ClipShape, PathCommand, ShapeKind};
use waterui::text::Text;
use waterui::theme::color::{MutedForeground, SurfaceVariant};
use waterui::widget::avatar::{AvatarConfig, AvatarRing};
use waterui_core::Environment;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, Size, StretchAxis, SubView,
    ViewDimensions,
};
use waterui_graphics::color::Color;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

use crate::animation::with_platform_animation;
use crate::components::accessibility_metadata::target;
use crate::components::border::cg_color;
use crate::components::clip_shape::shape_path;
use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::proposal;

/// Default avatar side length in logical points when callers do not specify one.
const DEFAULT_SIZE: f32 = 40.0;

/// Monogram font-size scale factor relative to the inner side length.
const INITIALS_SCALE: f32 = 0.4;

#[expect(
    clippy::cast_possible_truncation,
    reason = "CoreGraphics view geometry originates from f32 WaterUI layout coordinates"
)]
const fn cg_f32(value: f64) -> f32 {
    value as f32
}

struct AvatarContentState {
    bg_layer: layer::ShapeLayer,
    mask_layer: Retained<CAShapeLayer>,
    kind: ShapeKind,
    commands: Vec<PathCommand>,
    fallback: Option<Mounted>,
    #[cfg(waterui_feature = "image")]
    picture: Option<Mounted>,
}

struct AvatarState {
    content_host: Retained<HostView>,
    content: Rc<AvatarContentState>,
    ring_layer: Option<layer::ShapeLayer>,
    ring_width: f32,
    side: f32,
}

#[derive(Debug)]
struct AvatarSubView {
    side: f32,
}

impl SubView for AvatarSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(self.side, self.side))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

fn bounded_proposal(bounds: Rect) -> ProposalSize {
    ProposalSize::new(
        Some(cg_f32(bounds.size.width)),
        Some(cg_f32(bounds.size.height)),
    )
}

fn layout_content(content_view: &PlatformView, state: &AvatarContentState) {
    let inner_bounds = view::bounds(content_view);
    if inner_bounds.size.width <= 0.0 || inner_bounds.size.height <= 0.0 {
        return;
    }

    let path = shape_path(state.kind, &state.commands, inner_bounds);
    shape::set_path(&state.mask_layer, &path);
    state.bg_layer.set_frame(inner_bounds);
    state.bg_layer.set_path(Some(&path));

    if let Some(fallback) = &state.fallback {
        let measured = fallback.layout().measure(bounded_proposal(inner_bounds));
        let child_w = f64::from(measured.size.width)
            .min(inner_bounds.size.width)
            .max(0.0);
        let child_h = f64::from(measured.size.height)
            .min(inner_bounds.size.height)
            .max(0.0);
        let child_frame = Rect::new(
            (inner_bounds.size.width - child_w) * 0.5,
            (inner_bounds.size.height - child_h) * 0.5,
            child_w,
            child_h,
        );
        view::set_frame(fallback.view(), child_frame);
    }

    #[cfg(waterui_feature = "image")]
    if let Some(picture) = &state.picture {
        view::set_frame(picture.view(), inner_bounds);
    }
}

fn configure_content_clip(content_host: &HostView) {
    #[cfg(target_os = "ios")]
    content_host.setClipsToBounds(true);
    #[cfg(target_os = "macos")]
    if let Some(content_layer) = shape::layer(content_host) {
        content_layer.setMasksToBounds(true);
    }
}

fn build_content_state(
    config: &mut AvatarConfig<Held>,
    ctx: &RenderContext<'_>,
    content_host: &HostView,
    inner_side: f32,
) -> Rc<AvatarContentState> {
    let default_shape = ClipShape::new(Circle);
    let shape = config.shape().unwrap_or(&default_shape);
    let kind = shape.kind();
    let commands = shape.commands().to_vec();

    let bg_layer = layer::ShapeLayer::new();
    bg_layer.set_stroke(None);
    if let Some(content_layer) = layer::layer_of(content_host) {
        layer::add_sublayer(&content_layer, bg_layer.layer());
    }

    let mask_layer = shape::shape_layer();
    shape::set_mask(content_host, Some(&mask_layer));

    let fallback = config.take_fallback().map_or_else(
        || {
            config.monogram().map(|monogram| {
                let monogram_text = Text::new(monogram.clone())
                    .font_size(f64::from(inner_side * INITIALS_SCALE))
                    .color(Color::new(MutedForeground));
                ctx.render(monogram_text).mount(content_host)
            })
        },
        |slot| Some(ctx.realize(slot).mount(content_host)),
    );

    #[cfg(waterui_feature = "image")]
    let picture = config.image().map(|source| {
        let photo = Photo::new(source.clone())
            .resizable()
            .content_mode(ContentMode::Fill);
        ctx.render(photo).mount(content_host)
    });

    Rc::new(AvatarContentState {
        bg_layer,
        mask_layer,
        kind,
        commands,
        fallback,
        #[cfg(waterui_feature = "image")]
        picture,
    })
}

fn attach_layout_and_proposals(host: &HostView, state: &Rc<AvatarState>) -> proposal::SinkGuard {
    state.content_host.set_layout_handler({
        let content_state = Rc::clone(&state.content);
        move |content_view| {
            layout_content(content_view, &content_state);
        }
    });

    host.set_layout_handler({
        let state = Rc::clone(state);
        move |host_view| {
            let bounds = view::bounds(host_view);
            let outer_side = cg_f32(bounds.size.width)
                .min(cg_f32(bounds.size.height))
                .max(0.0);
            let inset = f64::from(state.ring_width);
            let inner_frame = Rect::new(
                bounds.origin.x + inset,
                bounds.origin.y + inset,
                f64::mul_add(inset, -2.0, bounds.size.width).max(0.0),
                f64::mul_add(inset, -2.0, bounds.size.height).max(0.0),
            );
            view::set_frame(&state.content_host, inner_frame);
            layout_content(&state.content_host, &state.content);

            if let Some(ring) = &state.ring_layer {
                let clamped = state.ring_width.min(outer_side);
                let half = f64::from(clamped) * 0.5;
                let stroke_bounds = Rect::new(
                    bounds.origin.x + half,
                    bounds.origin.y + half,
                    (bounds.size.width - f64::from(clamped)).max(0.0),
                    (bounds.size.height - f64::from(clamped)).max(0.0),
                );
                ring.set_frame(bounds);
                ring.set_line_width(f64::from(clamped));
                let ring_path =
                    shape_path(state.content.kind, &state.content.commands, stroke_bounds);
                ring.set_path(Some(&ring_path));
            }
        }
    });

    proposal::register_sink(host, {
        let state = Rc::clone(state);
        move |selected| {
            let inner_proposal = selected.proposal.map(|p| {
                ProposalSize::new(
                    p.width
                        .map(|w| f32::mul_add(state.ring_width, -2.0, w).max(0.0)),
                    p.height
                        .map(|h| f32::mul_add(state.ring_width, -2.0, h).max(0.0)),
                )
            });
            let inner_placement = proposal::Placement {
                proposal: inner_proposal,
                ..selected
            };
            if let Some(fallback) = &state.content.fallback {
                proposal::deliver(fallback.view(), inner_placement);
            }
            #[cfg(waterui_feature = "image")]
            if let Some(picture) = &state.content.picture {
                proposal::deliver(picture.view(), inner_placement);
            }
        }
    })
}

fn attach_color_watchers(
    leaf: &mut NativeLeaf,
    host: Retained<HostView>,
    state: &Rc<AvatarState>,
    ring_cfg: Option<&AvatarRing>,
    env: &Environment,
) {
    let bg_color = Color::new(SurfaceVariant).resolve(env);
    let initial_bg = cg_color(&bg_color.snapshot());
    state.content.bg_layer.set_fill(Some(&initial_bg));
    leaf.watch(&bg_color, {
        let state = Rc::clone(state);
        let host = host.clone();
        move |wctx| {
            let color = *wctx.value();
            let state = Rc::clone(&state);
            let host = host.clone();
            with_platform_animation(wctx.metadata(), move || {
                let cg = cg_color(&color);
                state.content.bg_layer.set_fill(Some(&cg));
                view::invalidate_captured_rendering(&host);
            });
        }
    });

    if let Some(ring_cfg) = ring_cfg
        && let Some(ring_layer) = &state.ring_layer
    {
        let ring_color = ring_cfg.color().resolve(env);
        let initial_ring = cg_color(&ring_color.snapshot());
        ring_layer.set_stroke(Some(&initial_ring));
        leaf.watch(&ring_color, {
            let state = Rc::clone(state);
            move |wctx| {
                let color = *wctx.value();
                let state = Rc::clone(&state);
                let host = host.clone();
                with_platform_animation(wctx.metadata(), move || {
                    if let Some(ring_layer) = &state.ring_layer {
                        let cg = cg_color(&color);
                        ring_layer.set_stroke(Some(&cg));
                        view::invalidate_captured_rendering(&host);
                    }
                });
            }
        });
    }
}

/// Realizes an [`AvatarConfig`](waterui::widget::avatar::AvatarConfig) as a native
/// AppKit/UIKit `HostView` leaf.
pub fn realize_avatar_config(
    mut config: AvatarConfig<Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let side = config.side().unwrap_or(DEFAULT_SIZE).max(0.0);
    let ring_width = config.ring().map_or(0.0, |ring| ring.width().max(0.0));
    let inner_side = f32::mul_add(ring_width, -2.0, side).max(0.0);

    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    layer::ensure_layer(&host);

    let content_host = HostView::new(mtm, Rect::ZERO);
    layer::ensure_layer(&content_host);
    view::add_subview(&host, &content_host);
    configure_content_clip(&content_host);

    let content = build_content_state(&mut config, ctx, &content_host, inner_side);
    let ring_layer = config.ring().filter(|_| ring_width > 0.0).map(|_| {
        let ring = layer::ShapeLayer::new();
        ring.set_fill(None);
        ring.set_line_width(f64::from(ring_width));
        if let Some(host_layer) = layer::layer_of(&host) {
            layer::add_sublayer(&host_layer, ring.layer());
        }
        ring
    });

    let state = Rc::new(AvatarState {
        content_host,
        content,
        ring_layer,
        ring_width,
        side,
    });
    let sink_guard = attach_layout_and_proposals(&host, &state);

    let mut leaf = NativeLeaf::new(&*host, AvatarSubView { side: state.side });
    leaf.keep(sink_guard);
    attach_color_watchers(&mut leaf, host, &state, config.ring(), ctx.env());

    let a11y_target = view::retain_base(leaf.view());
    a11y::set_role(target::of(&a11y_target), Role::Image);
    a11y::exclude_children(target::of(&a11y_target));
    leaf.keep(a11y_target.clone());
    leaf.bind(config.spoken(), move |label| {
        a11y::set_label(target::of(&a11y_target), &NSString::from_str(&label));
    });

    leaf.keep(state);
    leaf
}

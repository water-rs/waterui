//! Native Apple realization of [`SnackbarHostConfig`]: compact floating Apple
//! confirmation banners anchored per [`SnackbarPosition`] on `AppKit` and
//! `UIKit`, with `SF Symbols` `xmark` close affordance, trailing action button,
//! safe-area awareness, RTL alignment, and Core Animation enter/reflow/exit
//! transitions.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt::{self, Debug, Formatter};
use core::time::Duration;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HitTest, HostView};
use cocoa_ui::core_animation::{animate_with, without_animation};
use cocoa_ui::geometry::Rect as KitRect;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HitTest, HostView};
use cocoa_ui::{PlatformView, Retained, view};
use executor_core::spawn_local;
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal as _};
use native_executor::sleep;
use waterui_accessibility::{
    AccessibilityAnnouncer, AccessibilityLiveRegion, AccessibilityRole, LiveRegion,
};
use waterui_backend_core::{Environment, SlotView};
use waterui_controls::button::{Button, ButtonStyle, button};
use waterui_controls::label::{Label, label};
use waterui_controls::slot::SlotBuilder;
use waterui_controls::snackbar::{
    SnackbarHostConfig, SnackbarPosition, SnackbarPresentation, snackbar_close_label,
};
use waterui_controls::theme::ForegroundOverride;
use waterui_controls::theme::color::{Accent, Border, Foreground, Surface};
use waterui_core::animation::Animation;
use waterui_core::env::Installed;
use waterui_core::layout::{
    LayoutDirection, LayoutError, ProposalSize, Rect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions, layout_direction,
};
use waterui_core::{Metadata, OneOf2, View};
use waterui_graphics::color::Color;
use waterui_layout::background::BackgroundView;
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::stack::hstack;
use waterui_shape::border::Border as BorderStyle;
use waterui_shape::style::Shadow;
use waterui_shape::{RoundedRectangle, ShapeExt as _};

use crate::SfSymbol;
use crate::animation::timing;
use crate::contract::{Held, Mounted, NativeLeaf, RenderContext, Renderer};
use crate::native_layout::LayoutContext;
use crate::primary_content;
use crate::proposal::{self, Placement};

struct BuiltIconSlot(SlotBuilder<Held>);

impl Clone for BuiltIconSlot {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl View for BuiltIconSlot {
    fn body(self, _env: &Environment) -> impl View {
        SlotView::new(self.0.build())
    }
}

const CONTENT_VERTICAL_PADDING: f32 = 14.0;
const CONTENT_HORIZONTAL_PADDING: f32 = 20.0;
const ACTION_TRAILING_PADDING: f32 = 12.0;
const VIEWPORT_PADDING: f64 = 16.0;
const CONTENT_SPACING: f32 = 10.0;
const MAX_BANNER_WIDTH: f32 = 480.0;
const MIN_BANNER_HEIGHT: f32 = 50.0;
const STACK_GAP: f64 = 8.0;
const CORNER_RADIUS: f32 = 14.0;
const BORDER_WIDTH: f32 = 0.5;
const CLOSE_ICON_SIZE: f32 = 14.0;
const CLOSE_BOX_SIZE: f32 = 32.0;
const MOTION_OFFSET_Y: f64 = 16.0;
const ENTER_DURATION: Duration = Duration::from_millis(280);
const EXIT_DURATION: Duration = Duration::from_millis(220);

const fn enter_animation() -> Animation {
    Animation::bezier(ENTER_DURATION, 0.2, 0.0, 0.0, 1.0)
}

const fn exit_animation() -> Animation {
    Animation::bezier(EXIT_DURATION, 0.4, 0.0, 1.0, 1.0)
}

const fn slide_delta(position: SnackbarPosition) -> f64 {
    if position.is_top() {
        -MOTION_OFFSET_Y
    } else {
        MOTION_OFFSET_Y
    }
}

#[derive(Clone, Copy)]
struct AppleCloseIcon;

impl View for AppleCloseIcon {
    fn body(self, _env: &Environment) -> impl View {
        let icon = Frame::new(Installed::new(
            SfSymbol::XMARK,
            ForegroundOverride::new(Foreground),
        ))
        .width(CLOSE_ICON_SIZE)
        .height(CLOSE_ICON_SIZE);
        Frame::new(icon)
            .width(CLOSE_BOX_SIZE)
            .height(CLOSE_BOX_SIZE)
    }
}

fn build_apple_banner(presentation: &SnackbarPresentation<Held>) -> impl View + use<> {
    let content_padding = if presentation.action().is_some() || presentation.closeable() {
        EdgeInsets::new(
            CONTENT_VERTICAL_PADDING,
            CONTENT_HORIZONTAL_PADDING,
            CONTENT_VERTICAL_PADDING,
            ACTION_TRAILING_PADDING,
        )
    } else {
        EdgeInsets::symmetric(CONTENT_VERTICAL_PADDING, CONTENT_HORIZONTAL_PADDING)
    };

    let label_view = Installed::new(
        presentation.icon().map_or_else(
            || OneOf2::B(label(presentation.message().clone())),
            |icon| {
                OneOf2::A(label(presentation.message().clone()).icon(BuiltIconSlot(icon.clone())))
            },
        ),
        ForegroundOverride::new(Foreground),
    );

    let action_button = presentation.action().map(|action| {
        let presentation_for_action = presentation.clone();
        let action_label = action.label().clone();
        let semantic_text = action_label.semantic_text().clone();
        button(Label::new(semantic_text, move || {
            Installed::new(action_label.clone(), ForegroundOverride::new(Accent))
        }))
        .style(ButtonStyle::Borderless)
        .action(move |env: Environment| {
            presentation_for_action.trigger_action(&env);
        })
    });

    let close_button = presentation.closeable().then(|| {
        let presentation_for_close = presentation.clone();
        Button::new(
            label(snackbar_close_label())
                .icon(AppleCloseIcon)
                .icon_only(),
        )
        .style(ButtonStyle::Borderless)
        .action(move |_env: Environment| {
            presentation_for_close.close();
        })
    });

    let shape = RoundedRectangle::new(CORNER_RADIUS);
    let key_shadow = Shadow::splat(18.0)
        .color(Color::srgb(0, 0, 0).with_opacity(0.18))
        .offset(0.0, 6.0)
        .silhouette(shape);
    let ambient_shadow = Shadow::splat(2.0)
        .color(Color::srgb(0, 0, 0).with_opacity(0.06))
        .offset(0.0, 1.0)
        .silhouette(shape);

    let content = Frame::new(Padding::new(
        content_padding,
        hstack((label_view, action_button, close_button)).spacing(CONTENT_SPACING),
    ))
    .min_height(MIN_BANNER_HEIGHT);
    let bordered = Metadata::new(
        BackgroundView::new(content, shape.fill(Surface)),
        BorderStyle::new(Border, BORDER_WIDTH).corner_radius(CORNER_RADIUS),
    );
    let shadowed = Metadata::new(Metadata::new(bordered, ambient_shadow), key_shadow);
    Metadata::new(
        Metadata::new(shadowed, AccessibilityRole::Status),
        AccessibilityLiveRegion::new(LiveRegion::Polite),
    )
}

struct AppleSnackbarRow {
    id: u64,
    position: SnackbarPosition,
    mounted: Mounted,
    dismissing: bool,
    appeared: bool,
    _dismiss_guard: BoxWatcherGuard,
}

struct SnackbarHostState {
    host: Retained<HostView>,
    overlay_host: Retained<HostView>,
    child: Mounted,
    renderer: Renderer,
    env: Environment,
    direction: Computed<LayoutDirection>,
    rows: RefCell<Vec<AppleSnackbarRow>>,
}

impl Debug for SnackbarHostState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnackbarHostState").finish_non_exhaustive()
    }
}

struct SnackbarHostSubView {
    state: Rc<SnackbarHostState>,
}

impl Debug for SnackbarHostSubView {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnackbarHostSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for SnackbarHostSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
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
}

fn target_frame_for_row(
    safe_rect: KitRect,
    position: SnackbarPosition,
    rtl: bool,
    banner_w: f64,
    banner_h: f64,
    stack_offset: f64,
) -> KitRect {
    let min_x = safe_rect.origin.x + VIEWPORT_PADDING;
    let max_x = (safe_rect.origin.x + safe_rect.size.width - VIEWPORT_PADDING).max(min_x);
    let min_y = safe_rect.origin.y + VIEWPORT_PADDING;
    let max_y = (safe_rect.origin.y + safe_rect.size.height - VIEWPORT_PADDING).max(min_y);

    let x = match position {
        SnackbarPosition::BottomLeading => {
            if rtl {
                (max_x - banner_w).max(min_x)
            } else {
                min_x
            }
        }
        SnackbarPosition::BottomTrailing => {
            if rtl {
                min_x
            } else {
                (max_x - banner_w).max(min_x)
            }
        }
        SnackbarPosition::BottomCenter | SnackbarPosition::TopCenter | _ => {
            ((max_x - min_x) - banner_w).max(0.0).mul_add(0.5, min_x)
        }
    };

    let y = if position.is_top() {
        min_y + stack_offset
    } else {
        (max_y - banner_h - stack_offset).max(min_y)
    };

    KitRect::new(x, y, banner_w, banner_h)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "window and banner coordinates in logical points always fit f32"
)]
fn layout_banners(state: &Rc<SnackbarHostState>, animated: bool) {
    let bounds = view::bounds(&state.overlay_host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return;
    }
    let safe_rect = LayoutContext::of(&state.host).safe_rect();
    let rtl = matches!(state.direction.snapshot(), LayoutDirection::RightToLeft);
    let avail_w = VIEWPORT_PADDING
        .mul_add(-2.0, safe_rect.size.width)
        .max(0.0) as f32;
    let cap_w = avail_w.min(MAX_BANNER_WIDTH);

    let mut placements: Vec<(Retained<PlatformView>, KitRect, bool, SnackbarPosition)> = Vec::new();
    {
        let mut rows = state.rows.borrow_mut();
        let mut measured: Vec<(f64, f64)> = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let size = row
                .mounted
                .layout()
                .measure(ProposalSize::new(Some(cap_w), None))
                .size;
            let w = size.width.min(cap_w);
            let h = size.height.max(MIN_BANNER_HEIGHT);
            proposal::deliver(
                row.mounted.view(),
                Placement::platform_hosted(ProposalSize::new(Some(w), Some(h))),
            );
            measured.push((f64::from(w), f64::from(h)));
        }

        for index in 0..rows.len() {
            if rows[index].dismissing {
                continue;
            }
            let position = rows[index].position;
            let mut stack_offset = 0.0;
            for earlier in 0..index {
                if !rows[earlier].dismissing && rows[earlier].position == position {
                    stack_offset += measured[earlier].1 + STACK_GAP;
                }
            }
            let (w, h) = measured[index];
            let frame = target_frame_for_row(safe_rect, position, rtl, w, h, stack_offset);
            let first_appear = !rows[index].appeared;
            rows[index].appeared = true;
            placements.push((
                view::retain_base(rows[index].mounted.view()),
                frame,
                first_appear,
                position,
            ));
        }
    }

    for (banner_view, frame, first_appear, position) in placements {
        if first_appear {
            let mut start = frame;
            start.origin.y += slide_delta(position);
            without_animation(|| {
                view::set_frame(&banner_view, start);
                view::set_alpha(&banner_view, 0.0);
            });
            let target_view = Retained::clone(&banner_view);
            animate_with(timing(&enter_animation()), move || {
                view::set_frame(&target_view, frame);
                view::set_alpha(&target_view, 1.0);
            });
        } else if animated {
            let target_view = Retained::clone(&banner_view);
            animate_with(timing(&enter_animation()), move || {
                view::set_frame(&target_view, frame);
            });
        } else {
            view::set_frame(&banner_view, frame);
        }
    }
}

fn begin_row_exit(
    state: &Rc<SnackbarHostState>,
    id: u64,
    presentation: SnackbarPresentation<Held>,
) {
    let exiting = {
        let mut rows = state.rows.borrow_mut();
        let Some(row) = rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        if row.dismissing {
            return;
        }
        row.dismissing = true;
        (view::retain_base(row.mounted.view()), row.position)
    };

    layout_banners(state, true);

    let (banner_view, position) = exiting;
    let mut end_frame = view::frame(&banner_view);
    end_frame.origin.y += slide_delta(position);
    animate_with(timing(&exit_animation()), move || {
        view::set_frame(&banner_view, end_frame);
        view::set_alpha(&banner_view, 0.0);
    });

    spawn_local(async move {
        sleep(EXIT_DURATION).await;
        presentation.complete_dismissal();
    })
    .detach();
}

fn reconcile_presentations(
    state: &Rc<SnackbarHostState>,
    presentations: &[SnackbarPresentation<Held>],
) {
    let mut already_dismissing = Vec::new();
    let mut changed = false;

    {
        let mut rows = state.rows.borrow_mut();
        let before = rows.len();
        rows.retain(|row| presentations.iter().any(|p| p.id() == row.id));
        if rows.len() != before {
            changed = true;
        }

        for presentation in presentations {
            let id = presentation.id();
            if rows.iter().any(|row| row.id == id) {
                continue;
            }
            let leaf = state.renderer.render(build_apple_banner(presentation));
            let mounted = leaf.mount(&state.overlay_host);
            let message = presentation
                .message()
                .resolve(&state.env)
                .content()
                .snapshot()
                .to_plain();
            if let Some(announcer) = AccessibilityAnnouncer::from_env(&state.env) {
                announcer.announce_plain(message.as_str(), LiveRegion::Polite);
            } else {
                cocoa_ui::accessibility::announce(message.as_str(), false);
            }
            let dismiss_guard = presentation.dismissing().watch({
                let state = Rc::downgrade(state);
                let presentation = presentation.clone();
                move |ctx| {
                    if *ctx.value()
                        && let Some(state) = state.upgrade()
                    {
                        begin_row_exit(&state, id, presentation.clone());
                    }
                }
            });
            let dismissing_now = presentation.dismissing().snapshot();
            rows.push(AppleSnackbarRow {
                id,
                position: presentation.position(),
                mounted,
                dismissing: false,
                appeared: false,
                _dismiss_guard: dismiss_guard,
            });
            changed = true;
            if dismissing_now {
                already_dismissing.push((id, presentation.clone()));
            }
        }
    }

    if changed {
        layout_banners(state, true);
    }
    for (id, presentation) in already_dismissing {
        begin_row_exit(state, id, presentation);
    }
}

/// Realizes `Metadata<SnackbarHostConfig<Held>, Held>` as a transparent native
/// container overlaying floating Apple confirmation banners above its wrapped
/// content.
pub fn realize_snackbar_host_metadata(
    metadata: Metadata<SnackbarHostConfig<Held>, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, KitRect::ZERO);
    let host_view: &PlatformView = &host;
    let mounted_child = ctx.realize(metadata.content).mount(host_view);
    primary_content::forward(&host, mounted_child.view());

    let overlay_host = HostView::new(mtm, KitRect::ZERO);
    overlay_host.set_hit_test_handler(|_, _| HitTest::PassIfSelf);
    view::add_subview(host_view, &overlay_host);

    let state = Rc::new(SnackbarHostState {
        host: host.clone(),
        overlay_host,
        child: mounted_child,
        renderer: ctx.renderer(),
        env: ctx.env().clone(),
        direction: layout_direction(ctx.env()),
        rows: RefCell::new(Vec::new()),
    });

    host.set_layout_handler({
        let child = view::retain_base(state.child.view());
        let overlay = view::retain_base(&state.overlay_host);
        let state = Rc::clone(&state);
        move |view| {
            let bounds = view::bounds(view);
            view::set_frame(&child, bounds);
            view::set_frame(&overlay, bounds);
            layout_banners(&state, false);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        SnackbarHostSubView {
            state: Rc::clone(&state),
        },
    );

    leaf.keep(Rc::clone(&state));
    let presentations = metadata.value.presentations().clone();
    leaf.watch(&presentations, {
        let state = Rc::downgrade(&state);
        move |ctx| {
            if let Some(state) = state.upgrade() {
                reconcile_presentations(&state, ctx.value());
            }
        }
    });

    let initial = presentations.snapshot();
    if !initial.is_empty() {
        reconcile_presentations(&state, &initial);
    }

    leaf
}

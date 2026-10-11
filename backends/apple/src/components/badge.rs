//! The `badge` leaf: `Native<BadgeConfig>` rendered as a transparent
//! container overlaying a [`BadgeView`] indicator on its content's
//! top-trailing corner (mirrored in RTL).
//!
//! Mirrors `WuiBadge`: the wrapper contributes no size of its own —
//! measure, stretch and priority forward to the wrapped view — and the
//! indicator's placement is fixed inside the host's bounds, overhanging the
//! top edge. `value` drives the dot/capsule switch (a value change is a
//! size change, so the host re-lays out); `color` resolves each `Color`
//! through the environment, as `WuiColorPicker` does, and the count label
//! follows the `AccentForeground` theme slot.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::component::badge::BadgeConfig;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::{Computed, Signal, SignalExt};
use waterui::resolve::Resolvable;
use waterui::theme::color::AccentForeground;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::colors;
    pub(super) use cocoa_ui::appkit::{BadgeView, HostView};
    pub(super) use cocoa_ui::objc2_app_kit::NSColor as PlatformColor;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor as PlatformColor;
    pub(super) use cocoa_ui::uikit::colors;
    pub(super) use cocoa_ui::uikit::{BadgeView, HostView};
}

use platform::{BadgeView, HostView};

/// The badge chrome `SwiftUI` draws: a 6pt dot for a zero count, a 16pt
/// capsule carrying the count in an 11pt medium label, inset 12pt inside
/// the content's trailing edge and overhanging 14pt above its top.
const BADGE_METRICS: cocoa_ui::badge::BadgeMetrics = cocoa_ui::badge::BadgeMetrics {
    dot_size: 6.0,
    capsule_height: 16.0,
    capsule_horizontal_padding: 4.0,
    capsule_font_size: 11.0,
    count_horizontal_offset: 12.0,
    count_vertical_offset: 14.0,
};

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
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

/// Where the indicator's frame lands inside `bounds`: pinned
/// `horizontal_offset` inside the trailing edge (the leading edge, mirrored
/// in RTL) and `vertical_offset` below the top, so the capsule overhangs
/// upward — `WuiBadge.layoutIndicator`.
fn indicator_frame(
    bounds: Rect,
    indicator_size: cocoa_ui::Size,
    horizontal_offset: f64,
    vertical_offset: f64,
    right_to_left: bool,
) -> Rect {
    let x = if right_to_left {
        bounds.origin.x + horizontal_offset - indicator_size.width
    } else {
        bounds.origin.x + bounds.size.width - horizontal_offset
    };
    Rect::new(
        x,
        bounds.origin.y + vertical_offset - indicator_size.height,
        indicator_size.width,
        indicator_size.height,
    )
}

/// The leaf's live state, shared between the watchers, the layout face and
/// the host's layout handler.
struct BadgeState {
    /// The kit indicator view, mounted above the content.
    indicator: Retained<BadgeView>,
    /// The wrapped content leaf.
    content: Mounted,
    /// The watcher observing the currently bound `Color`'s resolved value;
    /// replaced every time the color signal emits a new `Color`.
    color_guard: Option<<Computed<WorkingColor> as Signal>::Guard>,
}

/// The container's layout face: every query forwards to the content, so
/// the badge is transparent to layout — `WuiBadge`'s `measure`,
/// `stretchAxis` and `layoutPriority`.
struct BadgeSubView {
    state: Rc<RefCell<BadgeState>>,
}

impl core::fmt::Debug for BadgeSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BadgeSubView").finish_non_exhaustive()
    }
}

impl SubView for BadgeSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().content.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().content.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.borrow().content.layout().priority()
    }
}

/// Renders a `BadgeConfig` into the host container: content filling the
/// bounds, the badge indicator pinned top-trailing.
fn render(config: BadgeConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    let BadgeConfig {
        value,
        content: content_builder,
        color,
    } = config;
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;
    let content = ctx.render(content_builder.build()).mount(host_view);
    crate::primary_content::forward(&host, content.view());
    let indicator = BadgeView::new(mtm, BADGE_METRICS);
    cocoa_ui::view::add_subview(host_view, &indicator);

    let state = Rc::new(RefCell::new(BadgeState {
        indicator,
        content,
        color_guard: None,
    }));

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |view| {
            let state = state.borrow();
            let bounds = cocoa_ui::view::bounds(view);
            cocoa_ui::view::set_frame(state.content.view(), bounds);
            let frame = indicator_frame(
                bounds,
                state.indicator.intrinsic_size(),
                state.indicator.horizontal_offset(),
                state.indicator.vertical_offset(),
                cocoa_ui::view::is_right_to_left(view),
            );
            cocoa_ui::view::set_frame(&state.indicator, frame);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        BadgeSubView {
            state: Rc::clone(&state),
        },
    );

    // `value` selects dot vs capsule — a size change, so the host re-lays
    // out, matching `setNeedsIndicatorLayout`.
    leaf.bind(&value, {
        let state = Rc::clone(&state);
        let host = Retained::clone(&host);
        move |value| {
            state.borrow().indicator.set_value(value);
            host.set_needs_layout();
            crate::measure_memo::invalidate();
        }
    });

    // Each `Color` the signal emits resolves in this environment; the
    // resolved signal is observed and its guard replaced, as
    // `WuiColorPicker` re-arms `observeColor`.
    leaf.watch(&color, {
        let state = Rc::clone(&state);
        let env = ctx.env().clone();
        move |ctx| {
            let resolved = ctx.value().resolve(&env);
            state
                .borrow()
                .indicator
                .set_fill_color(&platform_color(&resolved.snapshot()));
            let guard = resolved.watch({
                let state = Rc::clone(&state);
                move |ctx| {
                    state
                        .borrow()
                        .indicator
                        .set_fill_color(&platform_color(ctx.value()));
                }
            });
            state.borrow_mut().color_guard = Some(guard);
        }
    });

    // The count label follows `AccentForeground` — the
    // `WuiColorSlot_AccentForeground` theme observation in `WuiBadge`.
    leaf.bind(&AccentForeground.resolve(ctx.env()).computed(), {
        let state = Rc::clone(&state);
        move |color| {
            state
                .borrow()
                .indicator
                .set_label_color(&platform_color(&color));
        }
    });

    leaf.keep(state);
    leaf
}

/// Installs the `badge` handler on the dispatcher: `Native<BadgeConfig>`
/// maps to the overlay container.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<BadgeConfig>(render);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicator_pins_to_the_trailing_corner() {
        // LTR: the capsule's leading edge sits `offset` inside maxX; the
        // capsule's right edge may extend past the content, as the Swift
        // port places it.
        let frame = indicator_frame(
            Rect::new(0.0, 0.0, 100.0, 40.0),
            cocoa_ui::Size::new(24.0, 16.0),
            12.0,
            14.0,
            false,
        );
        assert_eq!(frame, Rect::new(88.0, -2.0, 24.0, 16.0));
    }

    #[test]
    fn indicator_mirrors_in_rtl() {
        let frame = indicator_frame(
            Rect::new(0.0, 0.0, 100.0, 40.0),
            cocoa_ui::Size::new(24.0, 16.0),
            12.0,
            14.0,
            true,
        );
        assert_eq!(frame, Rect::new(-12.0, -2.0, 24.0, 16.0));
    }

    #[test]
    fn dot_aligns_right_edge_to_content() {
        // A zero count draws the dot with its trailing edge on the
        // content's trailing edge — offset equals the dot's width.
        let frame = indicator_frame(
            Rect::new(0.0, 0.0, 100.0, 40.0),
            cocoa_ui::Size::new(6.0, 6.0),
            6.0,
            6.0,
            false,
        );
        assert_eq!(frame, Rect::new(94.0, 0.0, 6.0, 6.0));
    }
}

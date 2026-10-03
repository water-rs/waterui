//! The working-color leaf: `Native<WorkingColor>` rendered through the
//! kit's color-fill view — and `Native<Color>` alongside it, because the
//! deleted `WuiResolvedColorView.swift` also served `WuiColorView`, the
//! semantic-color twin whose `AnyResolvable` resolves through the
//! environment into a watched `Computed<WorkingColor>`.
//!
//! Mirrors `WuiColorViewBase`: the view is a greedy fill — it takes the
//! whole proposal on every axis and stretches both ways. A `Color`'s
//! resolution is scheme-aware, so every change lands as an imperative
//! `set_color` inside a watcher; a bare `WorkingColor` is already resolved
//! and applies once. No signal type crosses into `cocoa-ui`.

use cocoa_ui::Retained;
use waterui::graphics::color::{Color, WorkingColor};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::ColorView;
    pub(super) use cocoa_ui::appkit::colors;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::ColorView;
    pub(super) use cocoa_ui::uikit::colors;
}

use cocoa_ui::PlatformView;
use platform::ColorView;

/// The color view as its platform view, for the leaf.
fn as_view(view: &ColorView) -> &PlatformView {
    view
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object —
/// linear Display-P3 channels carried straight, values above `1.0` being
/// the color's HDR headroom already.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_ui_kit::UIColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object — the
/// `AppKit` variant, same straight channels.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_app_kit::NSColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// The color view's layout face: greedy on both axes, no intrinsic size —
/// `WuiGraphicsPrimitiveSizing.sizeThatFits` answers the proposal verbatim,
/// with unproposed axes measuring zero.
#[derive(Debug)]
struct ColorSubView;

impl SubView for ColorSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// A fresh color-fill leaf around `view`, the shape both handlers share.
fn color_leaf(mtm: cocoa_ui::MainThreadMarker) -> (Retained<ColorView>, NativeLeaf) {
    let view = ColorView::new(mtm);
    let leaf = NativeLeaf::new(as_view(&view), ColorSubView);
    (view, leaf)
}

/// Installs the handler on the dispatcher: `Native<Color>` resolves through
/// the environment into a watched color signal; `Native<WorkingColor>`
/// applies its already-resolved fill once.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Color>(|config, ctx| {
        let (view, mut leaf) = color_leaf(ctx.mtm());
        let resolved = config.resolve(ctx.env());
        leaf.bind(&resolved, move |color| {
            view.set_color(Some(&platform_color(&color)));
        });
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_echoes_the_proposal() {
        let view = ColorSubView;
        let dimensions = view.measure(ProposalSize::new(120.0, 45.0));
        assert_eq!(dimensions.size, Size::new(120.0, 45.0));
    }

    #[test]
    fn measure_reports_zero_on_unproposed_axes() {
        let view = ColorSubView;
        let dimensions = view.measure(ProposalSize::UNSPECIFIED);
        assert_eq!(dimensions.size, Size::new(0.0, 0.0));
        let partial = view.measure(ProposalSize::new(80.0, None));
        assert_eq!(partial.size, Size::new(80.0, 0.0));
    }

    #[test]
    fn stretches_both_axes_at_zero_priority() {
        let view = ColorSubView;
        assert_eq!(view.stretch_axis(), StretchAxis::Both);
        assert_eq!(view.priority(), 0);
        assert!(!view.is_empty());
    }
}

//! The `UIKit` progress indicator: a `UIProgressView` bar and a
//! `UIActivityIndicatorView` spinner in one container.
//!
//! `Progress` owns a plain `UIView` holding both indicators and swaps
//! their visibility by variant and determinacy: a linear, determinate
//! reading draws the bar; anything circular or indeterminate draws the
//! spinner — `UIKit` has no determinate ring.
//!
//! # Safety
//!
//! The `unsafe` here calls `objc2` bindings marked unsafe because `UIKit`
//! view APIs are main-thread only — which the [`MainThreadMarker`]
//! constructor argument and setter contracts guarantee.

use std::cell::Cell;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_ui_kit::{
    UIActivityIndicatorView, UIActivityIndicatorViewStyle, UIColor, UIProgressView, UIView,
    UIViewAutoresizing,
};

use crate::geometry::Size;
use crate::progress::ProgressVariant;

/// A progress indicator in a `UIView` container.
///
/// The container is what the leaf mounts and lays out; the indicators are
/// what the tint setters reach.
#[derive(Debug, Clone)]
pub struct Progress {
    container: Retained<UIView>,
    bar: Retained<UIProgressView>,
    spinner: Retained<UIActivityIndicatorView>,
    variant: Cell<ProgressVariant>,
    indeterminate: Cell<bool>,
}

impl Progress {
    /// A determinate linear indicator reporting `0..=1`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let bar = UIProgressView::new(mtm);
        bar.setAutoresizingMask(
            UIViewAutoresizing::FlexibleWidth
                | UIViewAutoresizing::FlexibleTopMargin
                | UIViewAutoresizing::FlexibleBottomMargin,
        );
        let spinner = UIActivityIndicatorView::initWithActivityIndicatorStyle(
            mtm.alloc(),
            UIActivityIndicatorViewStyle::Medium,
        );
        spinner.setHidesWhenStopped(true);
        spinner.setAutoresizingMask(
            UIViewAutoresizing::FlexibleTopMargin
                | UIViewAutoresizing::FlexibleBottomMargin
                | UIViewAutoresizing::FlexibleLeftMargin
                | UIViewAutoresizing::FlexibleRightMargin,
        );
        let container = UIView::new(mtm);
        container.addSubview(&bar);
        container.addSubview(&spinner);
        let this = Self {
            container,
            bar,
            spinner,
            variant: Cell::new(ProgressVariant::Linear),
            indeterminate: Cell::new(false),
        };
        this.update_indicators();
        this
    }

    /// The container view — what a leaf mounts.
    #[must_use]
    pub fn container(&self) -> &UIView {
        &self.container
    }

    /// Switches between the bar and the spinner.
    pub fn set_variant(&self, variant: ProgressVariant) {
        self.variant.set(variant);
        self.update_indicators();
    }

    /// Whether the indicator animates instead of reporting a value — an
    /// indeterminate linear reading draws the spinner — `UIKit` has no
    /// indeterminate bar.
    pub fn set_indeterminate(&self, indeterminate: bool) {
        self.indeterminate.set(indeterminate);
        self.update_indicators();
    }

    /// Sets the value the bar reports, optionally animating the fill.
    #[expect(clippy::cast_possible_truncation, reason = "UIProgressView speaks f32")]
    pub fn set_progress(&self, value: f64, animated: bool) {
        self.bar.setProgress_animated(value as f32, animated);
    }

    /// The fill tint, applied to whichever indicator is drawing.
    pub fn set_tint(&self, color: &UIColor) {
        self.bar.setProgressTintColor(Some(color));
        // SAFETY: `setColor:` on a live activity indicator.
        unsafe { self.spinner.setColor(Some(color)) };
    }

    /// The track tint behind the bar's fill; the spinner ignores it.
    pub fn set_track_tint(&self, color: &UIColor) {
        self.bar.setTrackTintColor(Some(color));
    }

    /// The indicator's intrinsic size — what a measure pass reports.
    /// `UIProgressView` has no intrinsic width of its own, so a linear
    /// indicator reports a zero width for its owner to floor.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        match self.variant.get() {
            ProgressVariant::Linear => Size::new(0.0, self.bar.intrinsicContentSize().height),
            ProgressVariant::Circular => self.spinner.intrinsicContentSize().into(),
        }
    }

    /// Shows the indicator the current variant and determinacy select.
    fn update_indicators(&self) {
        let spinning = self.variant.get() == ProgressVariant::Circular || self.indeterminate.get();
        self.bar.setHidden(spinning);
        if spinning {
            self.spinner.startAnimating();
        } else {
            self.spinner.stopAnimating();
        }
    }
}

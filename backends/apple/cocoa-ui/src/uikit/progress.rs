//! The `UIKit` progress indicator: a `UIProgressView` bar, a
//! `UIActivityIndicatorView` spinner and a determinate ring in one
//! container.
//!
//! `Progress` owns a plain `UIView` holding the three indicators and swaps
//! their visibility by variant and determinacy: a linear, determinate
//! reading draws the bar; a circular, determinate reading draws the ring;
//! anything indeterminate draws the spinner. `UIKit` has no determinate
//! ring control, so the ring is two `CAShapeLayer` arcs — a track circle
//! and a progress arc whose `strokeEnd` carries the value.
//!
//! # Safety
//!
//! The `unsafe` here calls `objc2` bindings marked unsafe because `UIKit`
//! view APIs are main-thread only — which the [`MainThreadMarker`]
//! constructor argument and setter contracts guarantee.

use std::cell::Cell;
use std::f64::consts::{FRAC_PI_2, TAU};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSNumber, NSNumberFormatter, NSNumberFormatterStyle};
use objc2_quartz_core::CATransaction;
use objc2_ui_kit::{
    NSObjectUIAccessibility, UIActivityIndicatorView, UIActivityIndicatorViewStyle, UIColor,
    UIProgressView, UIView, UIViewAutoresizing,
};

use crate::geometry::{Point, Rect, Size};
use crate::layer::{ShapeLayer, add_sublayer};
use crate::path::MutablePath;
use crate::progress::ProgressVariant;

/// The ring's stroke width as a fraction of its diameter — the spinner's
/// spoke weight at the same size.
const RING_LINE_FRACTION: f64 = 0.125;

/// How long a value change animates — `CALayer`'s implicit-animation
/// duration, which is also what `UIProgressView` animates over.
const RING_ANIMATION_SECONDS: f64 = 0.25;

/// A progress indicator in a `UIView` container.
///
/// The container is what the leaf mounts and lays out; the indicators are
/// what the tint setters reach.
#[derive(Debug, Clone)]
pub struct Progress {
    container: Retained<UIView>,
    bar: Retained<UIProgressView>,
    spinner: Retained<UIActivityIndicatorView>,
    ring: Ring,
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
        let ring = Ring::new(mtm, spinner.intrinsicContentSize().width);
        let container = UIView::new(mtm);
        container.addSubview(&bar);
        container.addSubview(&spinner);
        container.addSubview(&ring.view);
        let this = Self {
            container,
            bar,
            spinner,
            ring,
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

    /// Sets the value the bar and the ring report, optionally animating the
    /// fill.
    #[expect(clippy::cast_possible_truncation, reason = "UIProgressView speaks f32")]
    pub fn set_progress(&self, value: f64, animated: bool) {
        self.bar.setProgress_animated(value as f32, animated);
        self.ring.set_value(value, animated);
    }

    /// The fill tint, applied to whichever indicator is drawing.
    pub fn set_tint(&self, color: &UIColor) {
        self.bar.setProgressTintColor(Some(color));
        self.ring.set_tint(color);
        // SAFETY: `setColor:` on a live activity indicator.
        unsafe { self.spinner.setColor(Some(color)) };
    }

    /// The track tint behind the bar's and the ring's fill; the spinner
    /// ignores it.
    pub fn set_track_tint(&self, color: &UIColor) {
        self.bar.setTrackTintColor(Some(color));
        self.ring.set_track_tint(color);
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
        let indeterminate = self.indeterminate.get();
        let circular = self.variant.get() == ProgressVariant::Circular;
        self.bar.setHidden(circular || indeterminate);
        self.ring.view.setHidden(!circular || indeterminate);
        if indeterminate {
            self.spinner.startAnimating();
        } else {
            self.spinner.stopAnimating();
        }
    }
}

/// The determinate ring: a track circle under a progress arc, both
/// `CAShapeLayer` strokes of one circle path that starts at twelve o'clock
/// and runs clockwise, so `strokeEnd` is the fraction complete.
#[derive(Debug, Clone)]
struct Ring {
    view: Retained<UIView>,
    arc: ShapeLayerHandle,
    track: ShapeLayerHandle,
}

/// A shared handle to a [`ShapeLayer`], so the `Clone` indicator can hand
/// the same layers to every copy.
type ShapeLayerHandle = std::rc::Rc<ShapeLayer>;

impl Ring {
    /// A ring of `diameter` points, empty and untinted.
    fn new(mtm: MainThreadMarker, diameter: f64) -> Self {
        let view = UIView::new(mtm);
        view.setFrame(Rect::new(0.0, 0.0, diameter, diameter).into());
        view.setAutoresizingMask(
            UIViewAutoresizing::FlexibleTopMargin
                | UIViewAutoresizing::FlexibleBottomMargin
                | UIViewAutoresizing::FlexibleLeftMargin
                | UIViewAutoresizing::FlexibleRightMargin,
        );
        let line_width = diameter * RING_LINE_FRACTION;
        let center = Point::new(diameter / 2.0, diameter / 2.0);
        let circle = MutablePath::new();
        // Core Graphics measures `clockwise` in an unflipped space; a
        // `UIView`'s y axis points down, so increasing angles turn
        // clockwise on screen with `clockwise: false`.
        circle.arc(
            center,
            (diameter - line_width) / 2.0,
            -FRAC_PI_2,
            TAU - FRAC_PI_2,
            false,
        );
        let layer = view.layer();
        let make = || {
            let shape = ShapeLayer::new();
            shape.set_frame(Rect::new(0.0, 0.0, diameter, diameter));
            shape.set_path(Some(circle.path()));
            shape.set_fill(None);
            shape.set_line_width(line_width);
            shape.set_line_cap_butt();
            add_sublayer(&layer, shape.layer());
            std::rc::Rc::new(shape)
        };
        let track = make();
        let arc = make();
        arc.set_stroke_end(0.0);

        view.setIsAccessibilityElement(true, mtm);
        let this = Self { view, arc, track };
        this.set_accessibility_value(0.0);
        this
    }

    /// Moves the arc's end to `value` (`0..=1`), animating the sweep over
    /// [`RING_ANIMATION_SECONDS`] or jumping, as `UIProgressView` does.
    fn set_value(&self, value: f64, animated: bool) {
        CATransaction::begin();
        CATransaction::setDisableActions(!animated);
        if animated {
            CATransaction::setAnimationDuration(RING_ANIMATION_SECONDS);
        }
        self.arc.set_stroke_end(value);
        CATransaction::commit();
        self.set_accessibility_value(value);
    }

    fn set_tint(&self, color: &UIColor) {
        // SAFETY: `CGColor` on a live color.
        let cg = unsafe { color.CGColor() };
        self.arc.set_stroke(Some(&cg));
    }

    fn set_track_tint(&self, color: &UIColor) {
        // SAFETY: `CGColor` on a live color.
        let cg = unsafe { color.CGColor() };
        self.track.set_stroke(Some(&cg));
    }

    /// Reads the value out as a localized percentage, the way
    /// `UIProgressView` does.
    fn set_accessibility_value(&self, value: f64) {
        let mtm = MainThreadMarker::from(&*self.view);
        let percent = NSNumberFormatter::localizedStringFromNumber_numberStyle(
            &NSNumber::new_f64(value),
            NSNumberFormatterStyle::PercentStyle,
        );
        self.view.setAccessibilityValue(Some(&percent), mtm);
    }
}

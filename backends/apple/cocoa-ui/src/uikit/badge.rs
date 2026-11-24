//! The `UIKit` badge indicator: hidden, a 6pt dot, or a 16pt-high capsule
//! carrying a count or short text, drawn on a `UIView`.
//!
//! The view draws and measures itself — dot, capsule and the 11pt medium
//! count/text label — and reports the placement offsets a container uses to pin
//! it to a content view's top-trailing corner. It never takes hits: the
//! indicator announces itself through accessibility when carrying text or a
//! count, and stays transparent to input.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass, forwards to `UIView`'s
//! own implementations of the methods it overrides, loads attribute-name
//! constants the platform exports, and calls `objc2` bindings marked unsafe
//! because `UIKit` view APIs are main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee.

use std::cell::RefCell;
use std::fmt;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_core_graphics::{CGContext, CGPath};
use objc2_foundation::{
    NSAttributedStringKey, NSDictionary, NSObjectProtocol, NSRect, NSSize, NSString,
};

/// The attribute dictionary `sizeWithAttributes:`/`drawInRect:` take.
type TextAttributes = NSDictionary<NSAttributedStringKey, AnyObject>;
use objc2_ui_kit::{
    NSFontAttributeName, NSForegroundColorAttributeName, NSObjectUIAccessibility, NSStringDrawing,
    UIColor, UIFont, UIGraphicsGetCurrentContext, UIView,
};

use crate::badge::{BadgeDisplay, BadgeMetrics};
use crate::font;
use crate::geometry::Size;

/// A [`BadgeView`]'s state: the presentation display and the two paints it draws with.
pub struct BadgeViewIvars {
    /// Current indicator display (`Hidden`, `Dot`, or `Text`).
    display: RefCell<BadgeDisplay>,
    /// The capsule/dot fill.
    fill_color: RefCell<Retained<UIColor>>,
    /// The label's color.
    label_color: RefCell<Retained<UIColor>>,
    /// The geometry the indicator draws at.
    metrics: BadgeMetrics,
}

impl fmt::Debug for BadgeViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BadgeViewIvars")
            .field("display", &*self.display.borrow())
            .field("fill_color", &self.fill_color.borrow())
            .field("label_color", &self.label_color.borrow())
            .field("metrics", &self.metrics)
            .finish()
    }
}

/// The attributes the label measures and draws with — the medium
/// system face at the badge size, in the label color.
fn text_attributes(label_color: &UIColor, font: &UIFont) -> Retained<TextAttributes> {
    // SAFETY: both attribute names are statics the platform exports; the
    // values are the `UIFont`/`UIColor` each documents.
    unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[font.as_ref(), label_color.as_ref()],
        )
    }
}

impl BadgeViewIvars {
    /// The attributes for the current label color and font.
    fn text_attributes(&self, mtm: MainThreadMarker) -> Retained<TextAttributes> {
        text_attributes(
            &self.label_color.borrow(),
            &font::system(mtm, self.metrics.capsule_font_size, font::weight::MEDIUM),
        )
    }

    /// The width `text` measures under the current attributes.
    fn text_size(&self, text: &str, mtm: MainThreadMarker) -> NSSize {
        // SAFETY: `sizeWithAttributes:` is `NSStringDrawing`'s documented
        // measure; the dictionary carries the documented value types.
        unsafe { NSString::from_str(text).sizeWithAttributes(Some(&self.text_attributes(mtm))) }
    }
}

define_class!(
    // SAFETY: `UIView`'s designated initializer is `initWithFrame:`, which
    // `BadgeView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UIView))]
    #[name = "CocoaUiBadgeView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = BadgeViewIvars]
    #[derive(Debug)]
    /// A `UIView` drawing a badge indicator: hidden, a 6pt dot, or a
    /// 16pt-high capsule carrying a count or short text.
    pub struct BadgeView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for BadgeView {}

    impl BadgeView {
        // SAFETY: see the module safety note.
        #[unsafe(method(drawRect:))]
        fn draw_rect_override(&self, _rect: NSRect) {
            let mtm = MainThreadMarker::from(self);
            let rect = self.bounds();
            let Some(context) = UIGraphicsGetCurrentContext() else {
                return;
            };
            let display = self.ivars().display.borrow().clone();
            let label = match display {
                BadgeDisplay::Hidden => return,
                BadgeDisplay::Dot => {
                    let fill = self.ivars().fill_color.borrow();
                    // SAFETY: `CGColor` is a documented accessor on a live color;
                    // the returned object is used only within this draw pass.
                    let cg = unsafe { fill.CGColor() };
                    CGContext::set_fill_color_with_color(Some(&context), Some(&cg));
                    CGContext::fill_ellipse_in_rect(Some(&context), rect);
                    return;
                }
                BadgeDisplay::Text(label) => label,
            };
            let fill = self.ivars().fill_color.borrow();
            // SAFETY: `CGColor` is a documented accessor on a live color;
            // the returned object is used only within this draw pass.
            let cg = unsafe { fill.CGColor() };
            CGContext::set_fill_color_with_color(Some(&context), Some(&cg));
            // SAFETY: `rect` is live bounds data and a null transform is
            // valid; the returned path is a rounded rect with a
            // `height/2` corner — the capsule.
            let capsule = unsafe {
                CGPath::with_rounded_rect(
                    rect,
                    rect.size.height / 2.0,
                    rect.size.height / 2.0,
                    std::ptr::null(),
                )
            };
            CGContext::add_path(Some(&context), Some(&capsule));
            CGContext::fill_path(Some(&context));
            let text = NSString::from_str(&label);
            let attributes = self.ivars().text_attributes(mtm);
            // SAFETY: `sizeWithAttributes:` is `NSStringDrawing`'s
            // documented measure under the same attributes the draw uses.
            let text_size = unsafe { text.sizeWithAttributes(Some(&attributes)) };
            let text_rect = CGRect {
                origin: objc2_core_foundation::CGPoint {
                    x: rect.origin.x + (rect.size.width - text_size.width) / 2.0,
                    y: rect.origin.y + (rect.size.height - text_size.height) / 2.0,
                },
                size: text_size,
            };
            // SAFETY: `drawInRect:withAttributes:` renders the string in
            // the current context; the dictionary carries the documented
            // value types.
            unsafe { text.drawInRect_withAttributes(text_rect, Some(&attributes)) };
        }
    }
);

impl BadgeView {
    /// A badge indicator painting `systemRed` under white text, drawing
    /// at `metrics` (initially hidden).
    #[must_use]
    pub fn new(mtm: MainThreadMarker, metrics: BadgeMetrics) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BadgeViewIvars {
            display: RefCell::new(BadgeDisplay::Hidden),
            fill_color: RefCell::new(UIColor::systemRedColor()),
            label_color: RefCell::new(UIColor::whiteColor()),
            metrics,
        });
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        // The indicator is decoration: it stays out of the responder chain
        // and paints on a clear background.
        view.setUserInteractionEnabled(false);
        view.setBackgroundColor(Some(&UIColor::clearColor()));
        view.setHidden(true);
        view.update_accessibility();
        view
    }

    /// Sets the presentation carried by this badge indicator.
    pub fn set_display(&self, display: BadgeDisplay) {
        if display == *self.ivars().display.borrow() {
            return;
        }
        let hidden = matches!(display, BadgeDisplay::Hidden);
        *self.ivars().display.borrow_mut() = display;
        self.setHidden(hidden);
        self.update_accessibility();
        self.setNeedsDisplay();
    }

    /// The current presentation.
    #[must_use]
    pub fn display(&self) -> BadgeDisplay {
        self.ivars().display.borrow().clone()
    }

    /// The capsule/dot fill color.
    pub fn set_fill_color(&self, color: &UIColor) {
        *self.ivars().fill_color.borrow_mut() = color.into();
        self.setNeedsDisplay();
    }

    /// The badge label's color.
    pub fn set_label_color(&self, color: &UIColor) {
        *self.ivars().label_color.borrow_mut() = color.into();
        self.setNeedsDisplay();
    }

    /// The indicator's drawn extent: zero when hidden, the dot, or the
    /// capsule sized to the label text plus padding (never narrower than it
    /// is tall).
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let metrics = self.ivars().metrics;
        match &*self.ivars().display.borrow() {
            BadgeDisplay::Hidden => Size::ZERO,
            BadgeDisplay::Dot => Size::new(metrics.dot_size, metrics.dot_size),
            BadgeDisplay::Text(label) => {
                let text = self.ivars().text_size(label, MainThreadMarker::from(self));
                let width = metrics
                    .capsule_horizontal_padding
                    .mul_add(2.0, text.width.ceil());
                Size::new(width.max(metrics.capsule_height), metrics.capsule_height)
            }
        }
    }

    /// The inset from the content's trailing edge to the indicator's
    /// leading edge (its own width for the dot, which is right-aligned on
    /// the offset).
    #[must_use]
    pub fn horizontal_offset(&self) -> f64 {
        match *self.ivars().display.borrow() {
            BadgeDisplay::Dot => self.ivars().metrics.dot_size,
            BadgeDisplay::Hidden | BadgeDisplay::Text(_) => {
                self.ivars().metrics.count_horizontal_offset
            }
        }
    }

    /// The distance below the content's top edge the indicator's top sits —
    /// the capsule overhangs upward.
    #[must_use]
    pub fn vertical_offset(&self) -> f64 {
        match *self.ivars().display.borrow() {
            BadgeDisplay::Dot => self.ivars().metrics.dot_size,
            BadgeDisplay::Hidden | BadgeDisplay::Text(_) => {
                self.ivars().metrics.count_vertical_offset
            }
        }
    }

    /// Announces text/count badges; hidden and dot badges stay out of the
    /// accessibility tree.
    fn update_accessibility(&self) {
        let mtm = MainThreadMarker::from(self);
        let label = match &*self.ivars().display.borrow() {
            BadgeDisplay::Hidden | BadgeDisplay::Dot => None,
            BadgeDisplay::Text(text) => Some(NSString::from_str(text)),
        };
        self.setIsAccessibilityElement(label.is_some(), mtm);
        self.setAccessibilityLabel(label.as_deref(), mtm);
    }
}

//! The `AppKit` badge indicator: a count capsule (or a bare dot when the
//! count is zero) drawn on a layer-free `NSView`.
//!
//! The view draws and measures itself — dot, capsule and the 11pt medium
//! count label — and reports the placement offsets a container uses to pin
//! it to a content view's top-trailing corner. It never takes hits: the
//! indicator announces itself through accessibility but stays transparent
//! to input.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSView` subclass, forwards to `NSView`'s
//! own implementations of the methods it overrides, loads attribute-name
//! constants the platform exports, and calls `objc2` bindings marked unsafe
//! because `AppKit` view APIs are main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee.

use std::cell::{Cell, RefCell};
use std::fmt;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSGraphicsContext, NSStringDrawing, NSView,
};
use objc2_core_foundation::CGRect;
use objc2_core_graphics::{CGContext, CGPath};
use objc2_foundation::{
    NSAttributedStringKey, NSDictionary, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

/// The attribute dictionary `sizeWithAttributes:`/`drawInRect:` take.
type TextAttributes = NSDictionary<NSAttributedStringKey, AnyObject>;

use crate::font;
use crate::geometry::Size;

/// A [`BadgeView`]'s state: the count and the two paints it draws with.
pub struct BadgeViewIvars {
    /// The count; `0` draws the bare dot.
    value: Cell<i32>,
    /// The capsule/dot fill.
    fill_color: RefCell<Retained<NSColor>>,
    /// The count label's color.
    label_color: RefCell<Retained<NSColor>>,
    /// The geometry the indicator draws at.
    metrics: crate::badge::BadgeMetrics,
}

impl fmt::Debug for BadgeViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BadgeViewIvars")
            .field("value", &self.value.get())
            .field("fill_color", &self.fill_color.borrow())
            .field("label_color", &self.label_color.borrow())
            .field("metrics", &self.metrics)
            .finish()
    }
}

/// The attributes the count label measures and draws with — the medium
/// system face at the badge size, in the label color.
fn text_attributes(label_color: &NSColor, font: &NSFont) -> Retained<TextAttributes> {
    // SAFETY: both attribute names are statics the platform exports; the
    // values are the `NSFont`/`NSColor` each documents.
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

    /// The count rendered as the label text.
    fn value_text(&self) -> Retained<NSString> {
        NSString::from_str(&self.value.get().to_string())
    }

    /// The width the count text measures under the current attributes.
    fn text_size(&self, mtm: MainThreadMarker) -> NSSize {
        // SAFETY: `sizeWithAttributes:` is `NSStringDrawing`'s documented
        // measure; the dictionary carries the documented value types.
        unsafe {
            self.value_text()
                .sizeWithAttributes(Some(&self.text_attributes(mtm)))
        }
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `BadgeView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiBadgeView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = BadgeViewIvars]
    #[derive(Debug)]
    /// An `NSView` drawing a badge indicator: a dot for a zero count, a
    /// capsule carrying the count otherwise.
    ///
    /// Its coordinates are flipped: the origin is the top-left corner and
    /// `y` grows downward, matching the rest of the kit.
    pub struct BadgeView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for BadgeView {}

    // SAFETY: `NSAccessibility` asks an `NSView` subclass to declare its
    // element-ness and label; both are stored on the view itself.
    unsafe impl NSAccessibility for BadgeView {}

    impl BadgeView {
        // SAFETY: see the module safety note.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(hitTest:))]
        fn hit_test_override(&self, _point: NSPoint) -> Option<Retained<NSView>> {
            // The indicator is decoration: hits fall through to whatever
            // lies beneath it.
            None
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(drawRect:))]
        fn draw_rect_override(&self, _dirty_rect: NSRect) {
            let mtm = MainThreadMarker::from(self);
            let rect = self.bounds();
            let Some(context) = NSGraphicsContext::currentContext() else {
                return;
            };
            let context = context.CGContext();
            let fill = self.ivars().fill_color.borrow();
            CGContext::set_fill_color_with_color(Some(&context), Some(&fill.CGColor()));
            if self.ivars().value.get() == 0 {
                CGContext::fill_ellipse_in_rect(Some(&context), rect);
                return;
            }
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
            let text = self.ivars().value_text();
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
    /// A badge indicator painting `systemRed` under a white count, drawing
    /// at `metrics`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, metrics: crate::badge::BadgeMetrics) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BadgeViewIvars {
            value: Cell::new(0),
            fill_color: RefCell::new(NSColor::systemRedColor()),
            label_color: RefCell::new(NSColor::whiteColor()),
            metrics,
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        view.update_accessibility();
        view
    }

    /// The count the indicator carries; `0` draws the bare dot.
    pub fn set_value(&self, value: i32) {
        if value == self.ivars().value.get() {
            return;
        }
        self.ivars().value.set(value);
        self.update_accessibility();
        self.setNeedsDisplay(true);
    }

    /// The current count.
    #[must_use]
    pub fn value(&self) -> i32 {
        self.ivars().value.get()
    }

    /// The capsule/dot fill color.
    pub fn set_fill_color(&self, color: &NSColor) {
        *self.ivars().fill_color.borrow_mut() = color.into();
        self.setNeedsDisplay(true);
    }

    /// The count label's color.
    pub fn set_label_color(&self, color: &NSColor) {
        *self.ivars().label_color.borrow_mut() = color.into();
        self.setNeedsDisplay(true);
    }

    /// The indicator's drawn extent: the dot, or the capsule sized to the
    /// count text plus padding (never narrower than it is tall).
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let metrics = self.ivars().metrics;
        if self.ivars().value.get() == 0 {
            return Size::new(metrics.dot_size, metrics.dot_size);
        }
        let text = self.ivars().text_size(MainThreadMarker::from(self));
        let width = metrics
            .capsule_horizontal_padding
            .mul_add(2.0, text.width.ceil());
        Size::new(width.max(metrics.capsule_height), metrics.capsule_height)
    }

    /// The inset from the content's trailing edge to the indicator's
    /// leading edge (its own width for the dot, which is right-aligned on
    /// the offset).
    #[must_use]
    pub fn horizontal_offset(&self) -> f64 {
        if self.ivars().value.get() == 0 {
            self.ivars().metrics.dot_size
        } else {
            self.ivars().metrics.count_horizontal_offset
        }
    }

    /// The distance below the content's top edge the indicator's top sits —
    /// the capsule overhangs upward.
    #[must_use]
    pub fn vertical_offset(&self) -> f64 {
        if self.ivars().value.get() == 0 {
            self.ivars().metrics.dot_size
        } else {
            self.ivars().metrics.count_vertical_offset
        }
    }

    /// Announces the count when there is one; the bare dot stays out of the
    /// accessibility tree.
    fn update_accessibility(&self) {
        let value = self.ivars().value.get();
        self.setAccessibilityElement(value != 0);
        let label = (value != 0).then(|| NSString::from_str(&value.to_string()));
        self.setAccessibilityLabel(label.as_deref());
    }
}

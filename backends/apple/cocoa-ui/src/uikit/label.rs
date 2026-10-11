//! The `UIKit` label: a `UILabel` configured for wrapped multi-line text.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UILabel` subclass, forwards to `UILabel`'s
//! own implementations of the methods it overrides, and calls `objc2`
//! bindings marked unsafe because `UIKit` text APIs are main-thread only —
//! which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::cell::{Cell, RefCell};
use std::fmt;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSAttributedString, NSObjectProtocol, NSString};
use objc2_ui_kit::{NSLineBreakMode, NSTextAlignment, UILabel, UITraitEnvironment};

use crate::callback::guarded;
use crate::text::{TextMetrics, WrapWidth};

/// The text and line limit a [`Label`] measures with.
///
/// `UILabel`'s copy of the attributed string folds break mode in, so
/// measurement keeps the caller's own string.
pub struct LabelIvars {
    source_text: RefCell<Option<Retained<NSAttributedString>>>,
    /// Maximum visible lines; `0` wraps unbounded.
    line_limit: Cell<usize>,
    /// The alignment last requested through `set_text_alignment`.
    text_alignment: Cell<NSTextAlignment>,
    /// `bounds.width` as of the last `layoutSubviews` propagation.
    reported_width: Cell<f64>,
}

impl fmt::Debug for LabelIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LabelIvars")
            .field("source_text", &self.source_text.borrow().is_some())
            .field("line_limit", &self.line_limit.get())
            .field("text_alignment", &self.text_alignment.get())
            .field("reported_width", &self.reported_width.get())
            .finish()
    }
}

define_class!(
    // SAFETY: `UILabel`'s designated initializer is `initWithFrame:`, which
    // `Label::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UILabel))]
    #[name = "CocoaUiLabel"]
    #[thread_kind = MainThreadOnly]
    #[ivars = LabelIvars]
    #[derive(Debug)]
    /// A wrapped multi-line `UILabel` used as a text leaf.
    pub struct Label;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UILabel` subclass.
    unsafe impl NSObjectProtocol for Label {}

    impl Label {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSString>> {
            guarded("Label accessibilityValue", || {
                // SAFETY: see the module safety note.
                let value: Option<Retained<NSString>> =
                    unsafe { msg_send![super(self), accessibilityValue] };
                // Bidirectional control characters exist only to shape the
                // rendering; reading them aloud is noise.
                value.map(|text| {
                    let stripped = crate::text::strip_bidi_controls(&text.to_string());
                    NSString::from_str(&stripped)
                })
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("Label layoutSubviews", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                // `preferredMaxLayoutWidth` is the width text wraps at; a
                // new bounds width changes the label's intrinsic height.
                let width = self.bounds().size.width;
                if width > 0.0
                    && self.ivars().reported_width.replace(width).to_bits() != width.to_bits()
                {
                    self.setPreferredMaxLayoutWidth(width);
                    self.invalidate_layout();
                }
            });
        }
    }
);

impl Label {
    /// A wrapped text label, measuring and drawing `attributed` text.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(LabelIvars {
            source_text: RefCell::new(None),
            line_limit: Cell::new(0),
            text_alignment: Cell::new(NSTextAlignment::Natural),
            reported_width: Cell::new(0.0),
        });
        // SAFETY: `initWithFrame:` is `UILabel`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setNumberOfLines(0);
        this.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        this
    }

    /// Replaces the text the label draws, and reports that its size changed.
    pub fn set_attributed_text(&self, text: &NSAttributedString) {
        self.ivars().source_text.replace(Some(text.into()));
        self.setAttributedText(Some(text));
        // `setAttributedText` re-derives `lineBreakMode` and `textAlignment`
        // from the paragraph style inside the attributed string (word wrap,
        // natural alignment), clobbering what `set_line_limit` and
        // `set_text_alignment` configured — reapply them so the label's own
        // settings win regardless of call order.
        self.apply_line_limit();
        self.setTextAlignment(self.ivars().text_alignment.get());
        self.invalidate_layout();
    }

    /// The attributed string measurement runs against: the caller's own
    /// string, not the label's break-mode-adjusted copy.
    #[must_use]
    pub fn source_text(&self) -> Option<Retained<NSAttributedString>> {
        self.ivars().source_text.borrow().clone()
    }

    /// Limits the label to `limit` lines; `0` removes any limit and wraps.
    ///
    /// Truncation happens on the last visible line's tail.
    pub fn set_line_limit(&self, limit: usize) {
        self.ivars().line_limit.set(limit);
        self.apply_line_limit();
        self.invalidate_layout();
    }

    /// Applies `line_limit` to `numberOfLines` and `lineBreakMode`.
    fn apply_line_limit(&self) {
        let limit = self.ivars().line_limit.get();
        if limit == 0 {
            self.setNumberOfLines(0);
            self.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        } else {
            self.setNumberOfLines(limit.cast_signed());
            self.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        }
    }

    /// How the text aligns within the label's width.
    pub fn set_text_alignment(&self, alignment: NSTextAlignment) {
        self.ivars().text_alignment.set(alignment);
        self.setTextAlignment(alignment);
        self.invalidate_layout();
    }

    /// The width the cell needs to draw the text unclipped — its fitting
    /// size, wider than the bare text bounds by the label's insets.
    #[must_use]
    pub fn fitting_width(&self) -> f64 {
        self.intrinsicContentSize().width
    }

    /// Measures the label's current text under `wrap` at the scale of the
    /// screen the label draws on.
    #[must_use]
    pub fn measure(&self, wrap: WrapWidth) -> TextMetrics {
        let Some(text) = self.source_text() else {
            return TextMetrics {
                size: crate::geometry::Size::ZERO,
                first_baseline: None,
                last_baseline: None,
            };
        };
        crate::text::measure(
            MainThreadMarker::from(self),
            &text,
            wrap,
            self.ivars().line_limit.get(),
            self.display_scale(),
        )
    }

    /// Device pixels per point where the label is drawn, from its trait
    /// collection.
    #[must_use]
    pub fn display_scale(&self) -> f64 {
        // SAFETY: see the module safety note.
        let scale = unsafe { self.traitCollection().displayScale() };
        if scale > 0.0 { scale } else { 1.0 }
    }

    /// The label's intrinsic size changed: invalidate it and walk the
    /// superview chain so every ancestor re-runs layout.
    fn invalidate_layout(&self) {
        self.invalidateIntrinsicContentSize();
        crate::view::invalidate_layout(self);
    }
}

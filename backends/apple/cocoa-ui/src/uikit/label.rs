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
use std::ops::Range;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_foundation::{NSArray, NSAttributedString, NSObjectProtocol, NSString, NSUnionRect};
use objc2_ui_kit::{
    NSLineBreakMode, NSObjectUIAccessibility, NSTextAlignment,
    UIAccessibilityConvertFrameToScreenCoordinates, UIAccessibilityElement,
    UIAccessibilityLayoutChangedNotification, UIAccessibilityPostNotification,
    UIAccessibilityTraitLink, UIAccessibilityTraits, UILabel, UITraitEnvironment,
};

use crate::callback::guarded;
use crate::geometry::{Point, Rect};
use crate::text::{Layout, SpanLink, TextMetrics, WrapWidth};

/// Runs after each text replacement, with the new plain string.
type TextHandler = Rc<dyn Fn(&str)>;

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
    /// Called with the new string after each replacement.
    text_handler: RefCell<Option<TextHandler>>,
    /// The accessibility children the label answers with while its text has
    /// tap spans — the full-text element, then one `Link` per span — or
    /// `None`, when the label is an ordinary accessibility element.
    link_elements: RefCell<Option<Retained<NSArray<AnyObject>>>>,
}

impl fmt::Debug for LabelIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LabelIvars")
            .field("source_text", &self.source_text.borrow().is_some())
            .field("line_limit", &self.line_limit.get())
            .field("text_alignment", &self.text_alignment.get())
            .field("reported_width", &self.reported_width.get())
            .field("text_handler", &self.text_handler.borrow().is_some())
            .field("link_elements", &self.link_elements.borrow().is_some())
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
        // While the text has tap spans the label is a container — its
        // full text is the first child — so the answer overrides whatever
        // an accessibility modifier set.
        // SAFETY: see the module safety note.
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            guarded("Label isAccessibilityElement", || {
                self.ivars().link_elements.borrow().is_none()
                    // SAFETY: see the module safety note.
                    && unsafe { msg_send![super(self), isAccessibilityElement] }
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(accessibilityElements))]
        fn accessibility_elements(&self) -> Option<Retained<NSArray<AnyObject>>> {
            guarded("Label accessibilityElements", || {
                self.ivars().link_elements.borrow().clone().or_else(|| {
                    // SAFETY: see the module safety note.
                    unsafe { msg_send![super(self), accessibilityElements] }
                })
            })
        }

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

        // `UILabel` vertically centers the text inside the rect it is
        // handed; a leaf's frame is the layout answer, whose ink anchors
        // at the top the way `NSTextFieldCell` draws an oversized frame —
        // so the text rect is `super`'s answer re-anchored at the top of
        // the bounds.
        // SAFETY: see the module safety note.
        #[unsafe(method(textRectForBounds:limitedToNumberOfLines:))]
        fn text_rect_for_bounds(&self, bounds: CGRect, number_of_lines: isize) -> CGRect {
            guarded("Label textRectForBounds:limitedToNumberOfLines:", || {
                // SAFETY: see the module safety note.
                let mut rect: CGRect = unsafe {
                    msg_send![
                        super(self),
                        textRectForBounds: bounds,
                        limitedToNumberOfLines: number_of_lines
                    ]
                };
                rect.origin.y = bounds.origin.y;
                rect
            })
        }

        // `drawTextInRect:` receives the bounds; hand `super` the rect the
        // text actually occupies, anchored at its top — a natural-height
        // rect leaves no slack for `UILabel`'s vertical centering to show
        // up in.
        // SAFETY: see the module safety note.
        #[unsafe(method(drawTextInRect:))]
        fn draw_text_in_rect(&self, rect: CGRect) {
            guarded("Label drawTextInRect:", || {
                let text_rect =
                    self.textRectForBounds_limitedToNumberOfLines(rect, self.numberOfLines());
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), drawTextInRect: text_rect] };
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
            text_handler: RefCell::new(None),
            link_elements: RefCell::new(None),
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
        let handler = self.ivars().text_handler.borrow().clone();
        if let Some(handler) = handler {
            let plain = text.string().to_string();
            handler(&plain);
        }
    }

    /// Runs `handler` with the new string after each [`Self::set_attributed_text`].
    pub fn set_text_handler(&self, handler: impl Fn(&str) + 'static) {
        self.ivars().text_handler.replace(Some(Rc::new(handler)));
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

    /// The rect the label's text is drawn in, in bounds coordinates — the
    /// same answer `drawTextInRect:` works from.
    fn text_rect(&self) -> CGRect {
        self.textRectForBounds_limitedToNumberOfLines(self.bounds(), self.numberOfLines())
    }

    /// The laid-out stack a hit test runs through: container width is the
    /// text rect's, paragraph alignment the label's own.
    fn layout(&self) -> Option<Layout> {
        let text = self.source_text()?;
        Some(Layout::new(
            MainThreadMarker::from(self),
            &text,
            self.text_rect().size.width,
            self.ivars().text_alignment.get(),
            self.numberOfLines().cast_unsigned(),
        ))
    }

    /// The UTF-16 character index a point in bounds coordinates hits —
    /// `None` when it lands outside the text.
    #[must_use]
    pub fn character_index(&self, point: Point) -> Option<usize> {
        let origin = self.text_rect().origin;
        self.layout()?
            .character_index(CGPoint::new(point.x - origin.x, point.y - origin.y))
    }

    /// The rectangles a UTF-16 `range` covers in bounds coordinates — one
    /// per line fragment the range crosses.
    #[must_use]
    pub fn rects(&self, range: Range<usize>) -> Vec<Rect> {
        let Some(layout) = self.layout() else {
            return Vec::new();
        };
        let origin = self.text_rect().origin;
        layout
            .rects(range)
            .into_iter()
            .map(|rect| {
                Rect::new(
                    rect.origin.x + origin.x,
                    rect.origin.y + origin.y,
                    rect.size.width,
                    rect.size.height,
                )
            })
            .collect()
    }

    /// Makes the label an accessibility container of one element carrying
    /// its full text — label, value, traits and frame read from the label —
    /// followed by one `Link` element per span: its label the span's text,
    /// its frame the span's drawn rectangles, its activation the span's
    /// action. An empty list makes the label an ordinary element again.
    pub fn set_link_elements(&self, links: &[SpanLink<'_>]) {
        let elements = (!links.is_empty()).then(|| {
            let text = TextElement::new(self);
            let mut elements: Vec<Retained<AnyObject>> = Vec::with_capacity(links.len() + 1);
            elements.push(Retained::into_super(Retained::into_super(text)).into());
            for link in links {
                let element = SpanElement::new(self, link);
                elements.push(Retained::into_super(Retained::into_super(element)).into());
            }
            NSArray::from_retained_slice(&elements)
        });
        let had_links = self.ivars().link_elements.replace(elements).is_some();
        if !had_links && links.is_empty() {
            return;
        }
        // SAFETY: a null argument is the documented "layout changed" post.
        unsafe {
            UIAccessibilityPostNotification(UIAccessibilityLayoutChangedNotification, None);
        }
    }
}

define_class!(
    // SAFETY: `UIAccessibilityElement`'s designated initializer is
    // `initWithAccessibilityContainer:`, which `TextElement::new` calls,
    // and the class does not implement `Drop`.
    #[unsafe(super(UIAccessibilityElement))]
    #[name = "CocoaUiTextElement"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Weak<Label>]
    /// A container label's full-text element: every answer is read live
    /// from the label, so accessibility modifiers applied to the label keep
    /// reaching assistive technologies.
    struct TextElement;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIAccessibilityElement`
    // subclass.
    unsafe impl NSObjectProtocol for TextElement {}

    impl TextElement {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Option<Retained<NSString>> {
            guarded("TextElement accessibilityLabel", || {
                self.ivars().load()?.accessibilityLabel(self.mtm())
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSString>> {
            guarded("TextElement accessibilityValue", || {
                self.ivars().load()?.accessibilityValue(self.mtm())
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityTraits))]
        fn accessibility_traits(&self) -> UIAccessibilityTraits {
            guarded("TextElement accessibilityTraits", || {
                self.ivars()
                    .load()
                    .map_or(0, |label| label.accessibilityTraits(self.mtm()))
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> CGRect {
            guarded("TextElement accessibilityFrame", || {
                self.ivars().load().map_or(CGRect::ZERO, |label| {
                    UIAccessibilityConvertFrameToScreenCoordinates(label.bounds(), &label)
                })
            })
        }
    }
);

impl TextElement {
    fn new(label: &Label) -> Retained<Self> {
        let mtm = MainThreadMarker::from(label);
        let this = Self::alloc(mtm).set_ivars(Weak::new(label));
        // SAFETY: `initWithAccessibilityContainer:` is the designated
        // initializer; the label is a valid container.
        let this: Retained<Self> =
            unsafe { msg_send![super(this), initWithAccessibilityContainer: label] };
        this.setIsAccessibilityElement(true);
        this
    }
}

impl fmt::Debug for TextElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextElement").finish_non_exhaustive()
    }
}

/// What a [`SpanElement`] answers with: a screen-coordinate frame computed
/// live — so a moved or resized label never leaves a stale rect — and the
/// activation an assistive press runs.
struct SpanElementIvars {
    frame: Rc<dyn Fn() -> CGRect>,
    activate: Rc<dyn Fn()>,
}

define_class!(
    // SAFETY: `UIAccessibilityElement`'s designated initializer is
    // `initWithAccessibilityContainer:`, which `SpanElement::new` calls,
    // and the class does not implement `Drop`.
    #[unsafe(super(UIAccessibilityElement))]
    #[name = "CocoaUiSpanElement"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SpanElementIvars]
    struct SpanElement;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIAccessibilityElement`
    // subclass.
    unsafe impl NSObjectProtocol for SpanElement {}

    impl SpanElement {
        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> CGRect {
            guarded("SpanElement accessibilityFrame", || (self.ivars().frame)())
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityActivate))]
        fn accessibility_activate(&self) -> bool {
            guarded("SpanElement accessibilityActivate", || {
                (self.ivars().activate)();
                true
            })
        }
    }
);

impl SpanElement {
    /// One span's element: `Link` traits, the span's text as its label, and
    /// `label` as its accessibility container.
    fn new(label: &Label, link: &SpanLink<'_>) -> Retained<Self> {
        let mtm = MainThreadMarker::from(label);
        let weak = Weak::new(label);
        let range = link.range.clone();
        let frame = Rc::new(move || {
            let Some(label) = weak.load() else {
                return CGRect::ZERO;
            };
            let union = label
                .rects(range.clone())
                .into_iter()
                .map(CGRect::from)
                .reduce(|a, b| NSUnionRect(a, b));
            union.map_or(CGRect::ZERO, |rect| {
                UIAccessibilityConvertFrameToScreenCoordinates(rect, &label)
            })
        });
        let this = Self::alloc(mtm).set_ivars(SpanElementIvars {
            frame,
            activate: Rc::clone(&link.activate),
        });
        // SAFETY: `initWithAccessibilityContainer:` is the designated
        // initializer; the label is a valid container.
        let this: Retained<Self> =
            unsafe { msg_send![super(this), initWithAccessibilityContainer: label] };
        this.setAccessibilityLabel(Some(&NSString::from_str(link.text)));
        // SAFETY: the trait constant is an immutable `u64` UIKit publishes.
        this.setAccessibilityTraits(unsafe { UIAccessibilityTraitLink });
        this
    }
}

impl fmt::Debug for SpanElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpanElement").finish_non_exhaustive()
    }
}

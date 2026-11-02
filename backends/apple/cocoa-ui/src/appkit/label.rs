//! The `AppKit` label: an `NSTextField` configured as read-only wrapped text.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSTextField` subclass, forwards to
//! `NSTextField`'s own implementations of the methods it overrides, and calls
//! `objc2` bindings marked unsafe because `AppKit` text APIs are main-thread
//! only — which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityElement, NSAccessibilityFrameInView, NSAccessibilityLinkRole,
    NSLineBreakMode, NSScreen, NSTextAlignment, NSTextField, NSTextFieldCell, NSView,
};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_foundation::{
    NSArray, NSAttributedString, NSEdgeInsets, NSObjectProtocol, NSString, NSUnionRect,
};

use crate::callback::guarded;
use crate::geometry::{Point, Rect};
use crate::text::{Layout, SpanLink, TextMetrics, WrapWidth};

/// The text and line limit a [`Label`] measures with.
///
/// `NSTextField` folds its cell's break mode into the attributed string it
/// stores, so measurement keeps its own copy of the caller's string.
pub struct LabelIvars {
    source_text: RefCell<Option<Retained<NSAttributedString>>>,
    /// Maximum visible lines; `0` wraps unbounded.
    line_limit: Cell<usize>,
    /// `bounds.width` as of the last measure-time invalidation.
    reported_width: Cell<f64>,
}

impl fmt::Debug for LabelIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LabelIvars")
            .field("source_text", &self.source_text.borrow().is_some())
            .field("line_limit", &self.line_limit.get())
            .field("reported_width", &self.reported_width.get())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSTextField`'s designated initializer is `initWithFrame:`,
    // which `Label::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSTextField))]
    #[name = "CocoaUiLabel"]
    #[thread_kind = MainThreadOnly]
    #[ivars = LabelIvars]
    #[derive(Debug)]
    /// A non-editable, non-selectable, wrapped `NSTextField` used as a text
    /// leaf.
    pub struct Label;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSTextField` subclass.
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
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size_override(&self, size: objc2_core_foundation::CGSize) {
            guarded("Label setFrameSize:", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
                // A new width changes the wrapped height: invalidate the
                // intrinsic size once per width. `labelWithString:` runs
                // `setFrameSize:` during its own initialization — before
                // any `set_ivars` could have run — so the ivar storage is
                // read without the initialized check.
                let width = self.bounds().size.width;
                // SAFETY: the storage is `set_ivars`-written, marked, or
                // still zero-filled, all valid `LabelIvars` states.
                let ivars = unsafe { crate::defined_class::raw_ivars(self) };
                if width > 0.0 && ivars.reported_width.replace(width).to_bits() != width.to_bits() {
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
            reported_width: Cell::new(0.0),
        });
        // SAFETY: `initWithFrame:` is `NSTextField`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setEditable(false);
        this.setSelectable(false);
        this.setBordered(false);
        this.setDrawsBackground(false);
        this.setBezeled(false);
        let cell = this
            .cell()
            .and_then(|cell| cell.downcast::<NSTextFieldCell>().ok());
        if let Some(cell) = &cell {
            cell.setWraps(true);
            cell.setScrollable(false);
        }
        this
    }

    /// `+[NSTextField labelWithString:]`: the label-style factory, whose
    /// cell keeps minimal text insets.
    ///
    /// The factory's own `alloc` zero-fills the ivar storage, which is
    /// exactly the state `LabelIvars` starts in, so no `set_ivars` step is
    /// needed.
    #[must_use]
    pub fn label_with_string(mtm: MainThreadMarker, title: &str) -> Retained<Self> {
        let _ = mtm;
        let title = NSString::from_str(title);
        // SAFETY: invoked on the subclass itself, `labelWithString:`
        // allocates through `self`, so the result is a `Label`.
        let this: Retained<Self> = unsafe { msg_send![Self::class(), labelWithString: &*title] };
        crate::defined_class::mark_ivars_initialized(&*this);
        this.setEditable(false);
        this.setSelectable(false);
        this.setBordered(false);
        this.setDrawsBackground(false);
        this.setBezeled(false);
        let cell = this
            .cell()
            .and_then(|cell| cell.downcast::<NSTextFieldCell>().ok());
        if let Some(cell) = &cell {
            cell.setWraps(true);
            cell.setScrollable(false);
        }
        this
    }

    /// `-[NSView alignmentRectInsets]`: the insets between the label's
    /// frame and the rect its text aligns to.
    #[must_use]
    pub fn alignment_rect_insets(&self) -> NSEdgeInsets {
        self.alignmentRectInsets()
    }

    /// A plain-string replacement — no attributes, drawn with the label's
    /// own style.
    pub fn set_text(&self, text: &str) {
        let string = NSString::from_str(text);
        // SAFETY: `initWithString:` is `NSAttributedString`'s plain-string
        // designated initializer; `string` outlives the call.
        let attributed: Retained<NSAttributedString> = unsafe {
            msg_send![
                <NSAttributedString as objc2::AnyThread>::alloc(),
                initWithString: &*string
            ]
        };
        self.set_attributed_text(&attributed);
    }

    /// Replaces the text the label draws, and reports that its size changed.
    pub fn set_attributed_text(&self, text: &NSAttributedString) {
        self.ivars().source_text.replace(Some(text.into()));
        self.setAttributedStringValue(text);
        self.invalidate_layout();
    }

    /// The attributed string measurement runs against: the caller's own
    /// string, not the cell's break-mode-adjusted copy.
    #[must_use]
    pub fn source_text(&self) -> Option<Retained<NSAttributedString>> {
        self.ivars().source_text.borrow().clone()
    }

    /// Limits the label to `limit` lines; `0` removes any limit and wraps.
    ///
    /// Truncation happens on the last visible line's tail.
    pub fn set_line_limit(&self, limit: usize) {
        self.ivars().line_limit.set(limit);
        let cell = self
            .cell()
            .and_then(|cell| cell.downcast::<NSTextFieldCell>().ok());
        if limit == 0 {
            self.setMaximumNumberOfLines(0);
            if let Some(cell) = &cell {
                cell.setWraps(true);
                cell.setTruncatesLastVisibleLine(false);
                cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            }
        } else {
            self.setMaximumNumberOfLines(limit.cast_signed());
            if let Some(cell) = &cell {
                cell.setTruncatesLastVisibleLine(true);
                cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            }
        }
        self.invalidate_layout();
    }

    /// How the text aligns within the label's width.
    pub fn set_text_alignment(&self, alignment: NSTextAlignment) {
        self.setAlignment(alignment);
        self.invalidate_layout();
    }

    /// The width the cell needs to draw the text on one line — its cell
    /// size, wider than the bare text bounds by the cell's insets. Below it
    /// a wrapping cell folds text onto clipped lines.
    #[must_use]
    pub fn fitting_width(&self) -> f64 {
        self.cell().map_or_else(
            || self.intrinsicContentSize().width,
            |cell| cell.cellSize().width,
        )
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

    /// Device pixels per point where the label is drawn — its window's
    /// backing scale, falling back to the main screen.
    #[must_use]
    pub fn display_scale(&self) -> f64 {
        let scale = self.window().map_or_else(
            || {
                NSScreen::mainScreen(MainThreadMarker::from(self))
                    .map_or(1.0, |s| s.backingScaleFactor())
            },
            |window| window.backingScaleFactor(),
        );
        if scale > 0.0 { scale } else { 1.0 }
    }

    /// The label's intrinsic size changed: invalidate it and walk the
    /// superview chain so every ancestor re-runs layout.
    fn invalidate_layout(&self) {
        self.invalidateIntrinsicContentSize();
        crate::view::invalidate_layout(self);
    }

    /// The rect the cell draws its text in, in bounds coordinates —
    /// `drawingRectForBounds:` answers where the cell's interior goes.
    fn text_rect(&self) -> CGRect {
        self.cell()
            .and_then(|cell| cell.downcast::<NSTextFieldCell>().ok())
            .map_or_else(
                || self.bounds(),
                |cell| cell.drawingRectForBounds(self.bounds()),
            )
    }

    /// The laid-out stack a hit test runs through: container width is the
    /// drawing rect's, paragraph alignment the label's own.
    fn layout(&self) -> Option<Layout> {
        let text = self.source_text()?;
        Some(Layout::new(
            MainThreadMarker::from(self),
            &text,
            self.text_rect().size.width,
            self.alignment(),
            self.maximumNumberOfLines().cast_unsigned(),
        ))
    }

    /// The UTF-16 character index a point in bounds coordinates hits —
    /// `None` when it lands outside the text. The bounds' y axis runs up
    /// while the layout's runs down from the text rect's top.
    #[must_use]
    pub fn character_index(&self, point: Point) -> Option<usize> {
        let rect = self.text_rect();
        self.layout()?.character_index(CGPoint::new(
            point.x - rect.origin.x,
            rect.origin.y + rect.size.height - point.y,
        ))
    }

    /// The rectangles a UTF-16 `range` covers in bounds coordinates — one
    /// per line fragment the range crosses.
    #[must_use]
    pub fn rects(&self, range: Range<usize>) -> Vec<Rect> {
        let Some(layout) = self.layout() else {
            return Vec::new();
        };
        let rect = self.text_rect();
        let top = rect.origin.y + rect.size.height;
        layout
            .rects(range)
            .into_iter()
            .map(|container_rect| {
                Rect::new(
                    container_rect.origin.x + rect.origin.x,
                    top - container_rect.origin.y - container_rect.size.height,
                    container_rect.size.width,
                    container_rect.size.height,
                )
            })
            .collect()
    }

    /// Publishes one `Link` accessibility element per span as children of
    /// the label — the way `AppKit` hangs `AXLink` elements off a text
    /// element. An element's label is the span's text, its frame the span's
    /// drawn rectangles, its press the span's action. An empty list clears
    /// them.
    ///
    /// The label stays an accessibility element for its full text.
    pub fn set_link_elements(&self, links: &[SpanLink<'_>]) {
        let view: &NSView = self;
        let element: &ProtocolObject<dyn NSAccessibility> = ProtocolObject::from_ref(view);
        if links.is_empty() {
            // SAFETY: `nil` restores the view's traversal children.
            unsafe { element.setAccessibilityChildren(None) };
            return;
        }
        let elements: Vec<Retained<AnyObject>> = links
            .iter()
            .map(|link| {
                Retained::into_super(Retained::into_super(SpanElement::new(self, link))).into()
            })
            .collect();
        let array = NSArray::from_retained_slice(&elements);
        // SAFETY: every entry is an `NSAccessibilityElement`.
        unsafe { element.setAccessibilityChildren(Some(&array)) };
    }
}

/// What a [`SpanElement`] answers with: frames computed live — so a moved
/// or resized label never leaves a stale rect — and the activation an
/// assistive press runs. `frame` reports screen coordinates,
/// `frame_in_parent` the label's.
struct SpanElementIvars {
    frame: Rc<dyn Fn() -> CGRect>,
    frame_in_parent: Rc<dyn Fn() -> CGRect>,
    activate: Rc<dyn Fn()>,
}

define_class!(
    // SAFETY: `NSAccessibilityElement`'s designated initializer is `init`,
    // which `SpanElement::new` calls, and the class does not implement
    // `Drop`.
    #[unsafe(super(NSAccessibilityElement))]
    #[name = "CocoaUiSpanElement"]
    #[ivars = SpanElementIvars]
    struct SpanElement;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSAccessibilityElement`
    // subclass.
    unsafe impl NSObjectProtocol for SpanElement {}

    impl SpanElement {
        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> CGRect {
            guarded("SpanElement accessibilityFrame", || (self.ivars().frame)())
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrameInParentSpace))]
        fn accessibility_frame_in_parent_space(&self) -> CGRect {
            guarded("SpanElement accessibilityFrameInParentSpace", || {
                (self.ivars().frame_in_parent)()
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            guarded("SpanElement accessibilityPerformPress", || {
                (self.ivars().activate)();
                true
            })
        }
    }
);

impl SpanElement {
    /// One span's element: `Link` role, the span's text as its label, and
    /// the label as its accessibility parent.
    fn new(label: &Label, link: &SpanLink<'_>) -> Retained<Self> {
        let weak = Weak::new(label);
        let union = {
            let range = link.range.clone();
            let weak = weak.clone();
            move || -> CGRect {
                let Some(label) = weak.load() else {
                    return CGRect::ZERO;
                };
                label
                    .rects(range.clone())
                    .into_iter()
                    .map(CGRect::from)
                    .reduce(|a, b| NSUnionRect(a, b))
                    .unwrap_or(CGRect::ZERO)
            }
        };
        let frame = {
            let union = union.clone();
            Rc::new(move || {
                weak.load().map_or(CGRect::ZERO, |label| {
                    NSAccessibilityFrameInView(&label, union())
                })
            })
        };
        let frame_in_parent = Rc::new(union);
        let mtm = MainThreadMarker::from(label);
        let this = mtm.alloc::<Self>().set_ivars(SpanElementIvars {
            frame,
            frame_in_parent,
            activate: Rc::clone(&link.activate),
        });
        // SAFETY: `init` is the class's designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let element: &NSAccessibilityElement = &this;
        let accessibility = ProtocolObject::<dyn NSAccessibility>::from_ref(element);
        accessibility.setAccessibilityElement(true);
        // SAFETY: the role constant is an immutable `NSString` `AppKit`
        // exports.
        accessibility.setAccessibilityRole(Some(unsafe { NSAccessibilityLinkRole }));
        accessibility.setAccessibilityLabel(Some(&NSString::from_str(link.text)));
        // SAFETY: the label is the element's accessibility parent — an
        // `NSView` is a valid parent object.
        unsafe {
            accessibility.setAccessibilityParent(Some(AsRef::<AnyObject>::as_ref(label)));
        }
        this
    }
}

impl fmt::Debug for SpanElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpanElement").finish_non_exhaustive()
    }
}

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

use objc2::rc::Retained;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSLineBreakMode, NSScreen, NSTextAlignment, NSTextField, NSTextFieldCell};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSAttributedString, NSEdgeInsets, NSObjectProtocol, NSString};

use crate::callback::guarded;
use crate::text::{TextMetrics, WrapWidth};

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
                // intrinsic size once per width.
                let width = self.bounds().size.width;
                if width > 0.0
                    && self.raw_ivars().reported_width.replace(width).to_bits()
                        != width.to_bits()
                {
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
        this.mark_ivars_initialized();
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

    /// The `LabelIvars` storage read without the debug initialized-ivars
    /// check: `labelWithString:` triggers `setFrameSize:` during its own
    /// initialization — before any `set_ivars` could have run — so this
    /// override cannot go through `ivars()`.
    fn raw_ivars(&self) -> &LabelIvars {
        static OFFSET: std::sync::OnceLock<isize> = std::sync::OnceLock::new();
        let offset = *OFFSET.get_or_init(|| {
            Self::class()
                .instance_variable(c"ivars")
                .expect("objc2 stores DefinedClass ivars under the `ivars` ivar")
                .offset()
        });
        // SAFETY: `ivars` is the storage `#[ivars = LabelIvars]` registers,
        // so the offset points at a correctly aligned `LabelIvars`; freshly
        // allocated objects have it zero-filled, which is a valid
        // `LabelIvars` state (None / zeroed cells).
        #[expect(clippy::cast_ptr_alignment)]
        unsafe {
            &*std::ptr::from_ref::<Self>(self)
                .cast::<u8>()
                .offset(offset)
                .cast::<LabelIvars>()
        }
    }

    /// `labelWithString:` allocs through `self` but never runs
    /// `set_ivars`, so `objc2`'s drop flag reads `Allocated` — which both
    /// debug `ivars()` checks and ivar destruction gate on. The zero-filled
    /// storage already equals the initial `LabelIvars` state, so mark it
    /// initialized the way `initialize_ivars` would have.
    fn mark_ivars_initialized(&self) {
        let cls = Self::class();
        let Some(ivar) = cls.instance_variable(c"drop_flag") else {
            return;
        };
        // SAFETY: `drop_flag` is `objc2`'s ivar-state marker; `0x0f` is its
        // `InitializedIvars` value. The ivars hold valid zeroed state, so
        // this only records what is already true.
        unsafe {
            std::ptr::from_ref::<Self>(self)
                .cast::<u8>()
                .cast_mut()
                .offset(ivar.offset())
                .write(0x0f);
        }
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
}

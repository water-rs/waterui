//! The `AppKit` segmented control: an `NSSegmentedControl` reporting
//! selection changes.
//!
//! Segments carry a label and an optional icon; the control runs
//! `on_select` with the tapped segment's index. It is the Mac's in-toolbar
//! tab control: `trackingMode` `.selectOne`, spring-loaded, compact sizing.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSSegmentedControl` subclass that is its
//! own target — `NSSegmentedControl` fires `action` on `target` — and calls
//! `objc2`/`AppKit` bindings marked unsafe because `AppKit` control APIs are
//! main-thread only, which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSImage, NSSegmentSwitchTracking, NSSegmentedControl};
use objc2_core_foundation::{CGRect, CGSize};
use objc2_foundation::{NSObjectProtocol, NSString};

use crate::callback::guarded;

/// One segment's content.
#[derive(Clone, Debug, Default)]
pub struct Segment {
    /// The segment's label.
    pub label: String,
    /// A system-symbol icon name, when the segment carries an icon.
    pub symbol: Option<String>,
    /// An icon image, when the symbol did not resolve.
    pub image: Option<Retained<NSImage>>,
    /// Whether the segment is enabled.
    pub enabled: bool,
    /// The segment's badge text, shown after the label.
    pub badge: Option<String>,
}

/// Called with the tapped segment's index.
type SelectHandler = Rc<dyn Fn(usize)>;

/// The segmented control's selection state.
pub struct SegmentedControlIvars {
    /// Called with the tapped segment's index.
    select: RefCell<Option<SelectHandler>>,
}

impl fmt::Debug for SegmentedControlIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegmentedControlIvars")
            .field("has_select", &self.select.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSSegmentedControl`'s designated initializer is
    // `initWithFrame:`, which `SegmentedControl::new` calls, and the class
    // does not implement `Drop`.
    #[unsafe(super(NSSegmentedControl))]
    #[name = "CocoaUiSegmentedControl"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SegmentedControlIvars]
    #[derive(Debug)]
    /// An `NSSegmentedControl` reporting its selection.
    pub struct SegmentedControl;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSSegmentedControl`.
    unsafe impl NSObjectProtocol for SegmentedControl {}

    impl SegmentedControl {
        // SAFETY: `segmentChanged` is this class's own target action.
        #[unsafe(method(segmentChanged:))]
        fn segment_changed(&self, _sender: &NSSegmentedControl) {
            guarded("SegmentedControl segmentChanged:", || {
                let index = self.selectedSegment();
                if index < 0 {
                    return;
                }
                let handler = self.ivars().select.borrow().clone();
                if let Some(handler) = handler {
                    handler(index.cast_unsigned());
                }
            });
        }
    }
);

impl SegmentedControl {
    /// A segmented control with no segments.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SegmentedControlIvars {
            select: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is `NSSegmentedControl`'s designated
        // initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setSegmentStyle(objc2_app_kit::NSSegmentStyle::Automatic);
        this.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
        // SAFETY: `self` is its own target for `segmentChanged:`.
        unsafe {
            this.setTarget(Some(
                std::ptr::from_ref::<Self>(this.as_ref())
                    .cast::<AnyObject>()
                    .as_ref()
                    .unwrap_unchecked(),
            ));
        }
        // SAFETY: `setAction:` registers `segmentChanged:`.
        unsafe { this.setAction(Some(sel!(segmentChanged:))) };
        this
    }

    /// Replaces the segments.
    pub fn set_segments(&self, segments: &[Segment]) {
        self.setSegmentCount(segments.len().cast_signed());
        for (index, segment) in segments.iter().enumerate() {
            let index = index.cast_signed();
            let label = segment.badge.as_ref().map_or_else(
                || segment.label.clone(),
                |badge| format!("{} ({badge})", segment.label),
            );
            self.setLabel_forSegment(&NSString::from_str(&label), index);
            self.setEnabled_forSegment(segment.enabled, index);
            if let Some(symbol) = &segment.symbol {
                if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(symbol),
                    None,
                ) {
                    self.setImage_forSegment(Some(&image), index);
                }
            } else if let Some(image) = &segment.image {
                self.setImage_forSegment(Some(image), index);
            }
        }
        self.sizeToFit();
    }

    /// The selected segment's index; `None` when nothing is selected.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        let index = self.selectedSegment();
        (index >= 0).then_some(index.cast_unsigned())
    }

    /// Selects `index` without firing `on_select`.
    pub fn select(&self, index: Option<usize>) {
        self.setSelectedSegment(index.map_or(-1, usize::cast_signed));
    }

    /// Runs `handler` with the tapped segment's index.
    pub fn set_select_handler(&self, handler: impl Fn(usize) + 'static) {
        self.ivars().select.replace(Some(Rc::new(handler)));
    }

    /// The control's measured size.
    #[must_use]
    pub fn fitting_size(&self) -> CGSize {
        self.fittingSize()
    }
}

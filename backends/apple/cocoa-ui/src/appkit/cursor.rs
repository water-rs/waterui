//! The pointer's appearance over a view.
//!
//! [`Cursor`] names the system cursor a view wants while the pointer is
//! inside it; a host view's `cursorUpdate` handler calls [`Cursor::set`].
//!
//! # Safety
//!
//! No unsafe code beyond the documented call to create the wait cursor's
//! image; every other cursor is a shared `NSCursor` the system vends.

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{NSCursor, NSImage};
use objc2_foundation::{NSPoint, NSString};

/// A system cursor style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cursor {
    /// The default arrow.
    #[default]
    Arrow,
    /// A pointing hand, for clickable content.
    PointingHand,
    /// The text-insertion beam.
    IBeam,
    /// A crosshair, for precise selection.
    Crosshair,
    /// An open hand, for draggable content.
    OpenHand,
    /// A closed hand, while dragging.
    ClosedHand,
    /// An operation-not-allowed badge.
    NotAllowed,
    /// Left-edge resize.
    ResizeLeft,
    /// Right-edge resize.
    ResizeRight,
    /// Top-edge resize.
    ResizeUp,
    /// Bottom-edge resize.
    ResizeDown,
    /// Horizontal resize.
    ResizeLeftRight,
    /// Vertical resize.
    ResizeUpDown,
    /// Move; `AppKit` has no move cursor, so the open hand stands in.
    Move,
    /// A wait indicator — the system hourglass.
    Wait,
    /// A copy badge, for copy drags.
    Copy,
}

// The legacy resize cursors are the directional `CursorStyle` values.
#[allow(deprecated)]
impl Cursor {
    /// The `NSCursor` this style is drawn as.
    ///
    /// # Panics
    ///
    /// For [`Cursor::Wait`], when the system hourglass image is unavailable.
    #[must_use]
    pub fn platform(&self) -> Retained<NSCursor> {
        match self {
            Self::Arrow => NSCursor::arrowCursor(),
            Self::PointingHand => NSCursor::pointingHandCursor(),
            Self::IBeam => NSCursor::IBeamCursor(),
            Self::Crosshair => NSCursor::crosshairCursor(),
            Self::OpenHand | Self::Move => NSCursor::openHandCursor(),
            Self::ClosedHand => NSCursor::closedHandCursor(),
            Self::NotAllowed => NSCursor::operationNotAllowedCursor(),
            Self::ResizeLeft => NSCursor::resizeLeftCursor(),
            Self::ResizeRight => NSCursor::resizeRightCursor(),
            Self::ResizeUp => NSCursor::resizeUpCursor(),
            Self::ResizeDown => NSCursor::resizeDownCursor(),
            Self::ResizeLeftRight => NSCursor::resizeLeftRightCursor(),
            Self::ResizeUpDown => NSCursor::resizeUpDownCursor(),
            Self::Wait => {
                let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str("hourglass"),
                    None,
                )
                .expect("the system hourglass cursor image is unavailable");
                let size = image.size();
                let hotspot = NSPoint::new(size.width / 2.0, size.height / 2.0);
                // `image` is a valid cursor image.
                NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, hotspot)
            }
            Self::Copy => NSCursor::dragCopyCursor(),
        }
    }

    /// Makes this cursor the current one.
    pub fn set(&self) {
        self.platform().set();
    }
}

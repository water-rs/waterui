//! Pointer events on a view: hover entry, movement, exit, and cursor
//! changes.
//!
//! [`HostView`](crate::view::HostView)-style surfaces subscribe to a subset
//! of the events through a [`PointerEvents`] flag set and receive each one
//! as a [`PointerEvent`]. `UIKit` delivers these through a
//! `UIHoverGestureRecognizer`, `AppKit` through an `NSTrackingArea`; both
//! end up as the same value here.
//!
//! # Safety
//!
//! No unsafe code; the platform wires are safe calls on the view classes.

use crate::Point;

/// One pointer event on a view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerEvent {
    /// The pointer entered the view.
    Entered,
    /// The pointer moved within the view, at this point in view
    /// coordinates.
    Moved(Point),
    /// The pointer left the view.
    Exited,
    /// `AppKit` asking for the cursor inside this view. The handler chooses
    /// whether to let the event pass on (returns whether it consumed it);
    /// `UIKit` never delivers it.
    CursorUpdate,
}

/// Which pointer events a handler wants delivered, as a bit set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PointerEvents(u8);

impl PointerEvents {
    /// No events.
    pub const NONE: Self = Self(0);
    /// [`PointerEvent::Entered`].
    pub const ENTERED: Self = Self(1 << 0);
    /// [`PointerEvent::Moved`].
    pub const MOVED: Self = Self(1 << 1);
    /// [`PointerEvent::Exited`].
    pub const EXITED: Self = Self(1 << 2);
    /// [`PointerEvent::CursorUpdate`].
    pub const CURSOR_UPDATE: Self = Self(1 << 3);

    /// Every event.
    pub const ALL: Self = Self(0b1111);

    /// `self` plus `other`.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether `self` wants this event.
    #[must_use]
    pub const fn wants(self, event: PointerEvent) -> bool {
        let flag = match event {
            PointerEvent::Entered => Self::ENTERED,
            PointerEvent::Moved(_) => Self::MOVED,
            PointerEvent::Exited => Self::EXITED,
            PointerEvent::CursorUpdate => Self::CURSOR_UPDATE,
        };
        self.0 & flag.0 != 0
    }
}

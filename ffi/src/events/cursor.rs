//! FFI bindings for cursor types.

use crate::IntoFFI;
use crate::reactive::WuiComputed;
use waterui::cursor::{Cursor, CursorStyle};

/// FFI-safe cursor style enum.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiCursorStyle {
    /// Default arrow cursor (system default behavior).
    Arrow = 0,
    /// Pointing hand cursor (for clickable/link elements).
    PointingHand = 1,
    /// Text selection cursor (I-beam).
    IBeam = 2,
    /// Crosshair cursor (for precise selection).
    Crosshair = 3,
    /// Open hand cursor (for draggable content).
    OpenHand = 4,
    /// Closed hand cursor (while dragging).
    ClosedHand = 5,
    /// Not-allowed cursor (for disabled actions).
    NotAllowed = 6,
    /// Resize left cursor.
    ResizeLeft = 7,
    /// Resize right cursor.
    ResizeRight = 8,
    /// Resize up cursor.
    ResizeUp = 9,
    /// Resize down cursor.
    ResizeDown = 10,
    /// Resize left-right cursor (horizontal resize).
    ResizeLeftRight = 11,
    /// Resize up-down cursor (vertical resize).
    ResizeUpDown = 12,
    /// Move cursor (for movable content).
    Move = 13,
    /// Wait/loading cursor.
    Wait = 14,
    /// Copy cursor (for copy operations).
    Copy = 15,
}

impl IntoFFI for CursorStyle {
    type FFI = WuiCursorStyle;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Arrow => WuiCursorStyle::Arrow,
            Self::PointingHand => WuiCursorStyle::PointingHand,
            Self::IBeam => WuiCursorStyle::IBeam,
            Self::Crosshair => WuiCursorStyle::Crosshair,
            Self::OpenHand => WuiCursorStyle::OpenHand,
            Self::ClosedHand => WuiCursorStyle::ClosedHand,
            Self::NotAllowed => WuiCursorStyle::NotAllowed,
            Self::ResizeLeft => WuiCursorStyle::ResizeLeft,
            Self::ResizeRight => WuiCursorStyle::ResizeRight,
            Self::ResizeUp => WuiCursorStyle::ResizeUp,
            Self::ResizeDown => WuiCursorStyle::ResizeDown,
            Self::ResizeLeftRight => WuiCursorStyle::ResizeLeftRight,
            Self::ResizeUpDown => WuiCursorStyle::ResizeUpDown,
            Self::Move => WuiCursorStyle::Move,
            Self::Wait => WuiCursorStyle::Wait,
            Self::Copy => WuiCursorStyle::Copy,
            _ => panic!("unsupported CursorStyle variant for FFI"),
        }
    }
}

/// FFI-safe representation of cursor metadata.
#[repr(C)]
#[derive(Debug)]
pub struct WuiCursor {
    /// The cursor style (reactive).
    pub style: *mut WuiComputed<CursorStyle>,
}

impl IntoFFI for Cursor {
    type FFI = WuiCursor;
    fn into_ffi(self) -> Self::FFI {
        WuiCursor {
            style: self.style.into_ffi(),
        }
    }
}

// Generate FFI functions for CursorStyle computed values
crate::ffi_computed!(CursorStyle, WuiCursorStyle, cursor_style);

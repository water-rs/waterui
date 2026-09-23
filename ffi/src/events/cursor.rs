//! FFI bindings for cursor types.

use crate::IntoFFI;
use crate::reactive::WuiComputed;
use waterui::cursor::{Cursor, CursorStyle};

/// FFI-safe cursor style enum.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiCursorStyle {
    Arrow = 0,
    PointingHand = 1,
    IBeam = 2,
    Crosshair = 3,
    OpenHand = 4,
    ClosedHand = 5,
    NotAllowed = 6,
    ResizeLeft = 7,
    ResizeRight = 8,
    ResizeUp = 9,
    ResizeDown = 10,
    ResizeLeftRight = 11,
    ResizeUpDown = 12,
    Move = 13,
    Wait = 14,
    Copy = 15,
}

impl IntoFFI for CursorStyle {
    type FFI = WuiCursorStyle;
    fn into_ffi(self) -> Self::FFI {
        match self {
            CursorStyle::Arrow => WuiCursorStyle::Arrow,
            CursorStyle::PointingHand => WuiCursorStyle::PointingHand,
            CursorStyle::IBeam => WuiCursorStyle::IBeam,
            CursorStyle::Crosshair => WuiCursorStyle::Crosshair,
            CursorStyle::OpenHand => WuiCursorStyle::OpenHand,
            CursorStyle::ClosedHand => WuiCursorStyle::ClosedHand,
            CursorStyle::NotAllowed => WuiCursorStyle::NotAllowed,
            CursorStyle::ResizeLeft => WuiCursorStyle::ResizeLeft,
            CursorStyle::ResizeRight => WuiCursorStyle::ResizeRight,
            CursorStyle::ResizeUp => WuiCursorStyle::ResizeUp,
            CursorStyle::ResizeDown => WuiCursorStyle::ResizeDown,
            CursorStyle::ResizeLeftRight => WuiCursorStyle::ResizeLeftRight,
            CursorStyle::ResizeUpDown => WuiCursorStyle::ResizeUpDown,
            CursorStyle::Move => WuiCursorStyle::Move,
            CursorStyle::Wait => WuiCursorStyle::Wait,
            CursorStyle::Copy => WuiCursorStyle::Copy,
            // Handle any future cursor style variants
            _ => WuiCursorStyle::Arrow,
        }
    }
}

/// FFI-safe representation of cursor metadata.
#[repr(C)]
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

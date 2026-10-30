//! The binary boundary the generated extension calls across.
//!
//! [`#[widget]`](macro@crate::widget)-attributed functions are exported as
//! `waterui_widget_<crate>_<fn>` C symbols that take a JSON [`WidgetContext`]
//! and return a [`Buffer`] — an owned, leaked byte buffer carrying the
//! serialized [`Timeline`]. The extension frees each buffer through
//! [`waterui_widgets_snapshot_free`].

use crate::schema::{Timeline, WidgetContext};

/// An owned byte buffer handed across the FFI boundary.
///
/// Layout mirrors `WUISnapshotBuffer` in `swift/FFI.swift`. The extension
/// reads `len` bytes from `ptr`, then hands the buffer back to
/// [`waterui_widgets_snapshot_free`], which reconstructs and drops the
/// `Vec<u8>` — `cap` is required for that reconstruction.
#[repr(C)]
#[derive(Debug)]
#[non_exhaustive]
pub struct Buffer {
    /// The buffer's first byte.
    ptr: *const u8,
    /// Bytes initialized at `ptr`.
    len: usize,
    /// The allocation's capacity.
    cap: usize,
}

impl Buffer {
    /// The buffer's first byte.
    #[must_use]
    pub const fn ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Bytes initialized at `ptr`.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether no bytes are initialized at `ptr`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The allocation's capacity.
    #[must_use]
    pub const fn cap(&self) -> usize {
        self.cap
    }
}

impl Buffer {
    /// Serializes `timeline` and leaks the bytes for the extension to read.
    #[must_use]
    pub fn from_timeline(timeline: &Timeline) -> Self {
        let bytes =
            serde_json::to_vec(timeline).expect("a widget timeline always serializes to JSON");
        Self::from_vec(bytes)
    }

    /// Leaks `bytes`, transferring ownership of the allocation to the caller.
    #[must_use]
    pub fn from_vec(mut bytes: alloc::vec::Vec<u8>) -> Self {
        bytes.shrink_to_fit();
        let (ptr, len, cap) = bytes.into_raw_parts();
        Self { ptr, len, cap }
    }
}

/// Decodes the [`WidgetContext`] the extension passed.
///
/// # Safety
/// `ptr` must point to `len` valid bytes carrying the context JSON — the
/// generated extension constructs that JSON itself from the `WidgetKit`
/// context; malformed input is a schema violation and aborts like any
/// contract failure.
#[must_use]
pub unsafe fn decode_context(ptr: *const u8, len: usize) -> WidgetContext {
    // SAFETY: the caller contract above requires `ptr`/`len` to name a
    // valid byte slice.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    serde_json::from_slice(bytes).expect("the extension's widget context JSON must decode")
}

/// Frees the allocation a `waterui_widget_*` export's [`Buffer`] owns.
///
/// The boundary is scalar-only — `@_extern` cannot pass a `Swift` struct by
/// value, so the extension hands back `ptr`/`len`/`cap` exactly as the
/// [`Buffer`] carried them.
///
/// # Safety
/// The triple must come from a `Buffer` one of those exports produced,
/// used at most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_widgets_snapshot_free(ptr: *const u8, len: usize, cap: usize) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: the caller contract above guarantees `ptr`/`len`/`cap` is a
    // `Vec`'s raw parts handed out exactly once by a `waterui_widget_*`
    // export.
    drop(unsafe { alloc::vec::Vec::from_raw_parts(ptr.cast_mut(), len, cap) });
}

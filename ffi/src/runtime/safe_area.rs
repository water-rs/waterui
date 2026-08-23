//! Publishing the window's safe area to the Rust side.
//!
//! Placing *native* views against the device insets is the backend's own job.
//! What it cannot do from the outside is inset the layers WaterUI lays out
//! itself — the window's snackbar and overlay hosts arrive as a single
//! Rust-laid-out container, so a backend that framed only part of it would have
//! to reach inside a layout it does not own.
//!
//! Instead the backend installs its insets once per window and republishes them
//! whenever they change (rotation, a keyboard, a bar appearing). Those layers
//! read the signal and pad themselves.
//!
//! ```c
//! // Once, while building the environment:
//! waterui_env_install_safe_area(env, insetsSignal);
//! // Then, from the platform's layout callback:
//! setInsets(insetsSignal, (WuiEdgeInsets){ .top = 59, .bottom = 34 });
//! ```

use waterui_layout::padding::EdgeInsets;
use waterui_layout::safe_area::SafeAreaInsets;

use crate::{IntoFFI, IntoRust, WuiEnv, reactive::WuiComputed};

/// C ABI mirror of [`EdgeInsets`]: the four edge distances of a rectangle, in
/// logical points.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WuiEdgeInsets {
    /// Inset from the top edge.
    pub top: f32,
    /// Inset from the bottom edge.
    pub bottom: f32,
    /// Inset from the leading (left in LTR) edge.
    pub leading: f32,
    /// Inset from the trailing (right in LTR) edge.
    pub trailing: f32,
}

impl IntoFFI for EdgeInsets {
    type FFI = WuiEdgeInsets;
    fn into_ffi(self) -> Self::FFI {
        WuiEdgeInsets {
            top: self.top(),
            bottom: self.bottom(),
            leading: self.leading(),
            trailing: self.trailing(),
        }
    }
}

impl IntoRust for WuiEdgeInsets {
    type Rust = EdgeInsets;
    unsafe fn into_rust(self) -> Self::Rust {
        EdgeInsets::new(self.top, self.bottom, self.leading, self.trailing)
    }
}

crate::ffi_computed!(EdgeInsets, WuiEdgeInsets, edge_insets);
crate::ffi_computed_ctor!(EdgeInsets, WuiEdgeInsets, edge_insets);

/// Installs the window's safe area insets into the environment.
///
/// Backends without a safe-area concept install nothing; the Rust side then
/// reads zero insets, which is the right answer for a desktop window.
///
/// # Safety
/// The signal pointer must be an owning pointer from
/// `waterui_new_computed_edge_insets`, and `env` a valid handle that is not
/// otherwise borrowed for this call.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_safe_area(
    env: *mut WuiEnv,
    signal: *mut WuiComputed<EdgeInsets>,
) {
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };
    // SAFETY: the caller contract makes `signal` an owning pointer from the matching
    // FFI constructor, so reclaiming the box frees it exactly once.
    let computed = unsafe { alloc::boxed::Box::from_raw(signal) }.0;
    SafeAreaInsets::install(env, computed);
}

crate::ffi_watcher_notify!(EdgeInsets, WuiEdgeInsets, edge_insets);

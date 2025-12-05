//! Debug utilities for WaterUI.
//!
//! This module provides development-time features including hot reload support.
//! Only available in debug builds (`debug_assertions`).
//!
//! # Hot Reload Architecture
//!
//! ```text
//! ┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
//! │  CliConnection  │────▶│  Stream<CliEvent>│────▶│  Hotreload View │
//! └─────────────────┘     └──────────────────┘     └─────────────────┘
//!         │                                                 │
//!         │ WebSocket                                       │ Dynamic
//!         ▼                                                 ▼
//! ┌─────────────────┐                              ┌─────────────────┐
//! │   CLI Server    │                              │   App Content   │
//! └─────────────────┘                              └─────────────────┘
//! ```
//!
//! # Components
//!
//! - [`CliConnection`] - WebSocket connection to CLI, implements `Stream<Item = CliEvent>`
//! - [`CliEvent`] - Events from CLI: library updates, config changes, connection status
//! - [`Hotreload`] - View wrapper that handles hot reload lifecycle

pub mod connection;
pub mod event;
#[cfg(not(target_arch = "wasm32"))]
pub mod hot_reload;
#[cfg(not(target_arch = "wasm32"))]
pub mod library;
pub mod logging;

pub use connection::CliConnection;
pub use event::{CliEvent, ConnectionError};
#[cfg(not(target_arch = "wasm32"))]
pub use hot_reload::Hotreload;

/// Entry point macro for hot-reloadable views.
#[macro_export]
#[cfg(waterui_hot_reload_lib)]
macro_rules! hot_reloadable_library {
    ($f:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn waterui_hot_reload_main() -> *mut () {
            let view = $f();
            Box::into_raw(Box::new($crate::AnyView::new(view))).cast::<()>()
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn waterui_hot_reload_init() {
            unsafe { $crate::debug::__setup_local_executor() };
        }
    };
}

/// Initialize executor for hot-reloaded dylib.
///
/// # Safety
/// Must be called on the main thread before any async operations in the hot-reloaded dylib.
#[cfg(waterui_hot_reload_lib)]
#[doc(hidden)]
#[inline(always)]
pub unsafe fn __setup_local_executor() {
    let _ = executor_core::try_init_local_executor(native_executor::NativeExecutor::new());
}

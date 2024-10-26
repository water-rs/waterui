#![no_std]
#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]
extern crate alloc;

#[macro_use]
mod macros;

pub mod component;

pub mod animation;
pub mod array;
pub mod binding;
pub mod closure;
pub mod computed;
pub mod ty;
pub mod watcher;

pub use ty::*;
use waterui_reactive::watcher::WatcherGuard;
pub mod action;
pub use waterui_macro::*;
pub trait IntoFFI {
    type FFI;
    fn into_ffi(self) -> Self::FFI;
}

pub trait IntoRust {
    type Rust;
    unsafe fn into_rust(self) -> Self::Rust;
}
ffi_safe!(u8, i32, f64, bool);

ffi_type!(
    waterui_watcher_guard,
    WatcherGuard,
    waterui_drop_watcher_guard
);

#[doc(hidden)]
pub use waterui::block_on as __block_on;

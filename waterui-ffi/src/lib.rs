#![no_std]
extern crate alloc;

#[macro_use]
mod macros;

pub mod binding;
pub mod computed;
pub mod error;

pub mod ty;
pub use ty::*;

pub trait IntoFFI {
    type FFI;
    fn into_ffi(self) -> Self::FFI;
}

pub trait IntoRust {
    type Rust;
    fn into_rust(self) -> Self::Rust;
}

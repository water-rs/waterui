#![no_std]
#![allow(non_camel_case_types)]
extern crate alloc;

#[macro_use]
mod macros;

pub mod array;
pub mod binding;
pub mod closure;

pub mod computed;

pub mod ty;
pub use ty::*;

pub trait IntoFFI {
    type FFI;

    fn into_ffi(self) -> Self::FFI;
}

pub trait IntoRust {
    type Rust;
    unsafe fn into_rust(self) -> Self::Rust;
}

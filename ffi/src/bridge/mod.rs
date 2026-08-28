/// FFI bindings for invoking Rust action/callback closures from native code.
pub mod action;
/// The `WuiArray` family: a C-compatible, type-erased contiguous array with an
/// explicit drop/slice vtable.
pub mod array;
pub mod closure;
pub mod locale;

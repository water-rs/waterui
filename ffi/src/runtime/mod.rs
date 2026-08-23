/// FFI bindings for the application entry point (`WuiApp`) and its Android projection.
pub mod app;
/// FFI bindings for the identity type used by identifiable views and collections.
pub mod id;
pub mod inspector;
/// FFI bindings for publishing the window's safe area to Rust-laid-out layers.
pub mod safe_area;
pub mod theme;
/// FFI bindings for erased, identity-aware view collections (`WuiAnyViews`).
pub mod views;
/// FFI bindings for windows, their style/state/background, and window management.
pub mod window;

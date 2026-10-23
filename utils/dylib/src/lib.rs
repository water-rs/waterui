//! Dynamic-link anchor crate for `waterui`.

// A PE image exports at most 65535 symbols and this crate exports every one
// of the framework's, so it cannot link there; Windows links WaterUI
// statically instead.
#[cfg(windows)]
compile_error!("waterui-dylib does not support Windows: WaterUI links statically there");

extern crate waterui_internal;

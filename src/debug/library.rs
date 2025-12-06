//! Hot reload library loading utilities.

use libloading::Library;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use waterui_core::View;

/// Create a library file from binary data.
#[must_use] 
pub fn create_library(data: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join("hot_reload");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).expect("Failed to create hot_reload directory");
    }

    let name = format!("waterui_hot_{}", uuid::Uuid::new_v4());
    let mut path = dir.join(name);

    // Set platform-specific extension
    if cfg!(target_os = "windows") {
        path.set_extension("dll");
    } else if cfg!(target_os = "macos") {
        path.set_extension("dylib");
    } else {
        path.set_extension("so");
    }

    let mut file = File::create(&path).expect("Failed to create library file");
    file.write_all(data).expect("Failed to write library data");

    path
}

/// Load a view from a hot-reloaded library.
///
/// # Safety
///
/// Must be called on the main thread. The library must export the symbol correctly.
pub unsafe fn load_view(path: &Path) -> impl View {
    let lib = unsafe { Library::new(path) }.expect("Failed to load library");

    // Initialize the executor for the new dylib
    if let Ok(init) = unsafe { lib.get::<unsafe extern "C" fn()>(b"waterui_hot_reload_init") } {
        unsafe { init() };
    }

    // Load the main view function
    let func: libloading::Symbol<unsafe extern "C" fn() -> *mut waterui_core::AnyView> =
        unsafe { lib.get(b"waterui_hot_reload_main") }.expect("Symbol not found");

    let view_ptr = unsafe { func() };
    let view = unsafe { Box::from_raw(view_ptr) };

    // Retain the library so it stays loaded as long as the view exists
    use crate::ViewExt;
    (*view).retain(lib)
}

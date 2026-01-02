//! Dynamic library loading for preview.
//!
//! Handles loading dylibs received from the daemon and resolving preview symbols.

use std::ffi::CString;
use std::path::Path;

use waterui_core::AnyView;

/// A loaded preview library.
#[derive(Debug)]
pub struct PreviewLibrary {
    lib: libloading::Library,
}

impl PreviewLibrary {
    /// Load a library from a file path.
    ///
    /// # Safety
    /// The library must be a valid WaterUI preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the library cannot be loaded.
    pub unsafe fn load(path: &Path) -> Result<Self, libloading::Error> {
        let lib = unsafe { libloading::Library::new(path)? };
        Ok(Self { lib })
    }

    /// Load a library from bytes by writing to a temp file.
    ///
    /// # Safety
    /// The library must be a valid WaterUI preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the library cannot be loaded.
    #[cfg(unix)]
    pub unsafe fn load_from_bytes(data: &[u8]) -> Result<Self, LoadError> {
        use std::io::Write;

        let path = std::env::temp_dir().join("waterui_preview.dylib");

        let mut file = std::fs::File::create(&path).map_err(LoadError::Io)?;
        file.write_all(data).map_err(LoadError::Io)?;

        unsafe { Self::load(&path).map_err(LoadError::Library) }
    }

    /// Check if the library has a symbol.
    #[must_use]
    pub fn has_symbol(&self, name: &str) -> bool {
        let c_name = match CString::new(name.trim_end_matches('\0')) {
            Ok(s) => s,
            Err(_) => return false,
        };

        unsafe { self.lib.get::<*const ()>(c_name.as_bytes_with_nul()) }.is_ok()
    }

    /// Load a preview view from the library.
    ///
    /// # Safety
    /// The symbol must be a valid preview function that returns `*mut ()` (a boxed `AnyView`).
    ///
    /// # Errors
    /// Returns an error if the symbol cannot be loaded.
    pub unsafe fn load_view(&self, symbol_name: &str) -> Result<AnyView, libloading::Error> {
        let c_name = CString::new(symbol_name.trim_end_matches('\0'))
            .expect("symbol name should not contain null bytes");

        let func: libloading::Symbol<unsafe extern "C" fn() -> *mut ()> =
            unsafe { self.lib.get(c_name.as_bytes_with_nul())? };

        let ptr = unsafe { func() };
        let boxed: Box<AnyView> = unsafe { Box::from_raw(ptr.cast()) };
        Ok(*boxed)
    }
}

/// Errors that can occur when loading a library.
#[derive(Debug)]
pub enum LoadError {
    /// IO error writing temp file.
    Io(std::io::Error),
    /// Library loading error.
    Library(libloading::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Library(e) => write!(f, "Library error: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

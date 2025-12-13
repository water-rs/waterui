//! Hot reload library loading utilities.

use alloc::sync::Arc;
use futures::AsyncWriteExt;
use libloading::Library;
use std::path::{Path, PathBuf};
use waterui_core::AnyView;

use crate::ViewExt;

/// Errors that can occur while loading a hot-reloaded library.
#[derive(Debug)]
pub enum LoadViewError {
    /// Failed to open the dynamic library.
    LoadLibrary {
        /// Path to the dynamic library file.
        path: PathBuf,
        /// Underlying loader error message.
        error: String,
    },
    /// A required symbol was missing from the dynamic library.
    MissingSymbol {
        /// Path to the dynamic library file.
        path: PathBuf,
        /// The symbol name that was expected.
        symbol: String,
        /// Underlying loader error message.
        error: String,
    },
    /// The library entry point returned a null pointer.
    NullPointer {
        /// Path to the dynamic library file.
        path: PathBuf,
        /// The symbol name that returned null.
        symbol: String,
    },
}

impl core::fmt::Display for LoadViewError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::LoadLibrary { path, error } => {
                write!(
                    f,
                    "Failed to load hot reload library at {}: {error}",
                    path.display()
                )
            }
            Self::MissingSymbol {
                path,
                symbol,
                error,
            } => write!(
                f,
                "Hot reload library at {} is missing symbol '{symbol}': {error}. \
                 Make sure the library was built with RUSTFLAGS='--cfg waterui_hot_reload_lib'.",
                path.display()
            ),
            Self::NullPointer { path, symbol } => write!(
                f,
                "Hot reload symbol '{symbol}' returned a null pointer from {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LoadViewError {}

/// A loaded hot reload library with shared ownership.
///
/// The library is kept alive as long as any views loaded from it exist.
#[derive(Clone)]
pub struct HotReloadLibrary {
    lib: Arc<Library>,
    path: PathBuf,
}

impl HotReloadLibrary {
    /// Load a hot reload library from the given path.
    ///
    /// # Safety
    ///
    /// The library must be a valid dynamic library built for hot reload.
    ///
    /// # Errors
    ///
    /// Returns an error if the library cannot be loaded.
    pub unsafe fn load(path: &Path) -> Result<Self, LoadViewError> {
        let lib = unsafe { Library::new(path) }.map_err(|e| LoadViewError::LoadLibrary {
            path: path.to_path_buf(),
            error: e.to_string(),
        })?;

        // Initialize the executor for the new dylib
        if let Ok(init) = unsafe { lib.get::<unsafe extern "C" fn()>(b"waterui_hot_reload_init") } {
            unsafe { init() };
        }

        Ok(Self {
            lib: Arc::new(lib),
            path: path.to_path_buf(),
        })
    }

    /// Load a view from this library by symbol name.
    ///
    /// The symbol should be a function with signature `extern "C" fn() -> *mut AnyView`.
    ///
    /// # Safety
    ///
    /// The symbol must exist and return a valid `AnyView` pointer.
    ///
    /// # Errors
    ///
    /// Returns an error if the symbol is not found or returns null.
    pub unsafe fn load_symbol(&self, symbol: &str) -> Result<AnyView, LoadViewError> {
        let symbol_bytes = symbol.as_bytes();

        let func: libloading::Symbol<unsafe extern "C" fn() -> *mut AnyView> =
            unsafe { self.lib.get(symbol_bytes) }.map_err(|e| LoadViewError::MissingSymbol {
                path: self.path.clone(),
                symbol: symbol.to_string(),
                error: e.to_string(),
            })?;

        let view_ptr = unsafe { func() };
        if view_ptr.is_null() {
            return Err(LoadViewError::NullPointer {
                path: self.path.clone(),
                symbol: symbol.to_string(),
            });
        }
        let view = unsafe { Box::from_raw(view_ptr) };

        // Retain the library so it stays loaded as long as the view exists
        Ok(AnyView::new((*view).retain(self.lib.clone())))
    }

    /// Check if this library exports a symbol.
    pub fn has_symbol(&self, symbol: &str) -> bool {
        unsafe { self.lib.get::<*const ()>(symbol.as_bytes()) }.is_ok()
    }
}

impl core::fmt::Debug for HotReloadLibrary {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HotReloadLibrary")
            .field("path", &self.path)
            .finish()
    }
}

/// Create a library file from binary data.
///
/// # Panics
///
/// Panics if the `hot_reload` directory cannot be created, or if the library file
/// cannot be created or written.
#[must_use]
pub async fn create_library(data: &[u8]) -> PathBuf {
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

    let mut file = async_fs::File::create(&path)
        .await
        .expect("Failed to create library file");

    file.write_all(data)
        .await
        .expect("Failed to write library data");

    // Flush and sync to ensure the file is fully written to disk
    // before dlopen tries to load it (prevents race condition)
    file.flush().await.expect("Failed to flush library file");
    file.sync_all().await.expect("Failed to sync library file");

    path
}

/// Load a view from a hot-reloaded library (legacy single-symbol API).
///
/// # Safety
///
/// Must be called on the main thread. The library must export the symbol correctly.
///
/// # Errors
///
/// Returns an error if the library cannot be loaded or if required symbols are not found.
pub unsafe fn load_view(path: &Path) -> Result<AnyView, LoadViewError> {
    let lib = unsafe { HotReloadLibrary::load(path)? };
    unsafe { lib.load_symbol("waterui_hot_reload_main\0") }
}

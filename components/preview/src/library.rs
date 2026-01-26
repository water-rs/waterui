//! Dynamic library loading for preview.
//!
//! Handles loading dylibs received from the daemon and resolving preview symbols.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use waterui_core::AnyView;

/// A loaded preview library.
#[derive(Debug)]
pub struct PreviewLibrary {
    lib: libloading::Library,
    temp_path: Option<PathBuf>,
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
        Ok(Self {
            lib,
            temp_path: None,
        })
    }

    /// Load a library from bytes by writing to a temp file.
    ///
    /// # Safety
    /// The library must be a valid WaterUI preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the library cannot be loaded.
    #[cfg(unix)]
    pub async unsafe fn load_from_bytes(data: &[u8]) -> Result<Self, LoadError> {
        let path = unique_temp_path();

        async_fs::write(&path, data).await.map_err(LoadError::Io)?;

        #[cfg(target_os = "macos")]
        codesign_dylib(&path).await?;

        let lib = blocking::unblock({
            let path = path.clone();
            move || unsafe { libloading::Library::new(&path).map_err(LoadError::Library) }
        })
        .await?;
        Ok(Self {
            lib,
            temp_path: Some(path),
        })
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

impl Drop for PreviewLibrary {
    fn drop(&mut self) {
        if let Some(path) = self.temp_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(unix)]
fn unique_temp_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("waterui_preview_{pid}_{nanos}.dylib"))
}

#[cfg(target_os = "macos")]
async fn codesign_dylib(path: &Path) -> Result<(), LoadError> {
    let output = async_process::Command::new("codesign")
        .arg("--force")
        .arg("--sign")
        .arg("-")
        .arg("--timestamp=none")
        .arg(path)
        .output()
        .await
        .map_err(LoadError::Io)?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(LoadError::CodeSign(if stderr.is_empty() {
            "codesign failed".to_string()
        } else {
            stderr
        }))
    }
}

/// Errors that can occur when loading a library.
#[derive(Debug)]
pub enum LoadError {
    /// IO error writing temp file.
    Io(std::io::Error),
    /// Library loading error.
    Library(libloading::Error),
    /// Codesign error on macOS.
    CodeSign(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Library(e) => write!(f, "Library error: {e}"),
            Self::CodeSign(e) => write!(f, "Codesign error: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

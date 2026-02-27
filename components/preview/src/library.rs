//! Dynamic library loading for preview.
//!
//! Handles loading dylibs received from the daemon and resolving preview symbols.

use std::ffi::CString;
use std::path::Path;

use waterui_core::AnyView;
use waterui_preview_protocol::DylibId;

use crate::cache::preview_dylib_cache_path;

/// A loaded preview library.
#[derive(Debug)]
pub struct PreviewLibrary {
    lib: libloading::Library,
}

impl PreviewLibrary {
    /// Load a library from an on-disk cache path, codesigning only if needed (macOS).
    ///
    /// # Safety
    /// The library must be a valid WaterUI preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the library cannot be loaded.
    #[cfg(unix)]
    pub async unsafe fn load_from_path(path: &Path) -> Result<Self, LoadError> {
        Self::load_with_codesign_fallback(path).await
    }

    #[cfg(target_os = "macos")]
    async fn load_with_codesign_fallback(path: &Path) -> Result<Self, LoadError> {
        let first_try = blocking::unblock({
            let path = path.to_path_buf();
            move || unsafe { libloading::Library::new(&path).map_err(LoadError::Library) }
        })
        .await;

        match first_try {
            Ok(lib) => Ok(Self { lib }),
            Err(load_error) => {
                // Only codesign when the file is not already signed/valid. If it's already
                // signed and dlopen failed, surface the original error immediately.
                if codesign_verify_dylib(path).await? {
                    return Err(load_error);
                }

                codesign_dylib(path).await?;
                let lib = blocking::unblock({
                    let path = path.to_path_buf();
                    move || unsafe { libloading::Library::new(&path).map_err(LoadError::Library) }
                })
                .await?;
                Ok(Self { lib })
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    async fn load_with_codesign_fallback(path: &Path) -> Result<Self, LoadError> {
        let lib = blocking::unblock({
            let path = path.to_path_buf();
            move || unsafe { libloading::Library::new(&path).map_err(LoadError::Library) }
        })
        .await?;
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
    pub async unsafe fn load_from_bytes(id: DylibId, data: &[u8]) -> Result<Self, LoadError> {
        // Prefer a stable on-disk cache keyed by dylib id. This avoids re-codesigning and also
        // enables reuse across preview app restarts.
        let cache_path = preview_dylib_cache_path(id);
        ensure_cached_file(&cache_path, data).await?;

        Self::load_with_codesign_fallback(&cache_path).await
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
        let symbol_bytes = symbol_name.trim_end_matches('\0').as_bytes();
        let mut c_name_bytes = Vec::with_capacity(symbol_bytes.len() + 1);
        c_name_bytes.extend_from_slice(symbol_bytes);
        c_name_bytes.push(0);
        let c_name = unsafe { CString::from_vec_with_nul_unchecked(c_name_bytes) };

        let func: libloading::Symbol<unsafe extern "C" fn() -> *mut ()> =
            unsafe { self.lib.get(c_name.as_bytes_with_nul())? };

        let ptr = unsafe { func() };
        let boxed: Box<AnyView> = unsafe { Box::from_raw(ptr.cast()) };
        Ok(*boxed)
    }
}

async fn ensure_cached_file(path: &Path, bytes: &[u8]) -> Result<(), LoadError> {
    let parent = path
        .parent()
        .expect("cache path must have a parent directory");
    async_fs::create_dir_all(parent)
        .await
        .map_err(LoadError::Io)?;

    match async_fs::metadata(path).await {
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(LoadError::Io(e)),
    }

    let unique = format!(
        "{}.{}.{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("preview"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let temp = parent.join(unique);

    async_fs::write(&temp, bytes).await.map_err(LoadError::Io)?;
    match async_fs::rename(&temp, path).await {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            // Another process likely won the race; destination already exists.
            let _ = async_fs::remove_file(&temp).await;
            Ok(())
        }
        Err(e) => {
            let _ = async_fs::remove_file(&temp).await;
            Err(LoadError::Io(e))
        }
    }
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

#[cfg(target_os = "macos")]
async fn codesign_verify_dylib(path: &Path) -> Result<bool, LoadError> {
    let output = async_process::Command::new("codesign")
        .arg("--verify")
        .arg("--verbose=0")
        .arg(path)
        .output()
        .await
        .map_err(LoadError::Io)?;

    Ok(output.status.success())
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

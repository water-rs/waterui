//! Dynamic library loading for preview.
//!
//! Handles loading dylibs received from the CLI and resolving preview symbols.
//!
//! # Safety
//!
//! A preview dylib is loaded with `dlopen` and its preview symbols resolved by
//! name, so the `unsafe` here asserts that a symbol found under a
//! `waterui_preview_*` name really has the signature the macro generates. The
//! `Library` is kept alive for as long as any view resolved from it.

use std::ffi::CString;
use std::path::Path;
use std::time::Instant;

use waterui_core::AnyView;
use waterui_preview_protocol::{DylibId, PreviewDylibLoadTimings};

use crate::cache::preview_dylib_cache_path;

/// A loaded preview library.
#[derive(Debug)]
pub struct PreviewLibrary {
    lib: libloading::Library,
}

#[derive(Debug, Clone, Copy)]
struct BlockingDlopenTimings {
    total_ms: u64,
    closure_ms: u64,
}

impl PreviewLibrary {
    /// Load a library from an on-disk cache path.
    ///
    /// # Safety
    /// The library must be a valid `WaterUI` preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the library cannot be loaded.
    pub async unsafe fn load_from_path(
        path: &Path,
    ) -> Result<(Self, PreviewDylibLoadTimings), LoadError> {
        let (library, timings) = Self::load_with_codesign_fallback(path).await?;
        tracing::info!(
            path = %path.display(),
            elapsed_ms = timings.load_library_ms,
            "Preview library loaded from cached path"
        );
        Ok((library, timings))
    }

    /// Load a library from bytes by writing it into the preview dylib cache.
    ///
    /// # Safety
    /// The library must be a valid `WaterUI` preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the library cannot be cached or loaded.
    pub async unsafe fn load_from_bytes(
        id: DylibId,
        data: &[u8],
    ) -> Result<(Self, PreviewDylibLoadTimings), LoadError> {
        // SAFETY: preview dylib load or symbol resolution; see the module safety note.
        unsafe { Self::load_cached(id, CachedDylibSource::Bytes(data)) }.await
    }

    /// Load a library from a local filesystem path by caching it first.
    ///
    /// This avoids sending large dylib payloads over TCP while still keeping
    /// codesigning isolated from Cargo build artifacts.
    ///
    /// # Safety
    /// The library at `source_path` must be a valid `WaterUI` preview library with
    /// the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the source path cannot be cached or the library cannot be loaded.
    pub async unsafe fn load_from_local_path(
        id: DylibId,
        source_path: &Path,
    ) -> Result<(Self, PreviewDylibLoadTimings), LoadError> {
        // SAFETY: preview dylib load or symbol resolution; see the module safety note.
        unsafe { Self::load_cached(id, CachedDylibSource::File(source_path)) }.await
    }

    /// Materialize a dylib in the preview cache and load it from there.
    ///
    /// # Safety
    /// The source must be a valid `WaterUI` preview library with the expected ABI.
    async unsafe fn load_cached(
        id: DylibId,
        source: CachedDylibSource<'_>,
    ) -> Result<(Self, PreviewDylibLoadTimings), LoadError> {
        let cache_path = preview_dylib_cache_path(id);
        let cache_start = Instant::now();
        let byte_len = source.byte_len();
        ensure_cached_file(&cache_path, source).await?;
        let cache_file_ms = elapsed_ms(cache_start);
        tracing::info!(
            dylib_id = %id,
            bytes = byte_len,
            path = %cache_path.display(),
            elapsed_ms = cache_file_ms,
            "Preview library ensured dylib cache file"
        );

        // SAFETY: preview dylib load or symbol resolution; see the module safety note.
        let (library, mut timings) = unsafe { Self::load_from_path(&cache_path) }.await?;
        timings.cache_file_ms = cache_file_ms;
        Ok((library, timings))
    }

    #[cfg(target_os = "macos")]
    async fn load_with_codesign_fallback(
        path: &Path,
    ) -> Result<(Self, PreviewDylibLoadTimings), LoadError> {
        let total_start = Instant::now();
        let (first_try, first_try_timings) = dlopen_via_blocking(path).await;
        let initial_dlopen_ms = first_try_timings.total_ms;
        tracing::info!(
            path = %path.display(),
            elapsed_ms = initial_dlopen_ms,
            blocking_closure_ms = first_try_timings.closure_ms,
            blocking_wait_ms = initial_dlopen_ms.saturating_sub(first_try_timings.closure_ms),
            success = first_try.is_ok(),
            "Preview library attempted initial dlopen"
        );

        match first_try {
            Ok(lib) => Ok((
                Self { lib },
                PreviewDylibLoadTimings {
                    load_library_ms: elapsed_ms(total_start),
                    initial_dlopen_ms,
                    ..Default::default()
                },
            )),
            Err(load_error) => {
                let verify_start = Instant::now();
                let already_signed = codesign_verify_dylib(path).await?;
                let codesign_verify_ms = elapsed_ms(verify_start);
                tracing::info!(
                    path = %path.display(),
                    elapsed_ms = codesign_verify_ms,
                    already_signed,
                    "Preview library verified existing codesign state"
                );
                if already_signed {
                    return Err(load_error);
                }

                let codesign_start = Instant::now();
                codesign_dylib(path).await?;
                let codesign_ms = elapsed_ms(codesign_start);
                tracing::info!(
                    path = %path.display(),
                    elapsed_ms = codesign_ms,
                    "Preview library codesigned dylib"
                );
                let (lib, reload_timings) = dlopen_via_blocking(path).await;
                let lib = lib?;
                let reload_after_codesign_ms = reload_timings.total_ms;
                tracing::info!(
                    path = %path.display(),
                    elapsed_ms = reload_after_codesign_ms,
                    blocking_closure_ms = reload_timings.closure_ms,
                    blocking_wait_ms =
                        reload_after_codesign_ms.saturating_sub(reload_timings.closure_ms),
                    "Preview library reloaded dylib after codesign"
                );
                Ok((
                    Self { lib },
                    PreviewDylibLoadTimings {
                        load_library_ms: elapsed_ms(total_start),
                        initial_dlopen_ms,
                        codesign_verify_ms: Some(codesign_verify_ms),
                        codesign_ms: Some(codesign_ms),
                        reload_after_codesign_ms: Some(reload_after_codesign_ms),
                        ..Default::default()
                    },
                ))
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    async fn load_with_codesign_fallback(
        path: &Path,
    ) -> Result<(Self, PreviewDylibLoadTimings), LoadError> {
        let total_start = Instant::now();
        let (lib, load_timings) = dlopen_via_blocking(path).await;
        let lib = lib?;
        let initial_dlopen_ms = load_timings.total_ms;
        tracing::info!(
            path = %path.display(),
            elapsed_ms = initial_dlopen_ms,
            blocking_closure_ms = load_timings.closure_ms,
            blocking_wait_ms = initial_dlopen_ms.saturating_sub(load_timings.closure_ms),
            "Preview library loaded dylib"
        );
        Ok((
            Self { lib },
            PreviewDylibLoadTimings {
                load_library_ms: elapsed_ms(total_start),
                initial_dlopen_ms,
                ..Default::default()
            },
        ))
    }

    /// Check if the library has a symbol.
    #[must_use]
    pub fn has_symbol(&self, name: &str) -> bool {
        let Ok(c_name) = CString::new(name.trim_end_matches('\0')) else {
            return false;
        };

        // SAFETY: preview dylib load or symbol resolution; see the module safety
        // note.
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
        // SAFETY: preview dylib load or symbol resolution; see the module safety
        // note.
        let c_name = unsafe { CString::from_vec_with_nul_unchecked(c_name_bytes) };

        let func: libloading::Symbol<unsafe extern "C" fn() -> *mut ()> =
            // SAFETY: preview dylib load or symbol resolution; see the module safety
            // note.
            unsafe { self.lib.get(c_name.as_bytes_with_nul())? };

        // SAFETY: preview dylib load or symbol resolution; see the module safety
        // note.
        let ptr = unsafe { func() };
        // SAFETY: preview dylib load or symbol resolution; see the module safety
        // note.
        let boxed: Box<AnyView> = unsafe { Box::from_raw(ptr.cast()) };
        Ok(*boxed)
    }
}

async fn dlopen_via_blocking(
    path: &Path,
) -> (
    Result<libloading::Library, LoadError>,
    BlockingDlopenTimings,
) {
    let unblock_start = Instant::now();
    let (result, closure_ms) = blocking::unblock({
        let path = path.to_path_buf();
        move || {
            let closure_start = Instant::now();
            // SAFETY: preview dylib load or symbol resolution; see the module safety
            // note.
            let result = unsafe { libloading::Library::new(&path).map_err(LoadError::Library) };
            (result, elapsed_ms(closure_start))
        }
    })
    .await;

    (
        result,
        BlockingDlopenTimings {
            total_ms: elapsed_ms(unblock_start),
            closure_ms,
        },
    )
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

enum CachedDylibSource<'a> {
    Bytes(&'a [u8]),
    File(&'a Path),
}

impl CachedDylibSource<'_> {
    const fn byte_len(&self) -> Option<usize> {
        match self {
            Self::Bytes(bytes) => Some(bytes.len()),
            Self::File(_) => None,
        }
    }
}

async fn ensure_cached_file(path: &Path, source: CachedDylibSource<'_>) -> Result<(), LoadError> {
    let total_start = Instant::now();
    let parent = path
        .parent()
        .expect("cache path must have a parent directory");
    async_fs::create_dir_all(parent)
        .await
        .map_err(LoadError::Io)?;

    match async_fs::metadata(path).await {
        Ok(_) => {
            tracing::info!(
                path = %path.display(),
                bytes = source.byte_len(),
                elapsed_ms = total_start.elapsed().as_millis(),
                "Preview library cache hit"
            );
            return Ok(());
        }
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

    let write_start = Instant::now();
    match source {
        CachedDylibSource::Bytes(bytes) => {
            async_fs::write(&temp, bytes).await.map_err(LoadError::Io)?;
            tracing::info!(
                path = %temp.display(),
                bytes = bytes.len(),
                elapsed_ms = write_start.elapsed().as_millis(),
                "Preview library wrote temporary dylib cache file"
            );
        }
        CachedDylibSource::File(source_path) => {
            let source_path = source_path.to_path_buf();
            let source_path_for_copy = source_path.clone();
            let temp_path = temp.clone();
            let copied =
                blocking::unblock(move || std::fs::copy(&source_path_for_copy, &temp_path))
                    .await
                    .map_err(LoadError::Io)?;
            tracing::info!(
                source_path = %source_path.display(),
                path = %temp.display(),
                bytes = copied,
                elapsed_ms = write_start.elapsed().as_millis(),
                "Preview library copied local dylib into temporary cache file"
            );
        }
    }
    let rename_start = Instant::now();
    match async_fs::rename(&temp, path).await {
        Ok(()) => {
            tracing::info!(
                path = %path.display(),
                elapsed_ms = rename_start.elapsed().as_millis(),
                total_elapsed_ms = total_start.elapsed().as_millis(),
                "Preview library promoted temporary dylib cache file"
            );
            Ok(())
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            let _ = async_fs::remove_file(&temp).await;
            tracing::info!(
                path = %path.display(),
                elapsed_ms = rename_start.elapsed().as_millis(),
                total_elapsed_ms = total_start.elapsed().as_millis(),
                "Preview library lost cache file creation race"
            );
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
            // `libloading`'s `Display` for a failed `dlopen` is the bare words
            // "dlopen failed" — the dynamic linker's actual complaint (missing
            // symbol, bad architecture, unsigned library) lives in `Debug`.
            // Printing that is the difference between a diagnosable failure and
            // a dead end.
            Self::Library(e) => write!(f, "Library error: {e:?}"),
            Self::CodeSign(e) => write!(f, "Codesign error: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

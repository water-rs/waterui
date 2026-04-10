//! Dynamic library loading for preview.
//!
//! Handles loading dylibs received from the daemon and resolving preview symbols.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
    #[cfg(target_os = "macos")]
    pub async unsafe fn load_from_path(
        path: &Path,
        rust_sysroot: &Path,
    ) -> Result<Self, LoadError> {
        let total_start = Instant::now();
        let prepare_start = Instant::now();
        let prepared = prepare_macos_runtime_linking(path, rust_sysroot).await?;
        tracing::info!(
            path = %path.display(),
            prepared,
            elapsed_ms = prepare_start.elapsed().as_millis(),
            "Preview library prepared macOS runtime linking"
        );
        let library = Self::load_with_codesign_fallback(path).await?;
        tracing::info!(
            path = %path.display(),
            elapsed_ms = total_start.elapsed().as_millis(),
            "Preview library loaded from cached path"
        );
        Ok(library)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub async unsafe fn load_from_path(path: &Path) -> Result<Self, LoadError> {
        let total_start = Instant::now();
        let library = Self::load_with_codesign_fallback(path).await?;
        tracing::info!(
            path = %path.display(),
            elapsed_ms = total_start.elapsed().as_millis(),
            "Preview library loaded from cached path"
        );
        Ok(library)
    }

    #[cfg(target_os = "macos")]
    async fn load_with_codesign_fallback(path: &Path) -> Result<Self, LoadError> {
        let load_start = Instant::now();
        let first_try = blocking::unblock({
            let path = path.to_path_buf();
            move || unsafe { libloading::Library::new(&path).map_err(LoadError::Library) }
        })
        .await;
        tracing::info!(
            path = %path.display(),
            elapsed_ms = load_start.elapsed().as_millis(),
            success = first_try.is_ok(),
            "Preview library attempted initial dlopen"
        );

        match first_try {
            Ok(lib) => Ok(Self { lib }),
            Err(load_error) => {
                // Only codesign when the file is not already signed/valid. If it's already
                // signed and dlopen failed, surface the original error immediately.
                let verify_start = Instant::now();
                let already_signed = codesign_verify_dylib(path).await?;
                tracing::info!(
                    path = %path.display(),
                    elapsed_ms = verify_start.elapsed().as_millis(),
                    already_signed,
                    "Preview library verified existing codesign state"
                );
                if already_signed {
                    return Err(load_error);
                }

                let codesign_start = Instant::now();
                codesign_dylib(path).await?;
                tracing::info!(
                    path = %path.display(),
                    elapsed_ms = codesign_start.elapsed().as_millis(),
                    "Preview library codesigned dylib"
                );
                let reload_start = Instant::now();
                let lib = blocking::unblock({
                    let path = path.to_path_buf();
                    move || unsafe { libloading::Library::new(&path).map_err(LoadError::Library) }
                })
                .await?;
                tracing::info!(
                    path = %path.display(),
                    elapsed_ms = reload_start.elapsed().as_millis(),
                    "Preview library reloaded dylib after codesign"
                );
                Ok(Self { lib })
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    async fn load_with_codesign_fallback(path: &Path) -> Result<Self, LoadError> {
        let load_start = Instant::now();
        let lib = blocking::unblock({
            let path = path.to_path_buf();
            move || unsafe { libloading::Library::new(&path).map_err(LoadError::Library) }
        })
        .await?;
        tracing::info!(
            path = %path.display(),
            elapsed_ms = load_start.elapsed().as_millis(),
            "Preview library loaded dylib"
        );
        Ok(Self { lib })
    }

    /// Load a library from bytes by writing to a temp file.
    ///
    /// # Safety
    /// The library must be a valid WaterUI preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the library cannot be loaded.
    #[cfg(target_os = "macos")]
    pub async unsafe fn load_from_bytes(
        id: DylibId,
        data: &[u8],
        rust_sysroot: &Path,
    ) -> Result<Self, LoadError> {
        // Prefer a stable on-disk cache keyed by dylib id. This avoids re-codesigning and also
        // enables reuse across preview app restarts.
        let cache_path = preview_dylib_cache_path(id);
        let cache_start = Instant::now();
        ensure_cached_file(&cache_path, CachedDylibSource::Bytes(data)).await?;
        tracing::info!(
            dylib_id = %id,
            bytes = data.len(),
            path = %cache_path.display(),
            elapsed_ms = cache_start.elapsed().as_millis(),
            "Preview library ensured dylib cache file"
        );

        unsafe { Self::load_from_path(&cache_path, rust_sysroot) }.await
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub async unsafe fn load_from_bytes(id: DylibId, data: &[u8]) -> Result<Self, LoadError> {
        // Prefer a stable on-disk cache keyed by dylib id. This avoids re-codesigning and also
        // enables reuse across preview app restarts.
        let cache_path = preview_dylib_cache_path(id);
        let cache_start = Instant::now();
        ensure_cached_file(&cache_path, CachedDylibSource::Bytes(data)).await?;
        tracing::info!(
            dylib_id = %id,
            bytes = data.len(),
            path = %cache_path.display(),
            elapsed_ms = cache_start.elapsed().as_millis(),
            "Preview library ensured dylib cache file"
        );

        Self::load_with_codesign_fallback(&cache_path).await
    }

    /// Load a library from a local filesystem path by copying it into the preview cache first.
    ///
    /// This avoids sending large dylib payloads over TCP while still keeping codesigning isolated
    /// from Cargo build artifacts.
    ///
    /// # Safety
    /// The library at `source_path` must be a valid WaterUI preview library with the expected ABI.
    ///
    /// # Errors
    /// Returns an error if the source path cannot be cached or the library cannot be loaded.
    #[cfg(target_os = "macos")]
    pub async unsafe fn load_from_local_path(
        id: DylibId,
        source_path: &Path,
        rust_sysroot: &Path,
    ) -> Result<Self, LoadError> {
        let cache_path = preview_dylib_cache_path(id);
        let cache_start = Instant::now();
        ensure_cached_file(&cache_path, CachedDylibSource::File(source_path)).await?;
        tracing::info!(
            dylib_id = %id,
            source_path = %source_path.display(),
            cache_path = %cache_path.display(),
            elapsed_ms = cache_start.elapsed().as_millis(),
            "Preview library cached dylib from local path"
        );

        unsafe { Self::load_from_path(&cache_path, rust_sysroot) }.await
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub async unsafe fn load_from_local_path(
        id: DylibId,
        source_path: &Path,
    ) -> Result<Self, LoadError> {
        let cache_path = preview_dylib_cache_path(id);
        let cache_start = Instant::now();
        ensure_cached_file(&cache_path, CachedDylibSource::File(source_path)).await?;
        tracing::info!(
            dylib_id = %id,
            source_path = %source_path.display(),
            cache_path = %cache_path.display(),
            elapsed_ms = cache_start.elapsed().as_millis(),
            "Preview library cached dylib from local path"
        );

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

enum CachedDylibSource<'a> {
    Bytes(&'a [u8]),
    File(&'a Path),
}

impl CachedDylibSource<'_> {
    fn byte_len(&self) -> Option<usize> {
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
            // Another process likely won the race; destination already exists.
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
async fn prepare_macos_runtime_linking(
    path: &Path,
    rust_sysroot: &Path,
) -> Result<bool, LoadError> {
    let dependencies = dylib_dependencies(path).await?;
    let uses_dynamic_rust_std = dependencies
        .iter()
        .any(|dependency| dependency.starts_with("@rpath/libstd-"));
    if !uses_dynamic_rust_std {
        return Ok(false);
    }

    let Some(local_dependency) = dependencies.iter().find(|dependency| {
        dependency.ends_with(".dylib")
            && !dependency.starts_with("@rpath/")
            && !is_system_dylib_dependency(dependency)
    }) else {
        return Err(LoadError::RuntimeLink(format!(
            "dynamic preview dylib {} depends on @rpath/libstd but has no local Rust dylib dependency to infer the target triple",
            path.display()
        )));
    };

    let target_triple = infer_target_triple_from_local_dependency(Path::new(local_dependency))
        .ok_or_else(|| {
            LoadError::RuntimeLink(format!(
                "failed to infer Rust target triple from preview dylib dependency {}",
                local_dependency
            ))
        })?;
    let rust_target_libdir = rust_sysroot
        .join("lib")
        .join("rustlib")
        .join(&target_triple)
        .join("lib");
    if !rust_target_libdir.is_dir() {
        return Err(LoadError::RuntimeLink(format!(
            "Rust target library directory does not exist: {}",
            rust_target_libdir.display()
        )));
    }

    let existing_rpaths = dylib_rpaths(path).await?;
    if existing_rpaths
        .iter()
        .any(|existing| existing == &rust_target_libdir)
    {
        return Ok(false);
    }

    let patch_start = Instant::now();
    install_name_tool_add_rpath(path, &rust_target_libdir).await?;
    tracing::info!(
        path = %path.display(),
        rpath = %rust_target_libdir.display(),
        elapsed_ms = patch_start.elapsed().as_millis(),
        "Preview library added Rust stdlib rpath"
    );

    let codesign_start = Instant::now();
    codesign_dylib(path).await?;
    tracing::info!(
        path = %path.display(),
        elapsed_ms = codesign_start.elapsed().as_millis(),
        "Preview library codesigned dylib after runtime-link patch"
    );

    Ok(true)
}

#[cfg(target_os = "macos")]
async fn dylib_dependencies(path: &Path) -> Result<Vec<String>, LoadError> {
    let output = async_process::Command::new("otool")
        .arg("-L")
        .arg(path)
        .output()
        .await
        .map_err(LoadError::Io)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(LoadError::RuntimeLink(if stderr.is_empty() {
            format!("otool -L failed for {}", path.display())
        } else {
            stderr
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let dependencies = stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            trimmed
                .split_once(" (compatibility version ")
                .map(|(dependency, _)| dependency.to_string())
        })
        .collect();
    Ok(dependencies)
}

#[cfg(target_os = "macos")]
async fn dylib_rpaths(path: &Path) -> Result<Vec<PathBuf>, LoadError> {
    let output = async_process::Command::new("otool")
        .arg("-l")
        .arg(path)
        .output()
        .await
        .map_err(LoadError::Io)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(LoadError::RuntimeLink(if stderr.is_empty() {
            format!("otool -l failed for {}", path.display())
        } else {
            stderr
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut in_rpath_command = false;
    let mut rpaths = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed == "cmd LC_RPATH" {
            in_rpath_command = true;
            continue;
        }

        if in_rpath_command && trimmed.starts_with("path ") {
            let path_value = trimmed
                .trim_start_matches("path ")
                .split_once(" (offset ")
                .map_or_else(|| trimmed.trim_start_matches("path "), |(value, _)| value);
            rpaths.push(PathBuf::from(path_value));
            in_rpath_command = false;
        }
    }
    Ok(rpaths)
}

#[cfg(target_os = "macos")]
async fn install_name_tool_add_rpath(path: &Path, rpath: &Path) -> Result<(), LoadError> {
    let output = async_process::Command::new("install_name_tool")
        .arg("-add_rpath")
        .arg(rpath)
        .arg(path)
        .output()
        .await
        .map_err(LoadError::Io)?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(LoadError::RuntimeLink(if stderr.is_empty() {
        format!("install_name_tool -add_rpath failed for {}", path.display())
    } else {
        stderr
    }))
}

#[cfg(target_os = "macos")]
fn is_system_dylib_dependency(dependency: &str) -> bool {
    dependency.starts_with("/System/") || dependency.starts_with("/usr/lib/")
}

#[cfg(target_os = "macos")]
fn infer_target_triple_from_local_dependency(path: &Path) -> Option<String> {
    let deps_dir = path.parent()?;
    if deps_dir.file_name()?.to_str()? != "deps" {
        return None;
    }

    let profile_dir = deps_dir.parent()?;
    let target_triple_dir = profile_dir.parent()?;
    let target_dir = target_triple_dir.parent()?;
    if target_dir.file_name()?.to_str()? != "target" {
        return None;
    }

    Some(target_triple_dir.file_name()?.to_str()?.to_string())
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
    /// Runtime linking error while preparing dynamic dylib dependencies.
    RuntimeLink(String),
    /// Codesign error on macOS.
    CodeSign(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Library(e) => write!(f, "Library error: {e}"),
            Self::RuntimeLink(e) => write!(f, "Runtime link error: {e}"),
            Self::CodeSign(e) => write!(f, "Codesign error: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

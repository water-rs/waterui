//! Shared GPU context for efficient multi-view rendering.
//!
//! This module provides a global singleton [`SharedGpuContext`] that manages a single
//! `wgpu::Device` and `wgpu::Queue` shared across all GPU views. This eliminates the
//! expensive per-view device creation overhead and enables shared shader caching.
//!
//! # Usage
//!
//! The shared context is automatically initialized on first use. FFI code should call
//! [`init_shared_context`] during app initialization to control timing.
//!
//! ```ignore
//! use waterui_graphics::shared_context::{shared_context, init_shared_context};
//!
//! // Initialize early (optional, will auto-init on first use)
//! init_shared_context()?;
//!
//! // Get shared device/queue
//! let ctx = shared_context();
//! let guard = ctx.read();
//! let device = &guard.device;
//! ```

use fmt::Write as _;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use parking_lot::{Mutex, RwLock};
use waterkit_fs::WaterFs;
use wgpu;

/// Error type for shared context operations.
#[derive(Debug, Clone)]
pub enum SharedContextError {
    /// No suitable GPU adapter found.
    NoAdapter,
    /// Failed to create GPU device.
    DeviceCreationFailed(String),
    /// Context already initialized.
    AlreadyInitialized,
}

impl fmt::Display for SharedContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "No suitable GPU adapter found"),
            Self::DeviceCreationFailed(e) => write!(f, "Failed to create GPU device: {e}"),
            Self::AlreadyInitialized => write!(f, "Shared GPU context already initialized"),
        }
    }
}

impl Error for SharedContextError {}

/// Global shared GPU context.
///
/// Holds a single `wgpu::Device` and `wgpu::Queue` that are shared across all
/// GPU surfaces in the application. This dramatically reduces initialization
/// overhead and enables shader caching across views.
pub struct SharedGpuContext {
    /// The shared wgpu instance.
    pub instance: wgpu::Instance,
    /// The selected GPU adapter.
    pub adapter: wgpu::Adapter,
    /// The shared GPU device (thread-safe via Arc).
    pub device: Arc<wgpu::Device>,
    /// The shared GPU queue (thread-safe via Arc).
    pub queue: Arc<wgpu::Queue>,
    /// Optional pipeline cache for shader pre-warming.
    pub pipeline_cache: Option<wgpu::PipelineCache>,
    /// Pipeline cache persistence path resolved for current adapter/build.
    pipeline_cache_path: Option<PathBuf>,
    /// Cached shader modules keyed by WGSL source hash with collision buckets.
    shader_cache: parking_lot::Mutex<HashMap<u64, Vec<CachedShaderEntry>>>,
}

impl fmt::Debug for SharedGpuContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cache_size = self
            .shader_cache
            .lock()
            .values()
            .map(std::vec::Vec::len)
            .sum::<usize>();
        f.debug_struct("SharedGpuContext")
            .field("adapter", &self.adapter.get_info().name)
            .field("has_pipeline_cache", &self.pipeline_cache.is_some())
            .field("shader_cache_size", &cache_size)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct CachedShaderEntry {
    source: CachedShaderSource,
    module: Arc<wgpu::ShaderModule>,
}

#[derive(Debug)]
enum CachedShaderSource {
    Static(&'static str),
    Owned(Box<str>),
}

impl CachedShaderSource {
    fn as_str(&self) -> &str {
        match self {
            Self::Static(source) => source,
            Self::Owned(source) => source.as_ref(),
        }
    }
}

fn shader_source_hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

impl SharedGpuContext {
    /// Get a shader module from cache, or create and cache it.
    ///
    /// This method is used by the pre-warm system and renderers to get shader modules
    /// without redundant compilation.
    pub fn get_or_create_shader(&self, label: &str, source: &str) -> Arc<wgpu::ShaderModule> {
        self.get_or_create_shader_with_hash(label, source, shader_source_hash(source), None)
    }

    /// Get a static prewarmed shader from cache (or create it).
    pub fn get_or_create_static_shader(
        &self,
        shader: &'static crate::prewarm::ShaderSource,
    ) -> Arc<wgpu::ShaderModule> {
        self.get_or_create_shader_with_hash(
            shader.label,
            shader.source,
            shader.source_hash,
            Some(shader.source),
        )
    }

    /// Get shader cache statistics.
    pub fn shader_cache_stats(&self) -> (usize, usize) {
        let cache = self.shader_cache.lock();
        let cached_count = cache.values().map(std::vec::Vec::len).sum::<usize>();
        (cached_count, cached_count) // (cached_count, hit would require tracking)
    }

    fn get_or_create_shader_with_hash(
        &self,
        label: &str,
        source: &str,
        source_hash: u64,
        static_source: Option<&'static str>,
    ) -> Arc<wgpu::ShaderModule> {
        if let Some(module) = self.cached_shader_module(source_hash, source) {
            tracing::trace!("[ShaderCache] Cache HIT: {}", label);
            return module;
        }

        tracing::trace!("[ShaderCache] Cache MISS: {} - compiling", label);
        let module = Arc::new(
            self.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                }),
        );

        let mut cache = self.shader_cache.lock();
        if let Some(existing) = cache.get(&source_hash).and_then(|bucket| {
            bucket
                .iter()
                .find(|entry| entry.source.as_str() == source)
                .map(|entry| entry.module.clone())
        }) {
            return existing;
        }

        cache
            .entry(source_hash)
            .or_default()
            .push(CachedShaderEntry {
                source: match static_source {
                    Some(value) => CachedShaderSource::Static(value),
                    None => CachedShaderSource::Owned(source.to_owned().into_boxed_str()),
                },
                module: module.clone(),
            });

        module
    }

    fn cached_shader_module(
        &self,
        source_hash: u64,
        source: &str,
    ) -> Option<Arc<wgpu::ShaderModule>> {
        let cache = self.shader_cache.lock();
        cache.get(&source_hash).and_then(|bucket| {
            bucket
                .iter()
                .find(|entry| entry.source.as_str() == source)
                .map(|entry| entry.module.clone())
        })
    }
}

/// Creates a shader module with shared-source caching when the provided device
/// matches the global shared context device.
///
/// Falls back to direct module creation when no shared context is initialized
/// or when the device is not the shared device (for example, isolated tests).
pub fn create_cached_shader_module(
    device: &wgpu::Device,
    label: &str,
    source: &str,
) -> Arc<wgpu::ShaderModule> {
    if let Some(ctx) = try_shared_context() {
        let guard = ctx.read();
        if core::ptr::eq(Arc::as_ptr(&guard.device), device as *const wgpu::Device) {
            return guard.get_or_create_shader(label, source);
        }
    }

    Arc::new(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: if label.is_empty() { None } else { Some(label) },
        source: wgpu::ShaderSource::Wgsl(source.into()),
    }))
}

/// Creates a shader module from compile-time `ShaderSource` with precomputed hash.
///
/// Uses shared shader cache when `device` is the global shared device.
pub fn create_cached_shader_module_prewarmed(
    device: &wgpu::Device,
    shader: &'static crate::prewarm::ShaderSource,
) -> Arc<wgpu::ShaderModule> {
    if let Some(ctx) = try_shared_context() {
        let guard = ctx.read();
        if core::ptr::eq(Arc::as_ptr(&guard.device), device as *const wgpu::Device) {
            return guard.get_or_create_static_shader(shader);
        }
    }

    Arc::new(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: if shader.label.is_empty() {
            None
        } else {
            Some(shader.label)
        },
        source: wgpu::ShaderSource::Wgsl(shader.source.into()),
    }))
}

static SHARED_CONTEXT: OnceLock<Arc<RwLock<SharedGpuContext>>> = OnceLock::new();
static SHARED_CONTEXT_INIT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Initialize the shared GPU context.
///
/// This creates a headless wgpu instance, adapter, device, and queue that will be
/// shared across all GPU surfaces. Call this early during app initialization for
/// best performance.
///
/// # Errors
///
/// Returns an error if no suitable GPU adapter is found or device creation fails.
///
/// # Example
///
/// ```ignore
/// // During app startup
/// waterui_graphics::shared_context::init_shared_context_async().await?;
/// ```
pub async fn init_shared_context_async() -> Result<(), SharedContextError> {
    // Ensure only one thread performs the expensive device creation.
    let init_lock = SHARED_CONTEXT_INIT_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = init_lock.lock();

    if SHARED_CONTEXT.get().is_none() {
        let ctx = create_shared_context_async().await?;
        let _ = SHARED_CONTEXT.set(Arc::new(RwLock::new(ctx)));
    }

    Ok(())
}

pub fn init_shared_context() -> Result<(), SharedContextError> {
    pollster::block_on(init_shared_context_async())
}

/// Get the shared GPU context.
///
/// # Panics
///
/// Panics if initializing the shared context fails.
#[must_use]
pub fn shared_context() -> Arc<RwLock<SharedGpuContext>> {
    if SHARED_CONTEXT.get().is_none() {
        init_shared_context().expect("shared context initialization failed");
    }
    SHARED_CONTEXT
        .get()
        .expect("shared context must be initialized after successful init")
        .clone()
}

/// Try to get the shared context without initializing.
///
/// Returns `None` if the context hasn't been initialized yet.
#[must_use]
pub fn try_shared_context() -> Option<Arc<RwLock<SharedGpuContext>>> {
    SHARED_CONTEXT.get().cloned()
}

/// Check if the shared context is initialized.
#[must_use]
pub fn is_initialized() -> bool {
    SHARED_CONTEXT.get().is_some()
}

/// Save the pipeline cache to disk.
pub fn save_pipeline_cache() {
    let Some(ctx) = try_shared_context() else {
        return;
    };
    let guard = ctx.read();

    if let Some(cache) = &guard.pipeline_cache
        && let Some(data) = cache.get_data()
        && let Some(path) = &guard.pipeline_cache_path
    {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        match fs::write(path, &data) {
            Ok(_) => tracing::info!("[SharedGpuContext] Saved pipeline cache to {:?}", path),
            Err(e) => tracing::warn!("[SharedGpuContext] Failed to save pipeline cache: {}", e),
        }
    }
}

/// Delete the pipeline cache file (useful when cache is corrupted).
pub fn clear_pipeline_cache() {
    let path = try_shared_context().and_then(|ctx| {
        let guard = ctx.read();
        guard.pipeline_cache_path.clone()
    });

    if let Some(path) = path
        && path.exists()
    {
        match fs::remove_file(&path) {
            Ok(_) => tracing::info!("[SharedGpuContext] Cleared pipeline cache at {:?}", path),
            Err(e) => tracing::warn!("[SharedGpuContext] Failed to clear pipeline cache: {}", e),
        }
    }
}

fn cache_root_dir() -> Option<PathBuf> {
    WaterFs::cache_dir().map(|root| root.join("waterui"))
}

fn sanitize_cache_token(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous_was_underscore = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            let _ = out.write_char(ch.to_ascii_lowercase());
            previous_was_underscore = false;
        } else {
            if !previous_was_underscore {
                out.push('_');
                previous_was_underscore = true;
            }
        }
    }

    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed
    }
}

fn pipeline_cache_path_for_adapter(info: &wgpu::AdapterInfo) -> Option<PathBuf> {
    let backend = sanitize_cache_token(&format!("{:?}", info.backend));
    let adapter_name = sanitize_cache_token(&info.name);
    let vendor = format!("{:04x}", info.vendor);
    let device = format!("{:04x}", info.device);
    let build = option_env!("WATERUI_GRAPHICS_COMMIT").unwrap_or("unknown");

    cache_root_dir().map(|mut root| {
        root.push("pipeline");
        root.push(build);
        root.push(format!("{backend}_{vendor}_{device}_{adapter_name}.bin"));
        root
    })
}

async fn request_adapter_async(
    instance: &wgpu::Instance,
) -> Result<wgpu::Adapter, SharedContextError> {
    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .map_err(|_| SharedContextError::NoAdapter)
}

fn request_adapter(instance: &wgpu::Instance) -> Result<wgpu::Adapter, SharedContextError> {
    pollster::block_on(request_adapter_async(instance))
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AndroidBackendOverride {
    Auto,
    Vulkan,
    Gl,
}

#[cfg(target_os = "android")]
fn android_backend_override() -> AndroidBackendOverride {
    let value = std::env::var("WATERUI_ANDROID_GPU_BACKEND")
        .unwrap_or_default()
        .to_ascii_lowercase();
    match value.as_str() {
        "vulkan" | "vk" => AndroidBackendOverride::Vulkan,
        "gl" | "gles" | "opengl" => AndroidBackendOverride::Gl,
        _ => AndroidBackendOverride::Auto,
    }
}

#[cfg(target_os = "android")]
fn is_problematic_android_vulkan_adapter(info: &wgpu::AdapterInfo) -> bool {
    if info.backend != wgpu::Backend::Vulkan {
        return false;
    }

    if info.device_type == wgpu::DeviceType::Cpu {
        return true;
    }

    let name = info.name.to_ascii_lowercase();
    name.contains("swiftshader")
        || name.contains("llvmpipe")
        || name.contains("lavapipe")
        || name.contains("gfxstream")
}

#[cfg(target_os = "android")]
async fn request_android_adapter_with_backends_async(
    backends: wgpu::Backends,
) -> Result<(wgpu::Instance, wgpu::Adapter), SharedContextError> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });
    let adapter = request_adapter_async(&instance).await?;
    Ok((instance, adapter))
}

#[cfg(target_os = "android")]
fn request_android_adapter_with_backends(
    backends: wgpu::Backends,
) -> Result<(wgpu::Instance, wgpu::Adapter), SharedContextError> {
    pollster::block_on(request_android_adapter_with_backends_async(backends))
}

#[cfg(target_os = "android")]
async fn create_android_instance_and_adapter_async()
-> Result<(wgpu::Instance, wgpu::Adapter), SharedContextError> {
    match android_backend_override() {
        AndroidBackendOverride::Vulkan => {
            tracing::info!(
                "[SharedGpuContext] Android backend override: forcing Vulkan (WATERUI_ANDROID_GPU_BACKEND=vulkan)"
            );
            return request_android_adapter_with_backends_async(wgpu::Backends::VULKAN).await;
        }
        AndroidBackendOverride::Gl => {
            tracing::info!(
                "[SharedGpuContext] Android backend override: forcing OpenGL (WATERUI_ANDROID_GPU_BACKEND=gl)"
            );
            return request_android_adapter_with_backends_async(wgpu::Backends::GL).await;
        }
        AndroidBackendOverride::Auto => {}
    }

    let (vk_instance, vk_adapter) =
        request_android_adapter_with_backends_async(wgpu::Backends::VULKAN).await?;
    let vk_info = vk_adapter.get_info();

    if is_problematic_android_vulkan_adapter(&vk_info) {
        tracing::warn!(
            "[SharedGpuContext] Vulkan adapter '{}' appears to be CPU/SwiftShader on Android; trying OpenGL to avoid emulator vkCreateGraphicsPipelines stalls",
            vk_info.name
        );

        if let Ok((gl_instance, gl_adapter)) =
            request_android_adapter_with_backends_async(wgpu::Backends::GL).await
        {
            let gl_info = gl_adapter.get_info();
            tracing::info!(
                "[SharedGpuContext] Using OpenGL adapter '{}' on Android",
                gl_info.name
            );
            return Ok((gl_instance, gl_adapter));
        }

        tracing::warn!(
            "[SharedGpuContext] OpenGL fallback unavailable; continuing with Vulkan adapter '{}'",
            vk_info.name
        );
    }

    Ok((vk_instance, vk_adapter))
}

#[cfg(target_os = "android")]
fn create_android_instance_and_adapter()
-> Result<(wgpu::Instance, wgpu::Adapter), SharedContextError> {
    pollster::block_on(create_android_instance_and_adapter_async())
}

/// Create a new shared context (internal).
async fn create_shared_context_async() -> Result<SharedGpuContext, SharedContextError> {
    tracing::info!("[SharedGpuContext] Initializing shared GPU context");

    let (instance, adapter) = {
        #[cfg(target_os = "android")]
        {
            create_android_instance_and_adapter_async().await?
        }

        #[cfg(not(target_os = "android"))]
        {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });
            let adapter = request_adapter_async(&instance).await?;
            (instance, adapter)
        }
    };

    let adapter_info = adapter.get_info();
    tracing::info!(
        "[SharedGpuContext] Selected adapter: {} ({:?})",
        adapter_info.name,
        adapter_info.backend
    );
    let pipeline_cache_path = pipeline_cache_path_for_adapter(&adapter_info);

    // Determine appropriate limits
    let adapter_limits = adapter.limits();
    let downlevel_caps = adapter.get_downlevel_capabilities();
    let required_limits = if downlevel_caps.is_webgpu_compliant() {
        wgpu::Limits::default()
    } else if downlevel_caps
        .flags
        .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
    {
        wgpu::Limits::downlevel_defaults()
    } else {
        wgpu::Limits::downlevel_webgl2_defaults()
    }
    .using_resolution(adapter_limits.clone())
    .using_alignment(adapter_limits);

    // Determine features to request (pipeline cache if available)
    let adapter_features = adapter.features();
    let mut required_features = wgpu::Features::empty();
    if cfg!(target_os = "android") {
        tracing::info!(
            "[SharedGpuContext] PIPELINE_CACHE disabled on Android to avoid driver stalls"
        );
    } else if adapter_features.contains(wgpu::Features::PIPELINE_CACHE) {
        required_features |= wgpu::Features::PIPELINE_CACHE;
        tracing::info!("[SharedGpuContext] PIPELINE_CACHE feature available and requested");
    } else {
        tracing::info!("[SharedGpuContext] PIPELINE_CACHE feature not available on this adapter");
    }

    // Request device
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("WaterUI Shared Device"),
            required_features,
            required_limits,
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            trace: wgpu::Trace::default(),
        })
        .await
        .map_err(|e| SharedContextError::DeviceCreationFailed(e.to_string()))?;

    // Set error handler
    device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
        tracing::error!("[wgpu] Validation error: {error}");
    }));

    // Create pipeline cache only if the feature is available
    let pipeline_cache = if device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        // Try to load cache from disk, but don't fail if it's corrupted
        let cache_data = pipeline_cache_path
            .as_ref()
            .and_then(|path| match fs::read(path) {
                Ok(data) => {
                    tracing::info!(
                        "[SharedGpuContext] Loaded pipeline cache from {:?} ({} bytes)",
                        path,
                        data.len()
                    );
                    Some(data)
                }
                Err(_) => {
                    tracing::debug!("[SharedGpuContext] No existing pipeline cache found");
                    None
                }
            });

        // SAFETY: We're providing valid PipelineCacheDescriptor with fallback enabled
        // If the cache data is invalid, wgpu will create an empty cache instead of failing
        let cache = unsafe {
            device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: Some("WaterUI Pipeline Cache"),
                data: cache_data.as_deref(),
                fallback: true, // Important: create empty cache if data is invalid
            })
        };
        tracing::info!("[SharedGpuContext] Pipeline cache created");
        Some(cache)
    } else {
        tracing::info!("[SharedGpuContext] Pipeline cache not available (feature not supported)");
        None
    };

    tracing::info!("[SharedGpuContext] Initialized successfully");

    Ok(SharedGpuContext {
        instance,
        adapter,
        device: Arc::new(device),
        queue: Arc::new(queue),
        pipeline_cache,
        pipeline_cache_path,
        shader_cache: parking_lot::Mutex::new(HashMap::new()),
    })
}

fn create_shared_context() -> Result<SharedGpuContext, SharedContextError> {
    pollster::block_on(create_shared_context_async())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_context_creation() {
        // This will fail in CI without a GPU, but works locally
        if let Ok(()) = init_shared_context() {
            assert!(is_initialized());
            let ctx = shared_context();
            let guard = ctx.read();
            assert!(!guard.adapter.get_info().name.is_empty());
        }
    }

    #[test]
    fn test_save_pipeline_cache() {
        // Only run if we can initialize context
        if init_shared_context().is_ok() || is_initialized() {
            save_pipeline_cache();
            let path = try_shared_context().and_then(|ctx| {
                let guard = ctx.read();
                guard.pipeline_cache_path.clone()
            });

            if let Some(path) = path
                && path.exists()
            {
                let metadata = fs::metadata(&path).expect("Failed to read cache metadata");
                assert!(metadata.is_file());
                // Cleanup
                let _ = fs::remove_file(path);
            }
        }
    }
}

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

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;
use wgpu;
use std::path::PathBuf;
use std::fs;

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

impl std::fmt::Display for SharedContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "No suitable GPU adapter found"),
            Self::DeviceCreationFailed(e) => write!(f, "Failed to create GPU device: {e}"),
            Self::AlreadyInitialized => write!(f, "Shared GPU context already initialized"),
        }
    }
}

impl std::error::Error for SharedContextError {}

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
    /// Cached shader modules keyed by label.
    shader_cache: parking_lot::Mutex<HashMap<&'static str, Arc<wgpu::ShaderModule>>>,
}

impl std::fmt::Debug for SharedGpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cache_size = self.shader_cache.lock().len();
        f.debug_struct("SharedGpuContext")
            .field("adapter", &self.adapter.get_info().name)
            .field("has_pipeline_cache", &self.pipeline_cache.is_some())
            .field("shader_cache_size", &cache_size)
            .finish_non_exhaustive()
    }
}

impl SharedGpuContext {
    /// Get a shader module from cache, or create and cache it.
    ///
    /// This method is used by the pre-warm system and renderers to get shader modules
    /// without redundant compilation.
    pub fn get_or_create_shader(
        &self,
        label: &'static str,
        source: &str,
    ) -> Arc<wgpu::ShaderModule> {
        let mut cache = self.shader_cache.lock();

        if let Some(module) = cache.get(label) {
            tracing::trace!("[ShaderCache] Cache HIT: {}", label);
            return module.clone();
        }

        tracing::trace!("[ShaderCache] Cache MISS: {} - compiling", label);
        let module = Arc::new(self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        }));

        cache.insert(label, module.clone());
        module
    }

    /// Get shader cache statistics.
    pub fn shader_cache_stats(&self) -> (usize, usize) {
        let cache = self.shader_cache.lock();
        (cache.len(), cache.len()) // (cached_count, hit would require tracking)
    }
}

static SHARED_CONTEXT: OnceLock<Arc<RwLock<SharedGpuContext>>> = OnceLock::new();

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
/// waterui_graphics::shared_context::init_shared_context()?;
/// ```
pub fn init_shared_context() -> Result<(), SharedContextError> {
    if SHARED_CONTEXT.get().is_some() {
        return Err(SharedContextError::AlreadyInitialized);
    }

    let ctx = create_shared_context()?;
    
    // Try to set it; if another thread beat us, that's fine
    let _ = SHARED_CONTEXT.set(Arc::new(RwLock::new(ctx)));
    
    Ok(())
}

/// Get the shared GPU context.
///
/// Initializes the context on first call if not already initialized.
///
/// # Panics
///
/// Panics if context initialization fails (no suitable GPU found).
#[must_use]
pub fn shared_context() -> Arc<RwLock<SharedGpuContext>> {
    SHARED_CONTEXT
        .get_or_init(|| {
            match create_shared_context() {
                Ok(ctx) => Arc::new(RwLock::new(ctx)),
                Err(e) => panic!("Failed to initialize shared GPU context: {e}"),
            }
        })
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
    let Some(ctx) = try_shared_context() else { return };
    let guard = ctx.read();
    
    if let Some(cache) = &guard.pipeline_cache {
        if let Some(data) = cache.get_data() {
            if let Some(path) = get_cache_path() {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                
                match fs::write(&path, &data) {
                    Ok(_) => tracing::info!("[SharedGpuContext] Saved pipeline cache to {:?}", path),
                    Err(e) => tracing::warn!("[SharedGpuContext] Failed to save pipeline cache: {}", e),
                }
            }
        }
    }
}

fn get_cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|mut p| {
        p.push("waterui");
        p.push("gpu_cache.bin");
        p
    })
}

/// Create a new shared context (internal).
fn create_shared_context() -> Result<SharedGpuContext, SharedContextError> {
    tracing::info!("[SharedGpuContext] Initializing shared GPU context");

    // Create instance with all backends on desktop, specific backends on mobile
    let backends = if cfg!(target_os = "android") {
        // Try Vulkan first on Android, then GL
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    } else {
        wgpu::Backends::all()
    };

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });

    // Request headless adapter (compatible with any surface)
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None, // Headless mode
        force_fallback_adapter: false,
    }))
    .map_err(|_| SharedContextError::NoAdapter)?;

    let adapter_info = adapter.get_info();
    tracing::info!(
        "[SharedGpuContext] Selected adapter: {} ({:?})",
        adapter_info.name,
        adapter_info.backend
    );

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

    // Request device
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("WaterUI Shared Device"),
        required_features: wgpu::Features::empty(),
        required_limits,
        memory_hints: wgpu::MemoryHints::Performance,
        experimental_features: wgpu::ExperimentalFeatures::default(),
        trace: wgpu::Trace::default(),
    }))
    .map_err(|e| SharedContextError::DeviceCreationFailed(e.to_string()))?;

    // Set error handler
    device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
        tracing::error!("[wgpu] Validation error: {error}");
    }));

    // Create pipeline cache (may not be supported on all backends)
    
    // Try to load cache from disk
    let cache_data = get_cache_path().and_then(|path| {
        match fs::read(&path) {
            Ok(data) => {
                tracing::info!("[SharedGpuContext] Loaded pipeline cache from {:?}", path);
                Some(data)
            },
            Err(_) => None,
        }
    });

    // SAFETY: We're providing valid PipelineCacheDescriptor
    let pipeline_cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("WaterUI Pipeline Cache"),
            data: cache_data.as_deref(),
            fallback: true,
        })
    };

    tracing::info!("[SharedGpuContext] Initialized successfully");

    Ok(SharedGpuContext {
        instance,
        adapter,
        device: Arc::new(device),
        queue: Arc::new(queue),
        pipeline_cache: Some(pipeline_cache),
        shader_cache: parking_lot::Mutex::new(HashMap::new()),
    })
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
            // Ensure we have a clean slate if possible, or just overwrite
            save_pipeline_cache();
            
            if let Some(path) = get_cache_path() {
                assert!(path.exists(), "Cache file should exist after save");
                let metadata = fs::metadata(&path).expect("Failed to read cache metadata");
                // Size might be 0 if cache is empty, but file should exist
                println!("Cache saved to: {:?} (size: {} bytes)", path, metadata.len());
                
                // Cleanup
                let _ = fs::remove_file(path);
            }
        }
    }
}

//! Explicit GPU runtime ownership for shared-device rendering.
//!
//! [`GpuRuntime`] owns one adapter/device/queue context and its device-bound
//! shader and pipeline caches. Applications create it asynchronously and clone
//! the lightweight owner wherever GPU-backed views share that device.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;

use filtrate::ShaderCache;

/// Error type for GPU runtime creation.
#[derive(Debug, Clone)]
pub enum SharedContextError {
    /// No suitable GPU adapter was found.
    NoAdapter,
    /// GPU device creation failed.
    DeviceCreationFailed(String),
    /// Pipeline-cache creation failed validation on the selected device.
    PipelineCacheCreationFailed(String),
}

impl fmt::Display for SharedContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => formatter.write_str("no suitable GPU adapter found"),
            Self::DeviceCreationFailed(error) => {
                write!(formatter, "failed to create GPU device: {error}")
            }
            Self::PipelineCacheCreationFailed(error) => {
                write!(formatter, "GPU pipeline cache creation failed: {error}")
            }
        }
    }
}

impl Error for SharedContextError {}

/// GPU resources shared by every clone of one [`GpuRuntime`].
pub struct SharedGpuContext {
    /// The shared wgpu instance.
    pub instance: wgpu::Instance,
    /// The selected GPU adapter.
    pub adapter: wgpu::Adapter,
    /// The shared GPU device.
    pub device: Arc<wgpu::Device>,
    /// The shared GPU queue.
    pub queue: Arc<wgpu::Queue>,
    pipeline_cache: Option<wgpu::PipelineCache>,
    shader_cache: ShaderCache,
    submission_completion_driver: GpuSubmissionCompletionDriver,
}

impl fmt::Debug for SharedGpuContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedGpuContext")
            .field("adapter", &self.adapter.get_info().name)
            .field("has_pipeline_cache", &self.pipeline_cache.is_some())
            .finish_non_exhaustive()
    }
}

impl SharedGpuContext {
    async fn new() -> Result<Self, SharedContextError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| SharedContextError::NoAdapter)?;

        let adapter_features = adapter.features();
        let pipeline_cache_features = if cfg!(target_os = "android") {
            wgpu::Features::empty()
        } else {
            adapter_features & wgpu::Features::PIPELINE_CACHE
        };
        let required_features = required_media_features(adapter_features) | pipeline_cache_features;
        let required_limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("WaterUI GPU runtime device"),
                required_features,
                required_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::default(),
            })
            .await
            .map_err(|error| SharedContextError::DeviceCreationFailed(error.to_string()))?;

        let pipeline_cache = if pipeline_cache_features.contains(wgpu::Features::PIPELINE_CACHE) {
            Some(create_pipeline_cache(&device).await?)
        } else {
            None
        };
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let submission_completion_driver =
            GpuSubmissionCompletionDriver::new(Arc::clone(&device), Arc::clone(&queue));

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            pipeline_cache,
            shader_cache: ShaderCache::new(),
            submission_completion_driver,
        })
    }

    /// Returns the device-local pipeline cache when supported by this runtime.
    #[must_use]
    pub const fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> {
        self.pipeline_cache.as_ref()
    }

    /// Returns the shader-module cache bound to this runtime's device.
    #[must_use]
    pub const fn shader_cache(&self) -> &ShaderCache {
        &self.shader_cache
    }

    /// Returns the driver that resolves exact GPU-submission completion fences.
    #[must_use]
    #[doc(hidden)]
    pub fn submission_completion_driver(&self) -> GpuSubmissionCompletionDriver {
        self.submission_completion_driver.clone()
    }
}

/// Returns the adapter features required by `WaterUI`'s GPU media pipeline.
///
/// Every device hosting a GPU surface requests these features so decoded HDR
/// planes retain their native precision.
///
/// # Panics
///
/// Panics on Apple when the adapter cannot provide normalized 16-bit textures.
#[must_use]
pub fn required_media_features(adapter_features: wgpu::Features) -> wgpu::Features {
    if cfg!(target_vendor = "apple") {
        assert!(
            adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM),
            "WaterUI's Apple GPU backend requires normalized 16-bit textures for HDR media"
        );
        wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
    } else {
        wgpu::Features::empty()
    }
}

/// Cloneable owner for one explicitly-created shared GPU context.
#[derive(Clone)]
pub struct GpuRuntime {
    context: Arc<SharedGpuContext>,
}

impl fmt::Debug for GpuRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuRuntime")
            .field("context", &self.context)
            .finish()
    }
}

impl GpuRuntime {
    /// Creates an independent GPU runtime and prewarms built-in shader modules.
    ///
    /// # Errors
    ///
    /// Returns the adapter, device, or pipeline-cache initialization error.
    pub async fn new() -> Result<Self, SharedContextError> {
        let runtime = Self {
            context: Arc::new(SharedGpuContext::new().await?),
        };
        crate::prewarm::prewarm_builtin_shaders(&runtime);
        Ok(runtime)
    }

    /// Returns this runtime's shared GPU resources.
    #[must_use]
    pub fn context(&self) -> &SharedGpuContext {
        self.context.as_ref()
    }
}

#[cfg(not(target_arch = "wasm32"))]
type SubmissionCompletion = (wgpu::SubmissionIndex, Box<dyn FnOnce() + Send>);

/// Serial completion driver for submissions on one GPU device.
///
/// Native runtimes confine submission waits to one dedicated thread. WebGPU
/// runtimes use the browser-driven queue completion callback.
#[derive(Clone)]
#[doc(hidden)]
pub struct GpuSubmissionCompletionDriver {
    #[cfg(not(target_arch = "wasm32"))]
    sender: mpsc::Sender<SubmissionCompletion>,
    #[cfg(target_arch = "wasm32")]
    queue: Arc<wgpu::Queue>,
}

impl fmt::Debug for GpuSubmissionCompletionDriver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuSubmissionCompletionDriver")
            .finish_non_exhaustive()
    }
}

impl GpuSubmissionCompletionDriver {
    #[cfg(not(target_arch = "wasm32"))]
    fn new(device: Arc<wgpu::Device>, _queue: Arc<wgpu::Queue>) -> Self {
        let (sender, receiver) = mpsc::channel::<SubmissionCompletion>();
        std::thread::Builder::new()
            .name("waterui-gpu-completion".to_owned())
            .spawn(move || {
                while let Ok((submission, completion)) = receiver.recv() {
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: None,
                        })
                        .expect("GPU submission completion failed");
                    completion();
                }
            })
            .expect("failed to start the GPU submission completion driver");
        Self { sender }
    }

    #[cfg(target_arch = "wasm32")]
    fn new(_device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self { queue }
    }

    /// Runs `completion` after the specified submission finishes.
    ///
    /// # Panics
    ///
    /// Panics when this runtime's completion driver has stopped unexpectedly.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn on_complete(
        &self,
        submission: wgpu::SubmissionIndex,
        completion: impl FnOnce() + Send + 'static,
    ) {
        self.sender
            .send((submission, Box::new(completion)))
            .expect("GPU submission completion driver stopped unexpectedly");
    }

    /// Runs `completion` after all work submitted through this queue has finished.
    #[cfg(target_arch = "wasm32")]
    pub fn on_complete(
        &self,
        _submission: wgpu::SubmissionIndex,
        completion: impl FnOnce() + Send + 'static,
    ) {
        self.queue.on_submitted_work_done(completion);
    }
}

async fn create_pipeline_cache(
    device: &wgpu::Device,
) -> Result<wgpu::PipelineCache, SharedContextError> {
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    // SAFETY: no external cache data is supplied, and validation is awaited
    // before the device-local cache is exposed to callers.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("WaterUI pipeline cache"),
            data: None,
            fallback: false,
        })
    };
    if let Some(error) = device.pop_error_scope().await {
        return Err(SharedContextError::PipelineCacheCreationFailed(
            error.to_string(),
        ));
    }

    Ok(cache)
}

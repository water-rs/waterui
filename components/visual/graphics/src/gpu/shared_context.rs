//! Explicit GPU runtime ownership for shared-device rendering.
//!
//! [`GpuRuntime`] owns one adapter/device/queue context and its device-bound
//! shader-module cache. Applications create it asynchronously and clone
//! the lightweight owner wherever GPU-backed views share that device.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;

/// Error type for GPU runtime creation.
#[derive(Debug, Clone)]
pub enum SharedContextError {
    /// No suitable GPU adapter was found.
    NoAdapter,
    /// GPU device creation failed.
    DeviceCreationFailed(String),
}

impl fmt::Display for SharedContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => formatter.write_str("no suitable GPU adapter found"),
            Self::DeviceCreationFailed(error) => {
                write!(formatter, "failed to create GPU device: {error}")
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
    submission_completion_driver: GpuSubmissionCompletionDriver,
}

impl fmt::Debug for SharedGpuContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedGpuContext")
            .field("adapter", &self.adapter.get_info().name)
            .finish_non_exhaustive()
    }
}

impl SharedGpuContext {
    async fn new() -> Result<Self, SharedContextError> {
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = wgpu::Backends::all();
        let instance = wgpu::Instance::new(instance_descriptor);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| SharedContextError::NoAdapter)?;

        let adapter_features = adapter.features();
        let required_features = required_media_features(adapter_features);
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

        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let submission_completion_driver =
            GpuSubmissionCompletionDriver::new(Arc::clone(&device), Arc::clone(&queue));

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            submission_completion_driver,
        })
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
    let mut required = shaderloom::required_features(adapter_features);

    if cfg!(target_vendor = "apple") {
        assert!(
            adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM),
            "WaterUI's Apple GPU backend requires normalized 16-bit textures for HDR media"
        );
        assert!(
            adapter_features.contains(wgpu::Features::PASSTHROUGH_SHADERS),
            "WaterUI's Apple GPU backend requires native shader passthrough for embedded MetalLib artifacts"
        );
        required |= wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
    }

    required
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
    /// Creates an independent GPU runtime.
    ///
    /// # Errors
    ///
    /// Returns the adapter or device initialization error.
    pub async fn new() -> Result<Self, SharedContextError> {
        Ok(Self {
            context: Arc::new(SharedGpuContext::new().await?),
        })
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

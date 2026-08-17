//! Explicit GPU runtime ownership for shared-device rendering.
//!
//! [`GpuRuntime`] owns one adapter/device/queue context and its device-bound
//! shader-module cache. Applications create it asynchronously and clone
//! the lightweight owner wherever GPU-backed views share that device.

use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use shaderloom::WgslModuleCache;

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
    /// Module cache for shaders assembled at runtime on this device.
    ///
    /// Every GPU-backed view on this runtime shares it, so two views that
    /// produce byte-identical WGSL compile it once.
    pub shader_cache: Arc<WgslModuleCache>,
    /// The scene renderer every vector-drawing view on this device shares.
    ///
    /// A renderer is a device-level resource: it owns the pipelines that
    /// rasterize a scene, and those depend on the device rather than on what is
    /// being drawn. Building one per view made an application with a dozen
    /// icons build a dozen renderers, compile a dozen copies of the same
    /// pipelines, and reach a first frame a dozen separate times — which is
    /// what icons appearing one after another looks like.
    ///
    /// Built on first use, so an application that draws no vector content never
    /// builds one at all.
    scene_renderer: Arc<SharedSceneRenderer>,
    submission_completion_driver: GpuSubmissionCompletionDriver,
}

/// Which engine rasterizes scenes on a given device.
///
/// Decided from what the adapter reports, once, when the device is created —
/// not from a failure at draw time. A device that cannot run the classic
/// pipeline should never be asked to try it: doing so aborts the process rather
/// than degrading, because the error surfaces inside wgpu's default handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneEngine {
    /// The compute pipeline. Needs indirect execution; the faster of the two.
    Classic,
    /// Paths on the CPU, rasterization on the GPU. Needs no indirect execution,
    /// which is what makes it the one that runs on the iOS Simulator.
    Hybrid,
}

impl SceneEngine {
    /// The engine this adapter can actually run.
    #[must_use]
    pub fn for_adapter(adapter: &wgpu::Adapter) -> Self {
        if adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION)
        {
            Self::Classic
        } else {
            tracing::info!(
                adapter = %adapter.get_info().name,
                "adapter has no indirect execution; scenes render through the hybrid engine"
            );
            Self::Hybrid
        }
    }
}

/// One device's scene renderer, built the first time something draws a scene.
///
/// A renderer owns the pipelines that rasterize a scene; those depend on the
/// device rather than on what is being drawn, so every vector-drawing view on a
/// device shares this one. Building one per view made an application with a
/// dozen icons compile a dozen copies of the same pipelines and reach a first
/// frame a dozen separate times.
pub struct SharedSceneRenderer {
    engine: SceneEngine,
    classic: OnceLock<Mutex<vello::Renderer>>,
    // The hybrid engine rasterizes through a render pipeline, so its pipelines
    // are built for one target format; a device that also renders offscreen in
    // another format needs a second one. There are never more than a couple, so
    // they are looked up by scanning rather than hashed.
    hybrid: Mutex<Vec<(wgpu::TextureFormat, HybridRenderer)>>,
}

/// The hybrid renderer and the resources it draws with.
///
/// They are created together and used together, so they are kept together.
#[derive(Debug)]
pub struct HybridRenderer {
    /// The renderer itself.
    pub renderer: vello_hybrid::Renderer,
    /// Its atlas and buffer resources.
    pub resources: vello_hybrid::Resources,
}

impl Default for SharedSceneRenderer {
    fn default() -> Self {
        Self::new(SceneEngine::Classic)
    }
}

impl fmt::Debug for SharedSceneRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedSceneRenderer")
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

impl SharedSceneRenderer {
    /// A renderer for `engine`, built when something first draws a scene.
    #[must_use]
    pub const fn new(engine: SceneEngine) -> Self {
        Self {
            engine,
            classic: OnceLock::new(),
            hybrid: Mutex::new(Vec::new()),
        }
    }

    /// Which engine this device rasterizes scenes with.
    #[must_use]
    pub const fn engine(&self) -> SceneEngine {
        self.engine
    }

    /// Runs `use_renderer` against the classic renderer, building it if needed.
    ///
    /// # Panics
    ///
    /// Panics when the renderer cannot be built, or when this device runs the
    /// hybrid engine — a caller that reaches here on such a device has skipped
    /// the [`Self::engine`] check.
    pub fn with_classic<R>(
        &self,
        device: &wgpu::Device,
        use_renderer: impl FnOnce(&mut vello::Renderer) -> R,
    ) -> R {
        assert_eq!(
            self.engine,
            SceneEngine::Classic,
            "this device rasterizes scenes with the hybrid engine"
        );
        let renderer = self.classic.get_or_init(|| {
            Mutex::new(
                vello::Renderer::new(
                    device,
                    vello::RendererOptions {
                        use_cpu: false,
                        antialiasing_support: vello::AaSupport::area_only(),
                        num_init_threads: std::num::NonZeroUsize::new(1),
                        pipeline_cache: None,
                    },
                )
                .expect("the GPU device cannot rasterize vector scenes"),
            )
        });
        let mut guard = renderer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        use_renderer(&mut guard)
    }

    /// Runs `use_renderer` against the hybrid renderer, building it if needed.
    ///
    /// # Panics
    ///
    /// Panics when this device runs the classic engine.
    pub fn with_hybrid<R>(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        use_renderer: impl FnOnce(&mut HybridRenderer) -> R,
    ) -> R {
        assert_eq!(
            self.engine,
            SceneEngine::Hybrid,
            "this device rasterizes scenes with the classic engine"
        );
        let mut renderers = self
            .hybrid
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = renderers
            .iter()
            .position(|(candidate, _)| *candidate == format)
            .unwrap_or_else(|| {
                let (renderer, resources) = vello_hybrid::Renderer::new(
                    device,
                    &vello_hybrid::RenderTargetConfig {
                        format,
                        // Sized per render; every render is handed the target's
                        // own size, so the one given here only has to be valid.
                        width: 1,
                        height: 1,
                    },
                );
                renderers.push((
                    format,
                    HybridRenderer {
                        renderer,
                        resources,
                    },
                ));
                renderers.len() - 1
            });
        use_renderer(&mut renderers[index].1)
    }
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
        let (instance, adapter) = request_instance_and_adapter().await?;
        let scene_engine = SceneEngine::for_adapter(&adapter);
        let adapter_info = adapter.get_info();
        tracing::debug!(
            name = %adapter_info.name,
            backend = ?adapter_info.backend,
            device_type = ?adapter_info.device_type,
            "selected WaterUI GPU adapter"
        );

        let adapter_features = adapter.features();
        let required_features = required_media_features(adapter_features);
        let required_limits = required_device_limits(&adapter.limits());
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

        // Device loss otherwise surfaces only as a bare `Validation` status on the
        // next swapchain acquire, with the reason discarded; log it at the moment
        // it happens so the failure names its cause.
        device.set_device_lost_callback(|reason, message| {
            tracing::error!(?reason, message, "WaterUI GPU runtime device was lost");
        });

        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let submission_completion_driver =
            GpuSubmissionCompletionDriver::new(Arc::clone(&device), Arc::clone(&queue));

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            shader_cache: Arc::new(WgslModuleCache::new()),
            scene_renderer: Arc::new(SharedSceneRenderer::new(scene_engine)),
            submission_completion_driver,
        })
    }

    /// The scene renderer every vector-drawing view on this device shares.
    #[must_use]
    pub const fn scene_renderer(&self) -> &Arc<SharedSceneRenderer> {
        &self.scene_renderer
    }

    /// Returns the driver that resolves exact GPU-submission completion fences.
    #[must_use]
    #[doc(hidden)]
    pub fn submission_completion_driver(&self) -> GpuSubmissionCompletionDriver {
        self.submission_completion_driver.clone()
    }
}

async fn request_adapter(
    instance: &wgpu::Instance,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
}

#[cfg(target_os = "android")]
async fn request_instance_and_adapter()
-> Result<(wgpu::Instance, wgpu::Adapter), SharedContextError> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN.with_env();
    let instance = wgpu::Instance::new(descriptor);
    let adapter = request_adapter(&instance)
        .await
        .map_err(|_| SharedContextError::NoAdapter)?;
    Ok((instance, adapter))
}

#[cfg(not(target_os = "android"))]
async fn request_instance_and_adapter()
-> Result<(wgpu::Instance, wgpu::Adapter), SharedContextError> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::all();
    let instance = wgpu::Instance::new(descriptor);
    let adapter = request_adapter(&instance)
        .await
        .map_err(|_| SharedContextError::NoAdapter)?;
    Ok((instance, adapter))
}

fn required_device_limits(adapter_limits: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits::default()
        .or_worse_values_from(adapter_limits)
        .using_resolution(adapter_limits.clone())
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

#[cfg(test)]
mod tests {
    use super::required_device_limits;

    #[test]
    fn device_limits_clamp_compute_capabilities_to_adapter() {
        let adapter_limits = wgpu::Limits {
            max_texture_dimension_2d: 16_384,
            max_compute_workgroup_storage_size: 0,
            max_compute_invocations_per_workgroup: 0,
            max_compute_workgroup_size_x: 0,
            max_compute_workgroup_size_y: 0,
            max_compute_workgroup_size_z: 0,
            max_compute_workgroups_per_dimension: 0,
            ..wgpu::Limits::default()
        };

        let required = required_device_limits(&adapter_limits);

        assert_eq!(required.max_texture_dimension_2d, 16_384);
        assert_eq!(required.max_compute_workgroup_storage_size, 0);
        assert_eq!(required.max_compute_invocations_per_workgroup, 0);
        assert_eq!(required.max_compute_workgroup_size_x, 0);
        assert_eq!(required.max_compute_workgroup_size_y, 0);
        assert_eq!(required.max_compute_workgroup_size_z, 0);
        assert_eq!(required.max_compute_workgroups_per_dimension, 0);
        assert!(required.check_limits(&adapter_limits));
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

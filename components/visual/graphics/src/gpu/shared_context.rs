//! Explicit GPU runtime ownership for shared-device rendering.
//!
//! [`GpuRuntime`] owns one adapter/device/queue context and its device-bound
//! shader-module cache. Applications create it asynchronously and clone
//! the lightweight owner wherever GPU-backed views share that device.

use std::error::Error;
use std::fmt;
use std::mem::ManuallyDrop;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use shaderloom::WgslModuleCache;

use crate::scene2d_hybrid::HybridImageAtlas;
pub use crate::scene2d_hybrid::HybridRenderer;

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
///
/// All of it is device-bound: when the driver reports the device lost, the
/// owning [`GpuRuntime`] replaces the whole context — surfaces, pipelines,
/// caches and scene renderers are recreated against the fresh device — and
/// [`GpuRuntime::context`] hands the replacement out from then on. Callers
/// compare [`Self::generation`] against the generation their device-bound
/// resources were built under to know when to rebuild them.
pub struct SharedGpuContext {
    /// Which recreation this context is — `0` for the initial one, increasing
    /// by one for every device-loss rebuild the owning runtime performs.
    generation: u64,
    /// The shared wgpu instance.
    ///
    /// Never dropped: it lives as long as the process. Dropping a wgpu
    /// `Instance` drops wgpu-hal's `libloading` handles for `libEGL` /
    /// `libvulkan`, which `dlclose`s the driver they loaded. Mesa's software
    /// drivers (llvmpipe, lavapipe — every GPU-less CI runner, container and
    /// VM) and the LLVM inside them register `atexit` destructors, and once
    /// the mapping is gone the process dies inside `exit()` after the last
    /// context has been torn down cleanly. A driver has to outlive the process
    /// by construction, so this is not a leak; the device is still drained and
    /// destroyed in [`Drop`] exactly as before.
    pub instance: ManuallyDrop<wgpu::Instance>,
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
    /// The reason the device was lost, recorded by the device-lost callback.
    ///
    /// `get_current_texture` collapses a dead device into a bare `Validation`
    /// status with no scoped error, so the acquire path reads this to name the
    /// real cause instead of panicking on a shape that cannot be reconfigured.
    device_lost: DeviceLoss,
    /// Whether a submitted frame's GPU work has finished on this device.
    ///
    /// The submission completion driver sets it when a registered submission's
    /// wait succeeds — the one success signal a device reports. [`GpuRuntime`]
    /// reads it when the device is later reported lost: a context that
    /// presented before dying suffered an ordinary, recoverable loss, while
    /// one lost before its first completed frame is the stillborn device a
    /// wedged driver hands back on every recreation.
    frame_presented: Arc<AtomicBool>,
}

/// A view onto whether one context's device has been lost.
///
/// wgpu reports a loss exactly once, through the callback the runtime installs
/// at creation, and every resource call after it fails: `create_texture` hands
/// back an invalid handle and the first use of that handle raises a validation
/// error that wgpu treats as fatal. Work that runs off the frame path — a
/// raster worker streaming tiles from its own thread — never sees the frame
/// owner's rebuild, so it takes a clone of this handle at setup and asks it
/// before each batch of wgpu calls; once the answer is `true` the only correct
/// move is to stop, because the next [`GpuView::setup`] on the rebuilt context
/// replaces everything the worker was producing.
///
/// [`GpuView::setup`]: super::gpu_surface::GpuView::setup
#[derive(Clone, Debug, Default)]
pub struct DeviceLoss {
    reason: Arc<Mutex<Option<String>>>,
}

impl DeviceLoss {
    /// Starts observing `device`: installs its device-lost callback so the
    /// returned handle reports the loss the moment the driver announces it.
    ///
    /// wgpu keeps one lost callback per device, so this belongs to whoever
    /// owns the device — [`SharedGpuContext`] for the runtime's device, or a
    /// host that opened its own (a GTK backend adopting a `GLArea`'s
    /// adapter) — and is called once, right after the device is created.
    #[must_use]
    pub fn observe(device: &wgpu::Device) -> Self {
        let handle = Self::default();
        let recorder = handle.clone();
        device.set_device_lost_callback(move |reason, message| {
            tracing::error!(?reason, message, "WaterUI GPU device was lost");
            recorder.record(format!("{reason:?}: {message}"));
        });
        handle
    }

    /// Whether the driver has reported this device lost.
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.reason().is_some()
    }

    /// The reason the driver gave for the loss, once it reported one.
    #[must_use]
    pub fn reason(&self) -> Option<String> {
        self.reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn record(&self, reason: String) {
        *self
            .reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
    }
}

/// Which engine rasterizes scenes on a given device.
///
/// Decided from what the adapter reports, once, when the device is created —
/// not from a failure at draw time. A device that cannot run the classic
/// pipeline should never be asked to try it: doing so aborts the process rather
/// than degrading, because the error surfaces inside wgpu's default handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneEngine {
    /// The compute pipeline. Needs indirect execution and f16 unpacking in
    /// f32 shaders; the faster of the two.
    Classic,
    /// Paths on the CPU, rasterization on the GPU. Needs neither of the
    /// classic pipeline's capabilities, which is what makes it the one that
    /// runs on the iOS Simulator and the Android emulator.
    Hybrid,
}

impl SceneEngine {
    /// Everything the classic compute pipeline's shaders demand of a device.
    ///
    /// `SHADER_F16_IN_F32` is here because `vello.flatten` calls
    /// `unpack2x16float`, which naga validates against that capability at
    /// `create_shader_module` — an adapter that omits it (the Android
    /// emulator's SwiftShader-backed Vulkan, for one) aborts there, long
    /// after this choice was made.
    const CLASSIC_REQUIREMENTS: wgpu::DownlevelFlags =
        wgpu::DownlevelFlags::INDIRECT_EXECUTION.union(wgpu::DownlevelFlags::SHADER_F16_IN_F32);

    /// The engine this adapter can actually run.
    #[must_use]
    pub fn for_adapter(adapter: &wgpu::Adapter) -> Self {
        let flags = adapter.get_downlevel_capabilities().flags;
        if flags.contains(Self::CLASSIC_REQUIREMENTS) {
            Self::Classic
        } else {
            tracing::info!(
                adapter = %adapter.get_info().name,
                missing = ?Self::CLASSIC_REQUIREMENTS.difference(flags),
                "adapter cannot run the classic pipeline; scenes render through the hybrid engine"
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
                        images: HybridImageAtlas::default(),
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
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            clippy::arc_with_non_send_sync,
            reason = "wgpu's WebGPU backend is a thin wrapper over JS objects held in `Rc<RefCell<_>>`, so its adapter/device/queue handles and its request futures are neither `Send` nor `Sync` on this target alone. The context is shared by reference count on every target, so the storage type stays `Arc` rather than splitting into `Rc` here and `Arc` everywhere else."
        )
    )]
    async fn new(generation: u64) -> Result<Self, SharedContextError> {
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
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("WaterUI GPU runtime device"),
            required_features,
            required_limits,
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            trace: wgpu::Trace::default(),
        };
        // Android opens its device through the HAL so the external-memory
        // extensions view capture imports `AHardwareBuffer`s with are enabled;
        // that path talks to `vkCreateDevice` directly and has nothing to await.
        #[cfg(target_os = "android")]
        let (device, queue) = open_android_device(&adapter, &descriptor)?;
        #[cfg(not(target_os = "android"))]
        let (device, queue) = adapter
            .request_device(&descriptor)
            .await
            .map_err(|error| SharedContextError::DeviceCreationFailed(error.to_string()))?;

        // Device loss otherwise surfaces only as a bare `Validation` status on the
        // next swapchain acquire, with the reason discarded; record it so the
        // failure names its cause.
        let device_lost = DeviceLoss::observe(&device);

        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let frame_presented = Arc::new(AtomicBool::new(false));
        let submission_completion_driver = GpuSubmissionCompletionDriver::new(
            Arc::clone(&device),
            Arc::clone(&queue),
            Arc::clone(&frame_presented),
        );

        Ok(Self {
            generation,
            instance: ManuallyDrop::new(instance),
            adapter,
            device,
            queue,
            shader_cache: Arc::new(WgslModuleCache::new()),
            scene_renderer: Arc::new(SharedSceneRenderer::new(scene_engine)),
            submission_completion_driver,
            device_lost,
            frame_presented,
        })
    }

    /// Which recreation this context is — `0` initially, increasing each time
    /// the owning [`GpuRuntime`] rebuilds after device loss. A surface,
    /// pipeline or texture created under another generation belongs to a dead
    /// device and must be rebuilt against this context.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
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

    /// The device-lost reason recorded by the callback, once the driver reports
    /// one. A `Some` here means every device-bound resource on this context is
    /// dead: swapchain acquire, configure and pipeline use all fail until a new
    /// runtime is created.
    #[must_use]
    pub fn device_lost_reason(&self) -> Option<String> {
        self.device_lost.reason()
    }

    /// A handle that answers whether this context's device has been lost,
    /// for work that runs off the frame path and cannot wait for the owning
    /// runtime's rebuild to reach it.
    #[must_use]
    pub fn device_loss(&self) -> DeviceLoss {
        self.device_lost.clone()
    }

    /// Records a device loss the driver never reported, for tests that
    /// exercise the runtime's recreation path.
    #[doc(hidden)]
    pub fn mark_device_lost_for_testing(&self, reason: &str) {
        self.device_lost.record(reason.to_owned());
    }

    /// Whether a submitted frame's GPU work has finished on this context's
    /// device.
    ///
    /// A completed submission is the only success signal a device reports:
    /// [`GpuRuntime`] treats a loss as recoverable only when the lost context
    /// reached this point, and treats a run of losses with no presented frame
    /// between them as a driver that cannot sustain a device at all.
    #[must_use]
    pub fn frame_presented(&self) -> bool {
        self.frame_presented.load(Ordering::Relaxed)
    }

    /// Records a completed presented frame the driver never resolved, for
    /// tests that exercise the runtime's recreation budget.
    #[doc(hidden)]
    pub fn mark_frame_presented_for_testing(&self) {
        self.frame_presented.store(true, Ordering::Relaxed);
    }

    /// Marks this device generation as having presented a frame.
    ///
    /// `submission` must be ordered after the presented frame's real work on
    /// this context's queue — the marker a frame owner submits with
    /// `queue.submit([])` right after `present`, or the fence submission an
    /// external-capture path already made — so its retirement proves a
    /// presented frame's GPU work finished. Registering it with the
    /// completion driver is what makes [`Self::frame_presented`] observe the
    /// present: the driver's successful wait is the one success signal a
    /// device reports, and a generation [`GpuRuntime`] later finds lost is
    /// recoverable only when it reached this point.
    ///
    /// The same wait also releases the deferred-destruction bookkeeping the
    /// frame left behind, so a frame owner that registers every presented
    /// frame here no longer calls [`reclaim_device`].
    pub fn note_presented_submission(&self, submission: wgpu::SubmissionIndex) {
        self.submission_completion_driver
            .on_complete(submission, || {});
    }
}

#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "`wgpu::Instance::request_adapter` resolves through `navigator.gpu.requestAdapter()`, a JS promise the WebGPU backend keeps in an `Rc<RefCell<_>>`; the same future is `Send` on every other target"
    )
)]
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

/// The Vulkan device extensions Android view capture is built on.
///
/// [`VK_ANDROID_external_memory_android_hardware_buffer`][ahb] is what turns the
/// `AHardwareBuffer` a captured view subtree was rendered into by `HardwareRenderer`
/// into a `VkImage` this device can read; [`VK_EXT_queue_family_foreign`][foreign] is
/// what lets that image's ownership be acquired from — and released back to — the
/// Android framework, which owns the buffer between frames.
///
/// [ahb]: https://registry.khronos.org/vulkan/specs/latest/man/html/VK_ANDROID_external_memory_android_hardware_buffer.html
/// [foreign]: https://registry.khronos.org/vulkan/specs/latest/man/html/VK_EXT_queue_family_foreign.html
#[cfg(target_os = "android")]
const ANDROID_CAPTURE_DEVICE_EXTENSIONS: [&core::ffi::CStr; 2] = [
    ash::android::external_memory_android_hardware_buffer::NAME,
    ash::ext::queue_family_foreign::NAME,
];

/// Opens the Android GPU device with the view-capture extensions enabled.
///
/// `wgpu::Adapter::request_device` enables only the extensions wgpu itself needs,
/// and there is no descriptor field for asking for more, so the device is built
/// through the HAL adapter: `open_with_callback` hands the extension list to a
/// callback before `vkCreateDevice` sees it, and the resulting `OpenDevice` is
/// then adopted by wgpu with `create_device_from_hal`, which is what keeps the
/// device a perfectly ordinary `wgpu::Device` for everything else.
///
/// Both extensions are mandatory on every Android device that reports Vulkan 1.1
/// (Android CDD), so an adapter without them is a hard error naming the missing
/// extension rather than a capture path that silently is not there.
#[cfg(target_os = "android")]
fn open_android_device(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Device, wgpu::Queue), SharedContextError> {
    use wgpu_hal::api::Vulkan;

    // SAFETY: the HAL adapter is only borrowed to open a device from it. Nothing
    // here destroys it, and the guard is dropped before this function returns.
    let hal_adapter = unsafe { adapter.as_hal::<Vulkan>() }.ok_or_else(|| {
        SharedContextError::DeviceCreationFailed(
            "WaterUI renders through Vulkan on Android, and this adapter is not a Vulkan adapter"
                .to_owned(),
        )
    })?;

    let capabilities = hal_adapter.physical_device_capabilities();
    for extension in ANDROID_CAPTURE_DEVICE_EXTENSIONS {
        if !capabilities.supports_extension(extension) {
            return Err(SharedContextError::DeviceCreationFailed(format!(
                "the Vulkan driver does not support {}, which WaterUI needs to read a captured \
                 view subtree out of an AHardwareBuffer",
                extension.to_string_lossy()
            )));
        }
    }

    // SAFETY: the callback only appends extensions this adapter was just proven to
    // support, and removes nothing, which is `open_with_callback`'s contract. The
    // device it returns is handed straight to `create_device_from_hal` below, so
    // wgpu takes ownership of it exactly once.
    let open_device = unsafe {
        hal_adapter.open_with_callback(
            descriptor.required_features,
            &descriptor.required_limits,
            &descriptor.memory_hints,
            Some(Box::new(
                |args: wgpu_hal::vulkan::CreateDeviceCallbackArgs<'_, '_, '_>| {
                    for extension in ANDROID_CAPTURE_DEVICE_EXTENSIONS {
                        // wgpu may already have asked for one of these for its own
                        // reasons, and a repeated name is a `vkCreateDevice`
                        // validation error rather than a no-op.
                        if !args.extensions.contains(&extension) {
                            args.extensions.push(extension);
                        }
                    }
                },
            )),
        )
    }
    .map_err(|error| SharedContextError::DeviceCreationFailed(error.to_string()))?;

    // SAFETY: `open_device` was opened from this very adapter, with the features and
    // limits `descriptor` names, and has not been used for anything else.
    unsafe { adapter.create_device_from_hal::<Vulkan>(open_device, descriptor) }
        .map_err(|error| SharedContextError::DeviceCreationFailed(error.to_string()))
}

/// Whether the guest is an Android emulator (ranchu/goldfish/Cuttlefish).
///
/// Debug builds turn on `wgpu::InstanceFlags::DEBUG`, which makes naga tag every
/// SPIR-V module with `OpSource SourceLanguage::WGSL`. gfxstream's guest SPIR-V
/// validator predates that enumerator, so the first `vkCreateShaderModule`
/// carrying it kills the emulator's whole graphics stack. Clearing the flag on
/// emulators keeps the debug build usable; `WGPU_DEBUG` set explicitly still
/// wins for anyone who needs to debug shaders on an emulator.
#[cfg(target_os = "android")]
fn guest_is_emulator() -> bool {
    // SAFETY: `__system_property_get` only writes into `value` for the duration
    // of the call, the property names are valid C strings, and `value` stays
    // NUL-terminated afterwards.
    unsafe {
        let mut value = [0 as libc::c_char; libc::PROP_VALUE_MAX as usize];
        for name in [c"ro.kernel.qemu", c"ro.boot.qemu"] {
            if libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) > 0 {
                return true;
            }
        }
        libc::__system_property_get(c"ro.hardware".as_ptr(), value.as_mut_ptr());
        let hardware = core::ffi::CStr::from_ptr(value.as_ptr()).to_string_lossy();
        ["ranchu", "goldfish", "cutf"]
            .iter()
            .any(|prefix| hardware.starts_with(prefix))
    }
}

#[cfg(target_os = "android")]
async fn request_instance_and_adapter()
-> Result<(wgpu::Instance, wgpu::Adapter), SharedContextError> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    descriptor.backends = wgpu::Backends::VULKAN.with_env();
    if std::env::var_os("WGPU_DEBUG").is_none() && guest_is_emulator() {
        descriptor.flags -= wgpu::InstanceFlags::DEBUG
            | wgpu::InstanceFlags::VALIDATION
            | wgpu::InstanceFlags::GPU_BASED_VALIDATION;
    }
    let instance = wgpu::Instance::new(descriptor);
    let adapter = request_adapter(&instance)
        .await
        .map_err(|_| SharedContextError::NoAdapter)?;
    Ok((instance, adapter))
}

#[cfg(not(target_os = "android"))]
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "awaits `request_adapter` above, whose WebGPU implementation is a JS promise held in an `Rc<RefCell<_>>`, and holds the resulting `wgpu::Instance` across it; both are `Send` on every other target"
    )
)]
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
/// `TEXTURE_FORMAT_16BIT_NORM` is requested on **every** adapter that offers
/// it, not just Apple's. `P010` — the 10-bit layout every HDR video decodes
/// into — allocates its planes as `R16Unorm`/`Rg16Unorm`
/// (`runtime_player::create_visual_yuv_textures`), and wgpu rejects those
/// formats unless the feature was enabled when the device was created. Asking
/// for it only under `cfg!(target_vendor = "apple")` therefore paired a
/// platform-conditional request with a platform-unconditional use: on Linux
/// and Windows the first 10-bit frame died in `Device::create_texture` with
/// "Texture format `R16Unorm` can't be used due to missing features".
///
/// Adapters that genuinely lack the feature — llvmpipe/lavapipe on headless CI,
/// for instance — still cannot present 10-bit planes, and that remains a
/// hard error at the point of use rather than a silent drop to 8-bit.
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
    }

    if adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) {
        required |= wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
    }

    required
}

/// The most consecutive unproductive losses [`GpuRuntime`] recreates its
/// context for.
///
/// A context lost after presenting frames is the recoverable loss the rebuild
/// exists for; a context lost before its first presented frame produced
/// nothing, and several of those in a row mean the driver loses every device
/// it hands out. Past this count [`GpuRuntime::context`] reports the recorded
/// losses instead of paying for another stillborn device.
#[cfg(not(target_arch = "wasm32"))]
const MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS: usize = 3;

/// Cloneable owner for one explicitly-created shared GPU context.
///
/// The context is recreated in place when the driver reports its device lost:
/// [`GpuRuntime::context`] notices the recorded loss and swaps in a freshly
/// built [`SharedGpuContext`] — new instance, adapter, device, shader cache
/// and scene renderer — so every subsequent caller is back on live hardware.
/// The swap is what makes recovery possible at all: a dead device cannot
/// honour a surface, so nothing built on it is salvageable.
///
/// Recovery is bounded: a recreation only counts as recovery when the device
/// it replaced presented at least one frame. Consecutive losses of devices
/// that never presented — the signature of a driver that loses every device it
/// hands out — are capped at [`MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS`], after
/// which [`GpuRuntime::context`] panics with the collected loss reasons
/// instead of paying for another stillborn device.
#[derive(Clone)]
pub struct GpuRuntime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    /// The live context, replaced on the first access after its device is
    /// reported lost. The mutex also serializes the rebuild itself, so a loss
    /// observed by several views at once is paid for exactly once.
    context: Mutex<Arc<SharedGpuContext>>,
    /// The recorded reasons of consecutive losses that produced no presented
    /// frame — the current streak of stillborn devices. A context lost after
    /// presenting real work is an ordinary recoverable loss and clears the
    /// streak. WebGPU never rebuilds, so the streak exists on native targets
    /// only.
    #[cfg(not(target_arch = "wasm32"))]
    unproductive_losses: Mutex<Vec<String>>,
    /// Generation handed to the next rebuilt context. WebGPU has no
    /// synchronous rebuild path, so on wasm32 no context is ever rebuilt.
    #[cfg(not(target_arch = "wasm32"))]
    next_generation: AtomicU64,
}

impl fmt::Debug for GpuRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuRuntime")
            .field("context", &self.inner.context)
            .finish()
    }
}

impl GpuRuntime {
    /// Creates an independent GPU runtime.
    ///
    /// # Errors
    ///
    /// Returns the adapter or device initialization error.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            clippy::arc_with_non_send_sync,
            reason = "awaits `SharedGpuContext::new`, whose WebGPU adapter and device requests are JS promises, and stores the resulting JS-backed handles; the runtime is shared by reference count on every target, so the storage type stays `Arc` rather than splitting into `Rc` here and `Arc` everywhere else"
        )
    )]
    pub async fn new() -> Result<Self, SharedContextError> {
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                context: Mutex::new(Arc::new(SharedGpuContext::new(0).await?)),
                #[cfg(not(target_arch = "wasm32"))]
                unproductive_losses: Mutex::new(Vec::new()),
                #[cfg(not(target_arch = "wasm32"))]
                next_generation: AtomicU64::new(1),
            }),
        })
    }

    /// Returns this runtime's shared GPU resources.
    ///
    /// When the driver has reported the current context's device lost, this
    /// rebuilds the context first and returns the replacement. Callers holding
    /// device-bound resources from an earlier call compare
    /// [`SharedGpuContext::generation`] to know they must rebuild them.
    ///
    /// # Panics
    ///
    /// Panics once [`MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS`] devices in a row
    /// were each lost before presenting a frame — a driver that loses every
    /// device it hands out is not recoverable — and reports the collected
    /// loss reasons.
    #[must_use]
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the context lock must stay held across `rebuild_locked` so concurrent callers wait for the one rebuild instead of racing to create their own device"
    )]
    pub fn context(&self) -> Arc<SharedGpuContext> {
        let mut slot = self
            .inner
            .context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.device_lost_reason().is_none() {
            return Arc::clone(&slot);
        }
        self.rebuild_locked(&mut slot)
    }

    /// Replaces the lost context in `slot` with a freshly built one.
    ///
    /// A failed rebuild keeps the dead context in place and returns it, so the
    /// caller still sees the recorded loss reason instead of a second failure;
    /// the next `context()` call retries the rebuild, bounded by the same cap
    /// as the losses themselves.
    ///
    /// # Panics
    ///
    /// Panics when [`MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS`] devices in a row
    /// each died before presenting a frame: that streak means the driver loses
    /// every device it creates, so another recreation would only spin behind
    /// a blank screen. The panic carries every recorded loss reason.
    #[cfg(not(target_arch = "wasm32"))]
    fn rebuild_locked(&self, slot: &mut Arc<SharedGpuContext>) -> Arc<SharedGpuContext> {
        {
            let mut unproductive = self
                .inner
                .unproductive_losses
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.frame_presented() {
                unproductive.clear();
            } else {
                unproductive.push(
                    slot.device_lost_reason()
                        .unwrap_or_else(|| "device lost with no recorded reason".to_owned()),
                );
            }
            Self::enforce_rebuild_budget(&unproductive);
            drop(unproductive);
        }

        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        match pollster::block_on(SharedGpuContext::new(generation)) {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                tracing::warn!(
                    generation,
                    adapter = %fresh.adapter.get_info().name,
                    "GPU device was lost; recreated the runtime context"
                );
                *slot = Arc::clone(&fresh);
                fresh
            }
            Err(error) => {
                {
                    let mut unproductive = self
                        .inner
                        .unproductive_losses
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    unproductive.push(format!("context recreation failed: {error}"));
                    Self::enforce_rebuild_budget(&unproductive);
                    drop(unproductive);
                }
                tracing::error!(
                    "GPU device was lost and recreation failed ({error}); \
                     retrying on the next access"
                );
                Arc::clone(slot)
            }
        }
    }

    /// Stops an unbounded recreation streak: a run of devices that each died
    /// before presenting a frame means the driver cannot sustain one, so the
    /// runtime reports the collected reasons instead of rebuilding again.
    #[cfg(not(target_arch = "wasm32"))]
    fn enforce_rebuild_budget(unproductive: &[String]) {
        assert!(
            unproductive.len() <= MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS,
            "WaterUI GPU device was lost {} times in a row without ever \
             presenting a frame; the device is unrecoverable. Recorded losses: {}",
            unproductive.len(),
            unproductive.join(" | ")
        );
    }

    /// WebGPU reports device loss but offers no synchronous rebuild path —
    /// `request_adapter` is a JS promise this accessor cannot await — so the
    /// lost context stays in place and the failure keeps naming its cause.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::unused_self,
        clippy::needless_pass_by_ref_mut,
        reason = "keeps the native twin's signature so `context()` has one call site; the native rebuild reads the runtime's generation counter and replaces the slot"
    )]
    fn rebuild_locked(&self, slot: &mut Arc<SharedGpuContext>) -> Arc<SharedGpuContext> {
        Arc::clone(slot)
    }
}

/// Waits for a GPU device to finish everything it was given, before it goes away.
///
/// `wgpu` waits for the last submission when a device is destroyed, through a
/// fixed ladder of timeouts totalling 6.3 seconds (`wgpu-core`'s
/// `device/queue.rs`), and *panics* when it runs out — destroying resources the
/// GPU is still reading would be undefined behaviour, so it refuses. That
/// budget assumes hardware. A software adapter rasterizing a heavy scene
/// routinely needs longer, which is what takes GPU-backed tests down on a
/// runner with no GPU.
///
/// Calling this first removes the deadline rather than widening it: by the time
/// `wgpu`'s own wait runs there is nothing left to wait for. The wait is
/// indefinite on purpose — the work completes unless the driver itself is
/// wedged, and that is a condition for the caller's own timeout to catch rather
/// than something to hide behind a second arbitrary deadline. A device with
/// nothing outstanding returns immediately, so this costs nothing when the GPU
/// has kept up.
///
/// Every type that owns a device to the end of its life calls this from `Drop`.
pub fn drain_device_before_teardown(device: &wgpu::Device) {
    let drained = poll_device(
        device,
        wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        },
    );
    if let Err(error) = drained {
        tracing::warn!("GPU device did not drain before teardown: {error}");
    }
}

/// Releases the resources whose destruction the device deferred.
///
/// `wgpu` retires finished submissions from inside `Queue::submit`, but the
/// bookkeeping of the objects those submissions dropped — the bind groups and
/// texture views a scene renderer creates per frame — is released only from
/// `Device::poll`. A frame loop that only ever submits and presents therefore
/// keeps every frame's share of it forever: on a Pixel 9 Pro, 56 animated
/// `GpuSurface`s grew the native heap by 87 MB per 150 s, and were flat with
/// this call after each presented frame.
///
/// Non-blocking: `PollType::Poll` processes what has already completed and
/// returns. Every frame owner that presents without registering the submission
/// with the completion driver (which polls for it) calls this once per frame,
/// after the present.
pub fn reclaim_device(device: &wgpu::Device) {
    if let Err(error) = poll_device(device, wgpu::PollType::Poll) {
        tracing::warn!("GPU device did not reclaim deferred resources: {error}");
    }
}

/// Polls the device, treating a panic from inside `wgpu`'s bookkeeping as one
/// more failed poll.
///
/// Device loss is detected lazily: the driver notices mid-call, purges the
/// resource storage, and only then reports through the device-lost callback.
/// A poll that lands in that window — or on an already-purged device — can
/// dereference a resource the loss already removed, and `wgpu-core`'s storage
/// lookup panics rather than erroring. The device is dead either way, so the
/// poll reports failure and lets the owner move on to rebuilding instead of
/// taking the process down over bookkeeping for a device that no longer
/// exists.
fn poll_device(
    device: &wgpu::Device,
    poll_type: wgpu::PollType,
) -> Result<wgpu::PollStatus, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(poll_type))) {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(error)) => Err(error.to_string()),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            Err(format!(
                "poll panicked inside wgpu after device loss: {message}"
            ))
        }
    }
}

impl Drop for SharedGpuContext {
    fn drop(&mut self) {
        // Draining first leaves the completion thread nothing left to wait for,
        // so the join that follows — when `submission_completion_driver`, the
        // last field, drops — returns immediately. The device is destroyed
        // inside that join rather than after this context is gone.
        drain_device_before_teardown(&self.device);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    use super::MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS;
    use super::{GpuRuntime, required_device_limits, required_media_features};
    use std::sync::Arc;

    /// A runtime that has been dropped must leave no GPU handle behind.
    ///
    /// The submission completion thread is the device's only other owner, so
    /// after the runtime is gone the device is destroyed exactly when that
    /// thread has been joined. Before it was joined, the device outlived its
    /// runtime by however long the detached thread took to notice — and the
    /// destruction it then ran raced whatever the program did next, which in a
    /// test binary is `exit()` unloading the very driver being destroyed.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dropping_a_runtime_destroys_its_device_before_returning() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("GPU runtime teardown requires a working GPU runtime");
        let device = Arc::downgrade(&runtime.context().device);

        drop(runtime);

        assert!(
            device.upgrade().is_none(),
            "the GPU device outlived its runtime: the submission completion thread was still \
             holding it when the runtime finished dropping"
        );
    }

    /// A context whose device was reported lost is replaced on the next
    /// `context()` access, and handles issued earlier still describe the dead
    /// device so their owners can finish tearing down against it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_lost_device_is_replaced_on_the_next_context_access() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("device-loss recreation requires a working GPU runtime");
        let stale = runtime.context();
        let stale_generation = stale.generation();
        stale.mark_device_lost_for_testing("simulated device loss");

        let fresh = runtime.context();

        assert!(fresh.device_lost_reason().is_none());
        assert_ne!(fresh.generation(), stale_generation);
        assert!(stale.device_lost_reason().is_some());
    }

    /// A worker takes its [`DeviceLoss`] handle at setup, long before any
    /// loss; the handle must report the loss the context records later, and
    /// a handle from the rebuilt context must not.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_device_loss_handle_taken_at_setup_reports_a_later_loss() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("device-loss observation requires a working GPU runtime");
        let stale = runtime.context();
        let handle = stale.device_loss();
        assert!(!handle.is_lost());

        stale.mark_device_lost_for_testing("simulated device loss");

        assert!(handle.is_lost());
        assert_eq!(handle.reason().as_deref(), Some("simulated device loss"));
        assert!(!runtime.context().device_loss().is_lost());
    }

    /// A run of devices that each die before presenting a frame is not a
    /// recoverable loss pattern: the runtime must stop rebuilding and surface
    /// the collected reasons instead of spinning behind a blank screen.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn unproductive_device_losses_stop_the_rebuild_at_the_cap() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("rebuild-budget test requires a working GPU runtime");
        for _ in 0..MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS {
            runtime
                .context()
                .mark_device_lost_for_testing("simulated device loss");
            let _ = runtime.context();
        }

        runtime
            .context()
            .mark_device_lost_for_testing("simulated device loss");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.context()));

        assert!(
            result.is_err(),
            "the next consecutive device loss without a presented frame must              surface instead of rebuilding again"
        );
    }

    /// A device lost after presenting frames is the recoverable loss the
    /// rebuild exists for: it restarts the unproductive streak, so losses
    /// after real work never hit the cap early.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_loss_after_presented_frames_restarts_the_unproductive_count() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("rebuild-budget test requires a working GPU runtime");

        for _ in 0..MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS - 1 {
            runtime
                .context()
                .mark_device_lost_for_testing("simulated device loss");
            let _ = runtime.context();
        }
        let productive = runtime.context();
        productive.mark_frame_presented_for_testing();
        productive.mark_device_lost_for_testing("simulated device loss");

        // The productive context restarted the streak, so a full budget of
        // stillborn devices rebuilds before the cap surfaces again.
        for _ in 0..MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS {
            runtime
                .context()
                .mark_device_lost_for_testing("simulated device loss");
            let _ = runtime.context();
        }

        runtime
            .context()
            .mark_device_lost_for_testing("simulated device loss");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.context()));

        assert!(
            result.is_err(),
            "the streak must restart at the presented frame, not accumulate across it"
        );
    }

    /// The marker a swapchain path registers after `present` is what makes a
    /// generation count as productive: once the completion driver resolves
    /// it, a later loss restarts the unproductive streak instead of counting
    /// toward the cap.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_presented_frame_marker_marks_the_generation_productive() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("rebuild-budget test requires a working GPU runtime");
        let context = runtime.context();

        context.note_presented_submission(context.queue.submit([]));

        // The driver resolves waits serially on its own thread, so the
        // completion of a later submission proves the marker's wait — and its
        // `frame_presented` store — already ran.
        let (resolved, wait) = std::sync::mpsc::channel();
        context
            .submission_completion_driver()
            .on_complete(context.queue.submit([]), move || {
                resolved.send(()).expect("the test is still waiting");
            });
        wait.recv().expect("the completion driver is running");

        assert!(context.frame_presented());

        context.mark_device_lost_for_testing("simulated device loss");
        let _ = runtime.context();

        // The productive generation cleared the streak, so a full budget of
        // stillborn devices rebuilds before the cap surfaces again.
        for _ in 0..MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS {
            runtime
                .context()
                .mark_device_lost_for_testing("simulated device loss");
            let _ = runtime.context();
        }
        runtime
            .context()
            .mark_device_lost_for_testing("simulated device loss");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.context()));

        assert!(
            result.is_err(),
            "the registered presented frame must have restarted the unproductive streak"
        );
    }

    fn adapter_features_with_16bit_norm() -> wgpu::Features {
        let mut features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        if cfg!(target_vendor = "apple") {
            features |= wgpu::Features::PASSTHROUGH_SHADERS;
        }
        features
    }

    #[test]
    fn media_features_enable_16bit_norm_on_every_offering_adapter() {
        let required = required_media_features(adapter_features_with_16bit_norm());
        assert!(required.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM));
    }

    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn media_features_omit_16bit_norm_when_adapter_lacks_it() {
        let required = required_media_features(wgpu::Features::empty());
        assert!(!required.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    #[should_panic(expected = "normalized 16-bit textures")]
    fn media_features_panic_on_apple_without_16bit_norm() {
        let _ = required_media_features(wgpu::Features::PASSTHROUGH_SHADERS);
    }

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

/// The dedicated thread that resolves submission completions, and the join that
/// keeps it from outliving the runtime that owns it.
///
/// The thread holds its own handle on the device it polls, so it outlives the
/// context that spawned it and is that device's last owner: closing its channel
/// makes it exit and *then* destroy the device. Leaving that unjoined hands a
/// `wgpu` device's destruction — `vkDestroyDevice` and everything the driver
/// unwinds under it — to a detached thread racing the rest of the program, and
/// in a test binary the rest of the program is `exit()` unloading that same
/// driver. That race is what made a GPU test report `test result: ok` and then
/// take the process down with SIGSEGV on a software adapter.
#[cfg(not(target_arch = "wasm32"))]
struct CompletionThread {
    /// Taken in [`Drop`] to close the channel, which is what tells the thread to
    /// stop. `Some` for as long as the thread is running.
    jobs: Option<mpsc::Sender<SubmissionCompletion>>,
    /// Taken in [`Drop`] to join. `Some` for as long as the thread is running.
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl CompletionThread {
    /// Queues one submission wait, panicking when the thread is gone.
    fn submit(&self, job: SubmissionCompletion) {
        self.jobs
            .as_ref()
            .expect("the GPU submission completion driver is shutting down")
            .send(job)
            .expect("GPU submission completion driver stopped unexpectedly");
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for CompletionThread {
    fn drop(&mut self) {
        // Order matters and is explicit rather than left to field declaration
        // order: the thread only leaves `recv` once every sender is gone, so the
        // channel closes first and the join follows.
        drop(self.jobs.take());
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .expect("the GPU submission completion driver panicked");
        }
    }
}

/// Serial completion driver for submissions on one GPU device.
///
/// Native runtimes confine submission waits to one dedicated thread. WebGPU
/// runtimes use the browser-driven queue completion callback.
///
/// Every clone shares the one thread, so it is torn down — and joined — when the
/// last clone goes away, which for a runtime's own driver is the moment its
/// [`SharedGpuContext`] finishes dropping.
#[derive(Clone)]
#[doc(hidden)]
pub struct GpuSubmissionCompletionDriver {
    #[cfg(not(target_arch = "wasm32"))]
    thread: Arc<CompletionThread>,
    #[cfg(target_arch = "wasm32")]
    queue: Arc<wgpu::Queue>,
    #[cfg(target_arch = "wasm32")]
    frame_presented: Arc<AtomicBool>,
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
    fn new(
        device: Arc<wgpu::Device>,
        _queue: Arc<wgpu::Queue>,
        frame_presented: Arc<AtomicBool>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<SubmissionCompletion>();
        let handle = std::thread::Builder::new()
            .name("waterui-gpu-completion".to_owned())
            .spawn(move || {
                while let Ok((submission, completion)) = receiver.recv() {
                    // A lost device fails every wait; the submission is dead
                    // either way, so run the completion — waiters use it to
                    // release resources, not to learn about success.
                    if let Err(error) = poll_device(
                        &device,
                        wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: None,
                        },
                    ) {
                        tracing::warn!(
                            "GPU submission wait failed ({error}); resolving the completion anyway"
                        );
                    } else {
                        // A resolved submission is the only success signal the
                        // device reports; record it so a later loss counts as
                        // recoverable rather than stillborn.
                        frame_presented.store(true, Ordering::Relaxed);
                    }
                    completion();
                }
            })
            .expect("failed to start the GPU submission completion driver");
        Self {
            thread: Arc::new(CompletionThread {
                jobs: Some(sender),
                handle: Some(handle),
            }),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn new(
        _device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        frame_presented: Arc<AtomicBool>,
    ) -> Self {
        Self {
            queue,
            frame_presented,
        }
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
        self.thread.submit((submission, Box::new(completion)));
    }

    /// Runs `completion` after all work submitted through this queue has finished.
    #[cfg(target_arch = "wasm32")]
    pub fn on_complete(
        &self,
        _submission: wgpu::SubmissionIndex,
        completion: impl FnOnce() + Send + 'static,
    ) {
        let frame_presented = Arc::clone(&self.frame_presented);
        self.queue.on_submitted_work_done(move || {
            frame_presented.store(true, Ordering::Relaxed);
            completion();
        });
    }
}

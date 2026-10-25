//! The Cherenkov GPU attachment for the Android host.
//!
//! The `SurfaceView` supplies an owned native-window lease. Cherenkov presents
//! directly through child `SurfaceControls`, retaining buffers and repairing
//! only their accumulated damage. No wgpu swapchain or full-window copy is
//! involved. Attachment generations, device recovery and refresh-rate votes
//! remain owned by the host.

use std::fmt;
use std::sync::Arc;

use ndk::native_window::NativeWindow;
use raw_window_handle::{AndroidDisplayHandle, DisplayHandle, HasDisplayHandle, RawDisplayHandle};

use crate::platform::{PresentationSurface, SurfaceError};

/// The Android display's raw handle: the platform has one implicit default
/// display, which `AndroidDisplayHandle` names.
#[derive(Debug)]
struct AndroidDisplay;

impl HasDisplayHandle for AndroidDisplay {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: Android's display handle is a static system resource —
        // borrowing it raw is always sound for the process lifetime.
        Ok(unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::Android(AndroidDisplayHandle::new()))
        })
    }
}

/// A GPU-attachment failure surfaced to the Kotlin host as an exception.
#[derive(Debug)]
pub struct GpuError(pub String);

impl GpuError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn new_instance(backends: wgpu::Backends) -> wgpu::Instance {
    let mut descriptor =
        wgpu::InstanceDescriptor::new_with_display_handle(Box::new(AndroidDisplay));
    descriptor.backends = backends;
    wgpu::Instance::new(descriptor)
}

async fn request_adapter(instance: &wgpu::Instance) -> Result<wgpu::Adapter, GpuError> {
    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|error| {
            GpuError::new(format!(
                "hydrolysis android: no wgpu-capable GPU adapter: {error}"
            ))
        })
}

struct AndroidGpuContextInner {
    /// The live device generation and its loss budget; a lost device is
    /// recreated on the same instance (water-rs/waterui#2742).
    gpu: crate::platform::RecoverableGpu,
}

impl Drop for AndroidGpuContextInner {
    fn drop(&mut self) {
        crate::platform::drain_device_before_teardown(
            &self.gpu.generation().gpu_context().shared_device.device,
        );
    }
}

/// The process-wide wgpu context the Android host renders through.
///
/// Vulkan only — the plan of record is Cherenkov over Vulkan, so there is no
/// runtime fallback to another GPU API: a Vulkan adapter that cannot paint
/// is an explicit [`GpuError`] naming the adapter and the missing flags,
/// never a switch to GLES or another painter.
#[derive(Clone)]
pub struct AndroidGpuContext {
    inner: Arc<AndroidGpuContextInner>,
}

impl AndroidGpuContext {
    /// Requests the GPU context, blocking the calling (UI) thread.
    pub(crate) fn request() -> Result<Self, GpuError> {
        pollster::block_on(Self::request_async())
    }

    /// The first painter-capable Vulkan adapter on `instance`.
    async fn pick_adapter(instance: &wgpu::Instance) -> Result<wgpu::Adapter, GpuError> {
        // The engine's shaders require f16 arithmetic — its raster pipeline
        // uses f16-in-f32 builtins unconditionally. The default pick may lack
        // it (software Vulkan drivers like llvmpipe drop shaderFloat16), so
        // walk every Vulkan adapter and choose the first that can paint; a
        // device with none is an explicit error, never a painter fallback.
        const PAINTER_FLAGS: wgpu::DownlevelFlags = wgpu::DownlevelFlags::SHADER_F16_IN_F32;
        let flags_of = |adapter: &wgpu::Adapter| adapter.get_downlevel_capabilities().flags;
        let capable = |adapter: &wgpu::Adapter| flags_of(adapter).contains(PAINTER_FLAGS);
        let preferred = request_adapter(instance).await?;
        if capable(&preferred) {
            return Ok(preferred);
        }
        instance
            .enumerate_adapters(wgpu::Backends::VULKAN)
            .await
            .into_iter()
            .find(capable)
            .map_or_else(
                || {
                    let info = preferred.get_info();
                    let missing = PAINTER_FLAGS.difference(flags_of(&preferred));
                    Err(GpuError::new(format!(
                        "hydrolysis android: no Vulkan adapter meets the \
                             painter's requirements — '{}' ({:?}) lacks \
                             {missing:?} and no alternate Vulkan adapter qualifies",
                        info.name, info.backend
                    )))
                },
                Ok,
            )
    }

    /// Opens a device generation on `instance`: adapter pick, engine
    /// device request and loss observation. A lost device's recreation
    /// re-runs it on the lost generation's own instance, so the surfaces
    /// the chain holds stay valid (water-rs/waterui#2742).
    async fn open_generation(
        instance: &wgpu::Instance,
    ) -> Result<crate::platform::DeviceLoss, GpuError> {
        let adapter = Self::pick_adapter(instance).await?;
        let required_limits = crate::platform::required_device_limits(&adapter);
        let required_features = crate::platform::required_media_features(adapter.features())
            | (adapter.features()
                & (wgpu::Features::PIPELINE_CACHE | wgpu::Features::PASSTHROUGH_SHADERS));
        // The device is opened through the engine's creation path, not
        // `request_device`: the capabilities Cherenkov composes with —
        // external-frame and YCbCr import (cherenkov#166), and the
        // dynamic-rendering local read and rasterization-order
        // attachment access that keep an epoch's intermediates in tile
        // memory (waterui#2191) — are chosen when `vkCreateDevice` runs
        // and cannot be turned on afterwards. A device this host opened
        // itself would make the engine materialize every intermediate
        // through memory for the life of the process.
        let (device, queue) = crate::platform::open_engine_device(
            &adapter,
            &wgpu::DeviceDescriptor {
                label: Some("hydrolysis-android-device"),
                required_features,
                required_limits,
                memory_hints: cherenkov_gpu::interop::memory_hints(adapter.get_info().backend),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::default(),
            },
        )
        .await
        .map_err(|error| {
            GpuError::new(format!(
                "hydrolysis android: failed to open the engine's wgpu device: {error}"
            ))
        })?;
        let context_id = crate::platform::next_gpu_context_id();
        Ok(crate::platform::DeviceLoss::observe(
            cherenkov_gpu::interop::SharedDevice {
                instance: instance.clone(),
                adapter,
                device,
                queue,
            },
            context_id,
        ))
    }

    async fn request_async() -> Result<Self, GpuError> {
        // Native plane imports and rendering share this Vulkan device.
        let instance = new_instance(wgpu::Backends::VULKAN);
        let generation = Self::open_generation(&instance).await?;
        Ok(Self {
            inner: Arc::new(AndroidGpuContextInner {
                gpu: crate::platform::RecoverableGpu::new(generation),
            }),
        })
    }
}

/// A generation-tracked native presentation attachment. The engine window
/// is released before the `SurfaceView`'s native-window lease.
pub struct AndroidSurface {
    gpu: AndroidGpuContext,
    /// The device generation this attachment renders on — a clone of the
    /// context's live generation, adopted in `ensure_live`
    /// (water-rs/waterui#2742).
    handles: crate::platform::DeviceLoss,
    /// The generation the Kotlin host assigned this attachment; a stale
    /// `surfaceDestroyed` from an older `SurfaceHolder` callback is ignored.
    generation: u64,
    engine_window: Option<crate::renderer::CherenkovWindow<crate::engine::AndroidCherenkovSurface>>,
    /// GPU pipelines survive `SurfaceHolder` teardown while this host stays alive.
    engine_state: Option<std::rc::Rc<crate::engine::SharedEngineState>>,
    native_window: Option<WindowLease>,
    /// The Kotlin `SurfaceHolder`'s reported geometry, independent of the
    /// engine surface — valid while between surface generations too.
    width: u32,
    height: u32,
    /// Demand belongs to the session and is reapplied to each attachment.
    refresh_demand: RefreshDemand,
}

/// An attachment and its display's peak and content-category rates.
struct WindowLease {
    window: NativeWindow,
    /// Positive and finite; the JNI boundary rejects anything else.
    refresh: DisplayRefresh,
}

/// Display capabilities supplied by the platform at attachment.
#[derive(Clone, Copy)]
pub struct DisplayRefresh {
    pub peak: f32,
    pub normal: f32,
    pub high: f32,
}

#[derive(Default, PartialEq, Eq)]
pub struct RefreshDemand {
    pub continuous: bool,
    pub touching: bool,
    pub content: Option<cherenkov::RefreshRange>,
}

impl DisplayRefresh {
    fn requested(self, demand: &RefreshDemand) -> f32 {
        if demand.touching {
            return self.peak;
        }
        // Continuous callbacks also drive classified animations. Their content
        // vote sets the vsync cadence; only unclassified work needs peak rate.
        let Some(content) = &demand.content else {
            return if demand.continuous { self.peak } else { 0.0 };
        };
        let minimum = crate::num_cast::f64_as_f32(f64::from(*content.start()));
        let maximum = crate::num_cast::f64_as_f32(f64::from(*content.end()));
        let suggested = if maximum <= self.normal {
            self.normal
        } else {
            self.high
        };
        suggested.clamp(minimum, maximum).min(self.peak)
    }
}

impl AndroidSurface {
    pub fn new(gpu: AndroidGpuContext) -> Self {
        let handles = gpu.inner.gpu.generation();
        Self {
            gpu,
            handles,
            generation: 0,
            engine_window: None,
            engine_state: None,
            native_window: None,
            width: 0,
            height: 0,
            refresh_demand: RefreshDemand::default(),
        }
    }

    /// Whether the band has a live native window and a drawable extent.
    pub const fn is_attached(&self) -> bool {
        self.native_window.is_some() && self.width != 0 && self.height != 0
    }

    /// Attaches a new native window at `generation`, releasing any prior
    /// attachment first.
    ///
    /// `native_window` is the owned lease obtained from the Kotlin `Surface`
    /// (`ANativeWindow_fromSurface` acquires the reference it returns), and
    /// `refresh` describes its display's supported peak and suggested rates.
    /// Errors leave the attachment detached.
    pub(crate) fn attach(
        &mut self,
        native_window: NativeWindow,
        refresh: DisplayRefresh,
        width: u32,
        height: u32,
        generation: u64,
    ) -> Result<(), GpuError> {
        self.detach();
        self.width = width;
        self.height = height;
        self.generation = generation;
        self.native_window = Some(WindowLease {
            window: native_window,
            refresh,
        });
        // The frame-rate request is a property of the window, so the demand
        // the scheduler held across the re-attach goes onto the new lease.
        if let Err(error) = self.apply_frame_rate() {
            self.detach();
            return Err(error);
        }
        Ok(())
    }

    /// Drops the engine surface before releasing its native parent window.
    /// Refresh demand survives attachment replacement.
    pub(crate) fn detach(&mut self) {
        self.engine_window = None;
        self.native_window = None;
    }

    /// The band's geometry changed; the next engine frame applies the size.
    /// `generation` must match the live attachment — a stale `surfaceChanged`
    /// is ignored.
    pub(crate) fn resize_for(&mut self, width: u32, height: u32, generation: u64) {
        if generation != self.generation || self.native_window.is_none() {
            return;
        }
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            self.engine_window = None;
        }
    }

    /// Adopts the live device chain. The next render replaces an engine
    /// window from the lost chain before submitting any content.
    fn ensure_live(&mut self) {
        let gpu = self.gpu.clone();
        let handles = gpu.inner.gpu.ensure_live(|instance| {
            pollster::block_on(AndroidGpuContext::open_generation(instance))
                .map_err(|error| error.0)
        });
        if handles.gpu_context().context_id != self.handles.gpu_context().context_id {
            self.engine_state = None;
        }
        self.handles = handles;
    }

    fn render_targets(
        &mut self,
        display_scale: f64,
        base_color: waterui_graphics::draw::WorkingColor,
    ) -> (
        crate::renderer::FrameRenderTarget<'_>,
        crate::platform::EngineAndroidTarget<'_>,
    ) {
        let context = self.handles.gpu_context();
        let state = self.engine_state.get_or_insert_with(|| {
            crate::engine::shared_engine_state(
                context.context_id,
                &context.shared_device.adapter,
                context.shared_device.clone(),
            )
        });
        (
            crate::renderer::FrameRenderTarget {
                adapter: &context.shared_device.adapter,
                device: &context.shared_device.device,
                queue: &context.shared_device.queue,
                device_loss: self.handles.clone(),
                gpu_context_id: context.context_id,
                shared_device: context.shared_device.clone(),
                display_scale,
                width: self.width,
                height: self.height,
                base_color,
            },
            crate::platform::EngineAndroidTarget {
                state,
                window: &self
                    .native_window
                    .as_ref()
                    .expect("attached Android window")
                    .window,
                window_slot: &mut self.engine_window,
            },
        )
    }

    /// If `generation` names the live attachment, releases it. A stale
    /// generation (a `surfaceDestroyed` belonging to an already-replaced
    /// surface) is ignored.
    pub(crate) fn detach_for(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        self.detach();
        true
    }

    /// Holds content and interaction demand across attachments. An unchanged
    /// demand does not repeat the platform call.
    pub(crate) fn set_refresh_demand(&mut self, demand: RefreshDemand) -> Result<(), GpuError> {
        if self.refresh_demand == demand {
            return Ok(());
        }
        self.refresh_demand = demand;
        self.apply_frame_rate()
    }

    /// Writes the held demand onto the live native window. Between
    /// attachments there is nothing to write to — [`Self::attach`] applies
    /// it there. An idle demand writes a rate of 0, which clears the request
    /// and returns the surface to the system's choice.
    fn apply_frame_rate(&self) -> Result<(), GpuError> {
        let Some(lease) = self.native_window.as_ref() else {
            return Ok(());
        };
        let rate = lease.refresh.requested(&self.refresh_demand);
        // `ANativeWindow_setFrameRate` is API 30. The framework floor is API
        // 31, so the symbol is a direct link, not a run-time lookup.
        lease
            .window
            .set_frame_rate(rate, ndk::native_window::FrameRateCompatibility::Default)
            .map_err(|error| {
                GpuError::new(format!(
                    "hydrolysis android: ANativeWindow_setFrameRate({rate}) failed: {error}"
                ))
            })
    }
}

impl Drop for AndroidSurface {
    fn drop(&mut self) {
        self.detach();
    }
}

impl PresentationSurface for AndroidSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        &self.handles.gpu_context().shared_device.adapter
    }

    fn device(&self) -> &wgpu::Device {
        &self.handles.gpu_context().shared_device.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.handles.gpu_context().shared_device.queue
    }

    fn device_loss(&self) -> &crate::platform::DeviceLoss {
        &self.handles
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            self.engine_window = None;
        }
    }

    fn ensure_live(&mut self) {
        Self::ensure_live(self);
    }
}

impl crate::runner::window::GpuSurfaceFrame for AndroidSurface {
    fn render_frame(
        &mut self,
        renderer: &mut crate::renderer::HydrolysisRenderer,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> Result<crate::runner::window::SurfaceRenderResult, SurfaceError> {
        let started = std::time::Instant::now();
        self.ensure_live();
        if self.handles.is_lost() || !self.is_attached() {
            return Err(SurfaceError::Lost);
        }
        let (target, engine_target) =
            self.render_targets(display_scale, crate::renderer::working_color(clear_color));
        renderer
            .render_android_frame(&target, engine_target)
            .unwrap_or_else(|error| panic!("hydrolysis android: engine render failed: {error:#}"));
        self.device_loss().note_frame_presented();
        Ok(crate::runner::window::SurfaceRenderResult::engine_presented(started.elapsed()))
    }
}

//! The Cherenkov GPU attachment for the Android host.
//!
//! One [`AndroidGpuContext`] per process owns the wgpu instance, adapter,
//! device and queue; every presentation attachment ([`AndroidSurface`]) is a
//! `wgpu::Surface` over an owned `ANativeWindow` lease. The Kotlin side owns
//! the SurfaceView/TextureView band; the native side owns the swapchain. The
//! attachment is generation-tracked: the host increments the generation on
//! every `surfaceCreated`, and a `surfaceDestroyed` carrying a stale
//! generation never tears down a newer surface.
//!
//! Configuration is explicit, per the plan of record: FIFO present mode with a
//! maximum frame latency of two, and a high-refresh request routed through
//! `ANativeWindow_setFrameRate` (API 30+) while interaction or animation keeps
//! the scheduler awake. Zero-sized surfaces are never configured — the
//! attachment waits parked until the band reports a real size.
//!
//! Every failure here is an explicit error, never a silent redraw black hole:
//! adapter request failures, unsupported surface capabilities and device loss
//! all surface to Kotlin as [`GpuError`] exceptions.

use std::fmt;
use std::sync::Arc;

use ndk::native_window::NativeWindow;
use raw_window_handle::{AndroidDisplayHandle, DisplayHandle, HasDisplayHandle, RawDisplayHandle};

use crate::platform::{
    SurfaceError, SurfaceFrame, SurfaceProvider, acquire_surface_texture,
    select_hydrolysis_surface_format,
};

/// The Android display's raw handle: the platform has one implicit default
/// display, which `AndroidDisplayHandle` names.
#[derive(Debug)]
struct AndroidDisplay;

impl HasDisplayHandle for AndroidDisplay {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::Android(AndroidDisplayHandle::new()))
        })
    }
}

/// A GPU-attachment failure surfaced to the Kotlin host as an exception.
#[derive(Debug)]
pub(crate) struct GpuError(pub String);

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
        })
        .await
        .map_err(|error| {
            GpuError::new(format!(
                "hydrolysis android: no wgpu-capable GPU adapter: {error}"
            ))
        })
}

struct AndroidGpuContextInner {
    instance: wgpu::Instance,
    /// Identity of this device creation chain for the engine pool.
    context_id: u64,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    device_loss: waterui_graphics::DeviceLoss,
}

impl Drop for AndroidGpuContextInner {
    fn drop(&mut self) {
        waterui_graphics::shared_context::drain_device_before_teardown(&self.device);
    }
}

/// The process-wide wgpu context the Android host renders through.
///
/// Vulkan only — the plan of record is Cherenkov over Vulkan, so there is no
/// runtime fallback to another GPU API: a Vulkan adapter that cannot paint
/// is an explicit [`GpuError`] naming the adapter and the missing flags,
/// never a switch to GLES or another painter.
#[derive(Clone)]
pub(crate) struct AndroidGpuContext {
    inner: Arc<AndroidGpuContextInner>,
}

impl AndroidGpuContext {
    /// Requests the GPU context, blocking the calling (UI) thread.
    pub(crate) fn request() -> Result<Self, GpuError> {
        pollster::block_on(Self::request_async())
    }

    async fn request_async() -> Result<Self, GpuError> {
        // The engine's shaders require these capabilities — its compute
        // pipeline uses f16-in-f32 builtins unconditionally, and the pipeline is
        // compute. The default pick may lack them (software Vulkan drivers
        // like llvmpipe drop shaderFloat16), so walk every Vulkan adapter and
        // choose the first that can paint; a device with none is an explicit
        // error, never a painter fallback.
        const PAINTER_FLAGS: wgpu::DownlevelFlags =
            wgpu::DownlevelFlags::COMPUTE_SHADERS.union(wgpu::DownlevelFlags::SHADER_F16_IN_F32);
        let flags_of = |adapter: &wgpu::Adapter| adapter.get_downlevel_capabilities().flags;
        let capable = |adapter: &wgpu::Adapter| flags_of(adapter).contains(PAINTER_FLAGS);
        // A Vulkan-only instance: no other GPU API is ever instantiated, and
        // `create_surface` therefore only ever builds the Vulkan surface that
        // claims the `ANativeWindow`'s single producer connection.
        let instance = new_instance(wgpu::Backends::VULKAN);
        let preferred = request_adapter(&instance).await?;
        let adapter = if capable(&preferred) {
            preferred
        } else {
            let chosen = instance
                .enumerate_adapters(wgpu::Backends::VULKAN)
                .await
                .into_iter()
                .find(capable);
            match chosen {
                Some(candidate) => candidate,
                None => {
                    let info = preferred.get_info();
                    let missing = PAINTER_FLAGS.difference(flags_of(&preferred));
                    return Err(GpuError::new(format!(
                        "hydrolysis android: no Vulkan adapter meets the \
                         painter's requirements — '{}' ({:?}) lacks \
                         {missing:?} and no alternate Vulkan adapter qualifies",
                        info.name, info.backend
                    )));
                }
            }
        };
        crate::platform::ensure_compute_capable_adapter(
            &adapter,
            "hydrolysis android surface",
            "failed to find compute-capable wgpu adapter",
        );
        let required_limits = crate::platform::required_device_limits(&adapter);
        let required_features =
            waterui_graphics::shared_context::required_media_features(adapter.features())
                | (adapter.features()
                    & (wgpu::Features::PIPELINE_CACHE | wgpu::Features::PASSTHROUGH_SHADERS));
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("hydrolysis-android-device"),
                required_features,
                required_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::default(),
            })
            .await
            .map_err(|error| {
                GpuError::new(format!(
                    "hydrolysis android: failed to request wgpu device: {error}"
                ))
            })?;
        let device_loss = waterui_graphics::DeviceLoss::observe(&device);
        Ok(Self {
            inner: Arc::new(AndroidGpuContextInner {
                instance,
                context_id: crate::platform::next_gpu_context_id(),
                adapter,
                device,
                queue,
                device_loss,
            }),
        })
    }
}

/// A wgpu presentation surface over an `ANativeWindow` the Kotlin GPU band
/// hands over.
///
/// `attach` acquires the native window and creates/configures the swapchain;
/// `detach`/`drop` destroy the `wgpu::Surface` — which holds its own acquired
/// `ANativeWindow` reference internally and releases it during surface
/// teardown — before this struct's own lease goes away. Field and drop order
/// are the teardown order: SurfaceFlinger must never see a surface still
/// pointing at a released window.
pub(crate) struct AndroidSurface {
    gpu: AndroidGpuContext,
    /// The generation the Kotlin host assigned this attachment; a stale
    /// `surfaceDestroyed` from an older `SurfaceHolder` callback is ignored.
    generation: u64,
    surface: Option<wgpu::Surface<'static>>,
    /// This side's own lease on the current native window, used for
    /// `ANativeWindow_setFrameRate`; the lease wgpu holds inside `surface`
    /// dies with it.
    native_window: Option<NativeWindow>,
    config: Option<wgpu::SurfaceConfiguration>,
    /// The Kotlin `SurfaceHolder`'s reported geometry, independent of the
    /// wgpu configuration — valid while between surface generations too.
    width: u32,
    height: u32,
    /// API level of the running device, for `ANativeWindow_setFrameRate`
    /// gating (the symbol is API 30+).
    sdk_int: i32,
    /// The high-refresh request currently held, so a no-change demand does
    /// not re-call the platform.
    frame_rate_request: Option<f32>,
}

impl AndroidSurface {
    pub(crate) fn new(gpu: AndroidGpuContext, sdk_int: i32) -> Self {
        Self {
            gpu,
            generation: 0,
            surface: None,
            native_window: None,
            config: None,
            width: 0,
            height: 0,
            sdk_int,
            frame_rate_request: None,
        }
    }

    /// Whether a live `wgpu::Surface` is attached — the band exists and has a
    /// real size the last configure reported.
    pub(crate) fn is_attached(&self) -> bool {
        self.surface.is_some()
    }

    /// Attaches a new native window at `generation`, releasing any prior
    /// attachment first.
    ///
    /// `native_window` is the owned lease obtained from the Kotlin `Surface`
    /// (`ANativeWindow_fromSurface` acquires the reference it returns).
    /// Errors leave the attachment detached.
    pub(crate) fn attach(
        &mut self,
        native_window: NativeWindow,
        width: u32,
        height: u32,
        generation: u64,
    ) -> Result<(), GpuError> {
        self.detach();
        self.width = width;
        self.height = height;
        self.generation = generation;
        self.native_window = Some(native_window);
        if width == 0 || height == 0 {
            // Zero-sized bands never reach `surface.configure` — the
            // attachment stays parked until a real size arrives.
            return Ok(());
        }
        self.configure(width, height)
    }

    fn configure(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        let Some(window) = self.native_window.as_ref() else {
            return Err(GpuError::new(
                "hydrolysis android: configure without a native window".to_owned(),
            ));
        };
        // SAFETY: `window.ptr()` names the ANativeWindow this attachment just
        // acquired; `clone_from_ptr` takes a second lease that the wgpu
        // surface owns for its whole lifetime — it is released inside the
        // surface's drop, before this attachment's own lease can go away.
        let surface = self
            .gpu
            .inner
            .instance
            .create_surface(wgpu::SurfaceTarget::from_window_without_display(unsafe {
                NativeWindow::clone_from_ptr(window.ptr())
            }))
            .map_err(|error| {
                GpuError::new(format!(
                    "hydrolysis android: surface creation failed: {error}"
                ))
            })?;
        let caps = surface.get_capabilities(&self.gpu.inner.adapter);
        if caps.formats.is_empty() {
            return Err(GpuError::new(
                "hydrolysis android: surface reports no formats".to_owned(),
            ));
        }
        let format = select_hydrolysis_surface_format(&caps);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            // FIFO vsync-paced presentation with the plan's swapchain depth.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .iter()
                .copied()
                .find(|mode| *mode == wgpu::CompositeAlphaMode::Opaque)
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
        };
        surface.configure(&self.gpu.inner.device, &config);
        self.config = Some(config);
        self.surface = Some(surface);
        Ok(())
    }

    /// Releases the current attachment: the wgpu surface (and the window
    /// lease it holds) dies first, this side's lease second.
    pub(crate) fn detach(&mut self) {
        self.config = None;
        self.surface = None;
        self.native_window = None;
        self.frame_rate_request = None;
    }

    /// The band's geometry changed: resize bookkeeping plus a reconfigure.
    /// `generation` must match the live attachment — a stale `surfaceChanged`
    /// is ignored.
    pub(crate) fn resize_for(
        &mut self,
        width: u32,
        height: u32,
        generation: u64,
    ) -> Result<(), GpuError> {
        if generation != self.generation || self.native_window.is_none() {
            return Ok(());
        }
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            self.config = None;
            self.surface = None;
            return Ok(());
        }
        if self.surface.is_none() {
            // Parked on a zero size until now: configure for real.
            return self.configure(width, height);
        }
        if let (Some(surface), Some(config)) = (self.surface.as_ref(), self.config.as_mut()) {
            config.width = width;
            config.height = height;
            surface.configure(&self.gpu.inner.device, config);
        }
        Ok(())
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

    /// Requests high refresh while the scheduler reports active interaction
    /// or animation, and releases the request when it goes idle. No-op below
    /// API 30 and when the request is already what the window holds.
    pub(crate) fn set_high_refresh_demand(&mut self, demand: Option<f32>) {
        if self.sdk_int < 30 || self.frame_rate_request == demand {
            return;
        }
        let Some(window) = self.native_window.as_ref() else {
            return;
        };
        let rate = demand.unwrap_or(0.0);
        if window
            .set_frame_rate(rate, ndk::native_window::FrameRateCompatibility::Default)
            .is_ok()
        {
            self.frame_rate_request = demand;
        }
    }
}

impl Drop for AndroidSurface {
    fn drop(&mut self) {
        self.detach();
    }
}

impl SurfaceProvider for AndroidSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        &self.gpu.inner.adapter
    }

    fn device(&self) -> &wgpu::Device {
        &self.gpu.inner.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.gpu.inner.queue
    }

    fn device_loss(&self) -> &waterui_graphics::DeviceLoss {
        &self.gpu.inner.device_loss
    }

    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        if self.gpu.inner.device_loss.is_lost() {
            // Device loss is unrecoverable for this attachment — report it as
            // the explicit error the plan asks for rather than silently
            // failing every acquisition.
            return Err(SurfaceError::Validation);
        }
        let Some(surface) = self.surface.as_ref() else {
            // Between generations there is nothing to acquire against: the
            // frame pump treats Lost like a lost swapchain and reschedules —
            // the next attach drives the redraw through.
            return Err(SurfaceError::Lost);
        };
        let output = acquire_surface_texture(surface)?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(SurfaceFrame::Android { output, view })
    }

    fn present(&mut self, frame: SurfaceFrame) {
        let SurfaceFrame::Android { output, .. } = frame else {
            panic!("hydrolysis android: surface frame mismatched attachment");
        };
        output.present();
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.config
            .as_ref()
            .map(|config| config.format)
            .unwrap_or(wgpu::TextureFormat::Rgba8Unorm)
    }

    fn gpu_context_id(&self) -> u64 {
        self.gpu.inner.context_id
    }

    fn shared_device(&self) -> cherenkov_gpu::interop::SharedDevice {
        let inner = &*self.gpu.inner;
        cherenkov_gpu::interop::SharedDevice {
            instance: inner.instance.clone(),
            adapter: inner.adapter.clone(),
            device: inner.device.clone(),
            queue: inner.queue.clone(),
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            return;
        }
        if let (Some(surface), Some(config)) = (self.surface.as_ref(), self.config.as_mut()) {
            config.width = width;
            config.height = height;
            surface.configure(&self.gpu.inner.device, config);
        }
    }

    fn premultiply_alpha(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|config| config.alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied)
    }
}

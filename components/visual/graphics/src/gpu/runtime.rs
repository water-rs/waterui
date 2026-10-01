//! Shared native GPU devices and engine-composed content presentation.

use alloc::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use alloc::vec::Vec;
use core::fmt;
#[cfg(not(target_arch = "wasm32"))]
use core::sync::atomic::AtomicU64;
use core::sync::atomic::{AtomicBool, Ordering};
use std::string::String;
use std::sync::Mutex;

use wgpu::{Adapter, Device, Instance, Queue, TextureFormat};

use super::external::{ExternalFrameStream, FrameReceiver};
use super::{GpuContent, GpuContentView, RedrawHandle, Shared};
use crate::offscreen::{OffscreenImage, OffscreenSize};
use cherenkov::kurbo::Affine;
use cherenkov::{Display, Engine, FrameTime, Layer, Next, Surface};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{
        ExternalFrame, FramePlanes, GpuContentBox, OutputAlpha, OutputColor, Presenter,
        SharedDevice, TextureOutput, TextureTarget, shader_delivery,
    },
};

/// Why a [`GpuRuntime`] could not be created.
#[derive(Debug, thiserror::Error)]
pub enum GpuRuntimeError {
    /// No adapter satisfied the request.
    #[error("no compatible GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// The adapter refuses passthrough shaders, which a device shared with
    /// cherenkov requires on Vulkan and Metal (cherenkov issue #57).
    #[error(
        "{backend:?} adapter does not offer Features::PASSTHROUGH_SHADERS, which a device shared with cherenkov-gpu requires"
    )]
    PassthroughShadersUnsupported {
        /// The backend the adapter reported.
        backend: wgpu::Backend,
    },
    /// The adapter refused the device.
    #[error(transparent)]
    Device(#[from] wgpu::RequestDeviceError),
}

/// A handle that answers whether its device has been reported lost.
///
/// `DeviceLoss` exists separately from the owning [`SharedGpuContext`] because
/// work that runs off the frame path — a decoder or producer thread keeping
/// its own `wgpu` objects — cannot wait for the owning runtime's rebuild to
/// reach it. Observing the loss directly is what lets that work stop: the
/// next [`GpuContent::setup`] on the rebuilt context replaces everything the
/// worker was producing anyway.
///
/// [`GpuContent::setup`]: super::GpuContent::setup
#[derive(Clone, Default)]
pub struct DeviceLoss {
    reason: Arc<Mutex<Option<String>>>,
    /// Runs once a loss is recorded — the owning [`GpuRuntime`] installs its
    /// rebuild trigger here so recovery starts when the driver reports the
    /// loss instead of waiting for the next frame to notice it.
    wakeup: Arc<Mutex<Option<LossWakeup>>>,
}

impl fmt::Debug for DeviceLoss {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceLoss")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl DeviceLoss {
    /// Starts observing `device`: installs its device-lost callback so the
    /// returned handle reports the loss the moment the driver announces it.
    ///
    /// wgpu keeps one lost callback per device, so this belongs to whoever
    /// owns the device — the runtime's [`SharedGpuContext`] here, or a host
    /// that opened its own — and is called once, right after the device is
    /// created.
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

    /// Installs `wakeup`, run once after the loss is recorded. Only the
    /// device owner calls this: the [`GpuRuntime`] to trigger its rebuild,
    /// or a host that opened its own device.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_wakeup(&self, wakeup: impl Fn() + Send + Sync + 'static) {
        *self
            .wakeup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(Box::new(wakeup) as LossWakeup);
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
        if let Some(wakeup) = &*self
            .wakeup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            wakeup();
        }
    }
}

/// A one-shot action run when a device loss is recorded.
type LossWakeup = Box<dyn Fn() + Send + Sync>;

/// One device generation of a [`GpuRuntime`]: the shared `wgpu` instance,
/// adapter, device and queue the runtime hands out.
///
/// A context is replaced wholesale when the driver reports its device lost:
/// nothing created on it — swapchain, engine, pipeline — survives, so every
/// subsequent `GpuRuntime::context()` call answers a freshly built context on
/// a new device. Callers holding device-bound resources compare
/// [`generation`](Self::generation) against the generation they built under
/// to know they must rebuild them.
#[derive(Debug)]
pub struct SharedGpuContext {
    generation: u64,
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    device_loss: DeviceLoss,
    /// Whether a submitted frame's GPU work finished on this context's device.
    ///
    /// A completed submission is the only success signal a device reports:
    /// [`GpuRuntime`] treats a loss as recoverable only when the lost context
    /// reached this point, and treats a run of losses with no presented frame
    /// between them as a driver that cannot sustain a device at all.
    frame_presented: Arc<AtomicBool>,
}

impl SharedGpuContext {
    /// Requests a high-performance adapter and a device with default limits.
    ///
    /// Device loss otherwise surfaces only as a bare `Validation` status on the
    /// next swapchain acquire, with the reason discarded; the `DeviceLoss`
    /// observer records it so the failure names its cause.
    ///
    /// # Errors
    /// [`GpuRuntimeError`] when no adapter or device is available.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "wgpu's wasm32 request_adapter/request_device resolve through JS promises kept in an Rc<RefCell>, so this future is !Send there; it is Send on native targets"
        )
    )]
    pub async fn new(generation: u64) -> Result<Self, GpuRuntimeError> {
        let instance =
            Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await?;
        // cherenkov's fixed shaders are precompiled and load through wgpu's
        // passthrough API on Vulkan and Metal (cherenkov issue #57): a device
        // handed to cherenkov-gpu via `SharedDevice` must request the feature.
        let backend = adapter.get_info().backend;
        let required_features = match backend {
            wgpu::Backend::Vulkan | wgpu::Backend::Metal => {
                if !adapter
                    .features()
                    .contains(wgpu::Features::PASSTHROUGH_SHADERS)
                {
                    return Err(GpuRuntimeError::PassthroughShadersUnsupported { backend });
                }
                wgpu::Features::PASSTHROUGH_SHADERS
            }
            _ => wgpu::Features::empty(),
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("waterui GpuRuntime"),
                required_features,
                ..Default::default()
            })
            .await?;
        let device_loss = DeviceLoss::observe(&device);
        Ok(Self {
            generation,
            instance,
            adapter,
            device,
            queue,
            device_loss,
            frame_presented: Arc::new(AtomicBool::new(false)),
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

    /// The instance surfaces are created from.
    #[must_use]
    pub const fn instance(&self) -> &Instance {
        &self.instance
    }

    /// The adapter the device was created from.
    #[must_use]
    pub const fn adapter(&self) -> &Adapter {
        &self.adapter
    }

    /// The device.
    #[must_use]
    pub const fn device(&self) -> &Device {
        &self.device
    }

    /// The device's queue.
    #[must_use]
    pub const fn queue(&self) -> &Queue {
        &self.queue
    }

    /// The device-lost reason recorded by the callback, once the driver reports
    /// one. A `Some` here means every device-bound resource on this context is
    /// dead: swapchain acquire, configure and pipeline use all fail until a new
    /// runtime is created.
    #[must_use]
    pub fn device_lost_reason(&self) -> Option<String> {
        self.device_loss.reason()
    }

    /// A handle that answers whether this context's device has been lost, for
    /// work that runs off the frame path and cannot wait for the owning
    /// runtime's rebuild to reach it.
    #[must_use]
    pub fn device_loss(&self) -> DeviceLoss {
        self.device_loss.clone()
    }

    /// Records a device loss the driver never reported, for tests that
    /// exercise the runtime's recreation path.
    #[doc(hidden)]
    pub fn mark_device_lost_for_testing(&self, reason: &str) {
        self.device_loss.record(reason.to_owned());
    }

    /// Whether this device generation presented a frame to completion.
    ///
    /// A lost generation that presented at least once counts as an ordinary
    /// recoverable loss; a run of generations that each died first is a driver
    /// that cannot sustain a device, and [`GpuRuntime::context`] caps its
    /// rebuilds there.
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
    /// Registers an idle-queue callback ordered after the presented frame's
    /// real work, so its invocation proves the frame's GPU work finished. The
    /// callback resolves when `wgpu` next maintains the queue — on the next
    /// submission — which is soon enough for the rebuild budget, the only
    /// reader of the flag.
    pub fn note_frame_presented(&self) {
        let frame_presented = Arc::clone(&self.frame_presented);
        self.queue.on_submitted_work_done(move || {
            frame_presented.store(true, Ordering::Relaxed);
        });
    }
}

/// The budget for devices that die before ever presenting a frame.
///
/// One stillborn device is hardware noise; several in a row mean the driver
/// loses every device it hands out, so past this count [`GpuRuntime::context`]
/// reports the recorded losses instead of paying for another stillborn
/// device.
#[cfg(not(target_arch = "wasm32"))]
const MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS: usize = 3;

/// Cloneable owner for one explicitly-created shared GPU context.
///
/// The context is recreated in place when the driver reports its device lost:
/// [`GpuRuntime::context`] notices the recorded loss and swaps in a freshly
/// built [`SharedGpuContext`] — new instance, adapter and device — so every
/// subsequent caller is back on live hardware. The swap is what makes
/// recovery possible at all: a dead device cannot honour a surface, so
/// nothing built on it is salvageable.
///
/// Recovery is bounded: a recreation only counts as recovery when the device
/// it replaced presented at least one frame. Consecutive losses of devices
/// that never presented — the signature of a driver that loses every device
/// it hands out — are capped at [`MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS`],
/// after which [`GpuRuntime::context`] panics with the collected loss reasons
/// instead of paying for another stillborn device.
#[derive(Clone)]
pub struct GpuRuntime {
    inner: Shared<RuntimeInner>,
}

struct RuntimeInner {
    /// The live context, replaced when the spawned rebuild lands after its
    /// device was reported lost.
    context: Mutex<Shared<SharedGpuContext>>,
    /// The recorded reasons of consecutive losses that produced no presented
    /// frame — the current streak of stillborn devices. A context lost after
    /// presenting real work is an ordinary recoverable loss and clears the
    /// streak. WebGPU never rebuilds, so the streak exists on native targets
    /// only.
    #[cfg(not(target_arch = "wasm32"))]
    unproductive_losses: Mutex<Vec<String>>,
    /// Generation handed to the next rebuilt context. WebGPU has no
    /// asynchronous rebuild path, so on wasm32 no context is ever rebuilt.
    #[cfg(not(target_arch = "wasm32"))]
    next_generation: AtomicU64,
    /// Set while a rebuild runs on its spawned thread so at most one is in
    /// flight; callers keep receiving the still-lost context meanwhile and
    /// report the frame pending.
    #[cfg(not(target_arch = "wasm32"))]
    rebuild_in_flight: AtomicBool,
    /// The reasons the rebuild budget tripped on. The budget can trip inside
    /// the device-lost callback, where unwinding cannot run, so the verdict
    /// is stored here and the next [`GpuRuntime::context`] call panics with
    /// it on its own thread.
    #[cfg(not(target_arch = "wasm32"))]
    rebuild_exhausted: Mutex<Option<String>>,
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
    /// [`GpuRuntimeError`] when no adapter or device is available, or the
    /// adapter cannot provide the passthrough-shader feature cherenkov
    /// requires of a shared device.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "`wgpu::Instance::request_adapter` resolves through `navigator.gpu.requestAdapter()`, a JS promise the WebGPU backend keeps in an `Rc<RefCell<_>>`; the same future is `Send` on every other target"
        )
    )]
    pub async fn new() -> Result<Self, GpuRuntimeError> {
        let inner = Shared::new(RuntimeInner {
            context: Mutex::new(Shared::new(SharedGpuContext::new(0).await?)),
            #[cfg(not(target_arch = "wasm32"))]
            unproductive_losses: Mutex::new(Vec::new()),
            #[cfg(not(target_arch = "wasm32"))]
            next_generation: AtomicU64::new(1),
            #[cfg(not(target_arch = "wasm32"))]
            rebuild_in_flight: AtomicBool::new(false),
            #[cfg(not(target_arch = "wasm32"))]
            rebuild_exhausted: Mutex::new(None),
        });
        #[cfg(not(target_arch = "wasm32"))]
        Self::arm_rebuild_wakeup(&inner);
        Ok(Self { inner })
    }

    /// Hooks the live context's device-lost observer so the rebuild starts
    /// the moment the driver reports the loss instead of waiting for the
    /// next [`context`](Self::context) call.
    ///
    /// The observer is rebuilt with the context, so the rebuild thread arms
    /// the fresh context's `DeviceLoss` the same way through
    /// [`install_rebuild_wakeup`](Self::install_rebuild_wakeup).
    #[cfg(not(target_arch = "wasm32"))]
    fn arm_rebuild_wakeup(inner: &Shared<RuntimeInner>) {
        let device_loss = inner
            .context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .device_loss();
        Self::install_rebuild_wakeup(&device_loss, inner);
    }

    /// Installs the rebuild trigger on `device_loss`, held weakly so a lost
    /// context's callback cannot keep a dropped runtime alive.
    #[cfg(not(target_arch = "wasm32"))]
    fn install_rebuild_wakeup(device_loss: &DeviceLoss, inner: &Shared<RuntimeInner>) {
        let weak = Shared::downgrade(inner);
        device_loss.set_wakeup(move || {
            if let Some(inner) = weak.upgrade() {
                Self::start_rebuild(&inner);
            }
        });
    }

    /// Returns this runtime's shared GPU resources.
    ///
    /// While the driver-reported device loss is being recovered, this returns
    /// the still-lost context — its [`SharedGpuContext::device_lost_reason`]
    /// names the cause and callers report their frame pending — and a spawned
    /// thread performs the rebuild, so a driver call never runs on the
    /// frame's caller. Callers holding device-bound resources from an earlier
    /// call compare [`SharedGpuContext::generation`] to know they must
    /// rebuild them once a fresh context lands.
    ///
    /// # Panics
    ///
    /// Panics once [`MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS`] devices in a row
    /// were each lost before presenting a frame — a driver that loses every
    /// device it hands out is not recoverable — and reports the collected
    /// loss reasons.
    #[must_use]
    pub fn context(&self) -> Shared<SharedGpuContext> {
        {
            let slot = self
                .inner
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.device_lost_reason().is_none() {
                return Shared::clone(&slot);
            }
        }
        self.request_rebuild()
    }

    /// The lost half of [`context`](Self::context): kicks the rebuild and
    /// answers whatever context is current — usually still the lost one.
    #[cfg(not(target_arch = "wasm32"))]
    fn request_rebuild(&self) -> Shared<SharedGpuContext> {
        let exhausted = self
            .inner
            .rebuild_exhausted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(reasons) = exhausted {
            panic!("{reasons}");
        }
        Self::start_rebuild(&self.inner);
        Shared::clone(
            &self
                .inner
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// WebGPU reports device loss but offers no asynchronous rebuild path —
    /// `request_adapter` is a JS promise with no executor to run it here — so
    /// the lost context stays in place and keeps naming its cause.
    #[cfg(target_arch = "wasm32")]
    fn request_rebuild(&self) -> Shared<SharedGpuContext> {
        Shared::clone(
            &self
                .inner
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Spawns the context rebuild, once per device loss.
    ///
    /// `request_adapter`/`request_device` are driver calls that take tens of
    /// milliseconds; they run on the spawned thread, the same shape
    /// `create_gpu_runtime` uses for the first runtime, so the frame's caller
    /// never waits on a driver. `rebuild_in_flight` serializes attempts: the
    /// device-lost wakeup and every `context()` call that finds the loss all
    /// funnel into one rebuild, and the still-lost context is returned until
    /// the fresh one lands.
    ///
    /// A failed rebuild keeps the dead context in place and records the
    /// failure as a loss, so the next trigger retries under the same budget
    /// the losses themselves count against.
    #[cfg(not(target_arch = "wasm32"))]
    fn start_rebuild(inner: &Shared<RuntimeInner>) {
        if inner.rebuild_in_flight.swap(true, Ordering::AcqRel) {
            return;
        }
        let generation = {
            let slot = inner
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.device_lost_reason().is_none() {
                // The context was already replaced; nothing to rebuild.
                inner.rebuild_in_flight.store(false, Ordering::Release);
                return;
            }
            let mut unproductive = inner
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
            if unproductive.len() > MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS {
                let message = format!(
                    "WaterUI GPU device was lost {} times in a row without ever \
                     presenting a frame; the device is unrecoverable. Recorded losses: {}",
                    unproductive.len(),
                    unproductive.join(" | ")
                );
                drop(unproductive);
                drop(slot);
                *inner
                    .rebuild_exhausted
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message);
                // `rebuild_in_flight` stays set: the device is unrecoverable,
                // so no further attempt is ever spawned.
                return;
            }
            drop(unproductive);
            drop(slot);
            inner.next_generation.fetch_add(1, Ordering::Relaxed)
        };

        let worker = Shared::clone(inner);
        std::thread::Builder::new()
            .name("waterui-gpu-runtime-rebuild".to_owned())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    pollster::block_on(SharedGpuContext::new(generation))
                }));
                match outcome {
                    Ok(Ok(fresh)) => {
                        let fresh = Shared::new(fresh);
                        tracing::warn!(
                            generation,
                            adapter = %fresh.adapter().get_info().name,
                            "GPU device was lost; recreated the runtime context"
                        );
                        Self::install_rebuild_wakeup(&fresh.device_loss(), &worker);
                        // Only this thread replaces the slot: `rebuild_in_flight`
                        // stays set until the store below, so no second writer
                        // can interleave.
                        *worker
                            .context
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = fresh;
                    }
                    Ok(Err(error)) => {
                        worker
                            .unproductive_losses
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(format!("context recreation failed: {error}"));
                        tracing::error!(
                            "GPU device was lost and recreation failed ({error}); \
                             retrying on the next trigger"
                        );
                    }
                    Err(payload) => {
                        worker.rebuild_in_flight.store(false, Ordering::Release);
                        std::panic::resume_unwind(payload);
                    }
                }
                worker.rebuild_in_flight.store(false, Ordering::Release);
            })
            .expect("failed to spawn the GPU runtime rebuild thread");
    }

    /// The instance surfaces are created from.
    ///
    /// Convenience for `self.context().instance().clone()`; callers holding
    /// device-bound resources must take [`context`](Self::context) instead so
    /// they can compare its generation.
    #[must_use]
    pub fn instance(&self) -> Instance {
        self.context().instance().clone()
    }

    /// The adapter the device was created from.
    ///
    /// See [`instance`](Self::instance) for the generation contract.
    #[must_use]
    pub fn adapter(&self) -> Adapter {
        self.context().adapter().clone()
    }

    /// The device.
    ///
    /// See [`instance`](Self::instance) for the generation contract.
    #[must_use]
    pub fn device(&self) -> Device {
        self.context().device().clone()
    }

    /// The device's queue.
    ///
    /// See [`instance`](Self::instance) for the generation contract.
    #[must_use]
    pub fn queue(&self) -> Queue {
        self.context().queue().clone()
    }

    /// Creates an engine sharing this runtime's current context's device.
    ///
    /// The engine is bound to the context generation it was built under: a
    /// rebuilt context after device loss needs a new engine, created from the
    /// replacement context.
    ///
    /// # Errors
    /// When the engine cannot initialize its rendering resources.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn engine(&self) -> Result<Engine<Gpu>, cherenkov::EngineError> {
        self.engine_on(&self.context())
    }

    /// Creates an engine sharing this runtime's current context's device.
    ///
    /// # Errors
    /// When the engine cannot initialize its rendering resources.
    #[cfg(target_arch = "wasm32")]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
        )
    )]
    pub async fn engine(&self) -> Result<Engine<Gpu>, cherenkov::EngineError> {
        self.engine_on(&self.context()).await
    }

    /// Creates an engine on `context` — the generation a caller that also
    /// holds device-bound resources must pass so everything agrees.
    ///
    /// # Errors
    /// When the engine cannot initialize its rendering resources.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn engine_on(
        &self,
        context: &SharedGpuContext,
    ) -> Result<Engine<Gpu>, cherenkov::EngineError> {
        Engine::new(GpuConfig {
            device: Some(SharedDevice {
                instance: context.instance().clone(),
                adapter: context.adapter().clone(),
                device: context.device().clone(),
                queue: context.queue().clone(),
            }),
            ..GpuConfig::default()
        })
    }

    /// Creates an engine on `context` — the generation a caller that also
    /// holds device-bound resources must pass so everything agrees.
    ///
    /// # Errors
    /// When the engine cannot initialize its rendering resources.
    #[cfg(target_arch = "wasm32")]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
        )
    )]
    pub async fn engine_on(
        &self,
        context: &SharedGpuContext,
    ) -> Result<Engine<Gpu>, cherenkov::EngineError> {
        Engine::new(GpuConfig {
            device: Some(SharedDevice {
                instance: context.instance().clone(),
                adapter: context.adapter().clone(),
                device: context.device().clone(),
                queue: context.queue().clone(),
            }),
            ..GpuConfig::default()
        })
        .await
    }

    /// Renders owned GPU content through the engine's offscreen surface.
    ///
    /// # Panics
    /// When engine creation, rendering, or readback fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render_content(
        &self,
        content: impl GpuContent,
        size: OffscreenSize,
        scale: f32,
    ) -> OffscreenImage {
        let content = GpuContentView::new(content).take_engine_content(|| {});
        let mut renderer = GpuContentRenderer::new(self, content, size);
        renderer.render(
            size,
            Display {
                scale: f64::from(scale),
                headroom: 1.0,
            },
        );
        OffscreenImage::from_readback(
            &renderer
                .host
                .surface
                .readback()
                .expect("GPU content readback failed"),
        )
    }

    /// Renders owned GPU content through the engine's offscreen surface.
    ///
    /// # Panics
    /// When engine creation, rendering, or readback fails.
    #[cfg(target_arch = "wasm32")]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
        )
    )]
    pub async fn render_content(
        &self,
        content: impl GpuContent,
        size: OffscreenSize,
        scale: f32,
    ) -> OffscreenImage {
        let content = GpuContentView::new(content).take_engine_content(|| {});
        let mut renderer = GpuContentRenderer::new(self, content, size).await;
        renderer
            .render(
                size,
                Display {
                    scale: f64::from(scale),
                    headroom: 1.0,
                },
            )
            .await;
        OffscreenImage::from_readback(
            &renderer
                .host
                .surface
                .readback()
                .await
                .expect("GPU content readback failed"),
        )
    }
}

/// The swapchain format a host configures for `capabilities`.
///
/// Prefers a 16-bit float format when the host asks for HDR and the surface
/// offers one; otherwise the first 8-bit sRGB-encoded format, then the first
/// format the surface offers at all.
///
/// # Panics
/// When the surface offers no format.
#[must_use]
pub fn preferred_surface_format(
    capabilities: &wgpu::SurfaceCapabilities,
    prefer_hdr: bool,
) -> TextureFormat {
    let formats = &capabilities.formats;
    if prefer_hdr && let Some(f) = formats.iter().find(|f| **f == TextureFormat::Rgba16Float) {
        return *f;
    }
    formats
        .iter()
        .find(|f| f.is_srgb())
        .or_else(|| formats.first())
        .copied()
        .expect("surface offers no texture format")
}

/// The engine, retained surface and presenter one hosted layer renders
/// through, shared by [`GpuContentRenderer`] and [`ExternalFrameRenderer`].
///
/// Everything here is bound to the [`SharedGpuContext`] generation it was
/// created under.
struct LayerHost {
    surface: Surface<Gpu>,
    engine: Engine<Gpu>,
    context: Shared<SharedGpuContext>,
    textures: std::sync::mpsc::Receiver<wgpu::Texture>,
    source: wgpu::Texture,
    presenter: Presenter,
}

impl LayerHost {
    /// Creates an engine and a retained surface on the runtime's current
    /// context.
    ///
    /// # Panics
    /// When engine or surface creation fails.
    #[cfg(not(target_arch = "wasm32"))]
    fn new(runtime: &GpuRuntime, size: OffscreenSize) -> Self {
        let context = runtime.context();
        let engine = runtime
            .engine_on(&context)
            .expect("native content engine creation failed");
        let (target, textures) = TextureTarget::new((size.width(), size.height()));
        let surface = engine
            .surface(target)
            .expect("native content surface creation failed");
        Self::assemble(surface, engine, context, textures)
    }

    /// Creates an engine and a retained surface on the runtime's current
    /// context.
    ///
    /// # Panics
    /// When engine or surface creation fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    async fn new(runtime: &GpuRuntime, size: OffscreenSize) -> Self {
        let context = runtime.context();
        let engine = runtime
            .engine_on(&context)
            .await
            .expect("native content engine creation failed");
        let (target, textures) = TextureTarget::new((size.width(), size.height()));
        let surface = engine
            .surface(target)
            .await
            .expect("native content surface creation failed");
        Self::assemble(surface, engine, context, textures)
    }

    fn assemble(
        surface: Surface<Gpu>,
        engine: Engine<Gpu>,
        context: Shared<SharedGpuContext>,
        textures: std::sync::mpsc::Receiver<wgpu::Texture>,
    ) -> Self {
        let source = textures
            .try_recv()
            .expect("surface creation published its texture");
        let presenter = Presenter::new(
            context.device(),
            shader_delivery(context.adapter().get_info().backend, context.device())
                .expect("native content present shaders failed"),
        );
        Self {
            surface,
            engine,
            context,
            textures,
            source,
            presenter,
        }
    }

    /// Resizes the surface to `size`, answering whether it changed.
    ///
    /// # Panics
    /// When the engine rejects the size.
    fn resize(&self, size: OffscreenSize) -> bool {
        let pixels = (size.width(), size.height());
        if self.surface.size() == pixels {
            return false;
        }
        self.surface
            .resize(pixels)
            .expect("native content resize failed");
        true
    }

    /// Runs one engine pass for `display` and picks up the texture a resize
    /// published.
    ///
    /// # Panics
    /// When display configuration or rendering fails.
    #[cfg(not(target_arch = "wasm32"))]
    fn render(&mut self, display: Display) -> Next {
        self.surface
            .display(display)
            .expect("native content display configuration failed");
        let next = self
            .engine
            .render(FrameTime::now())
            .expect("native content rendering failed");
        self.adopt_published_texture();
        next
    }

    /// Runs one engine pass for `display` and picks up the texture a resize
    /// published.
    ///
    /// # Panics
    /// When display configuration or rendering fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    async fn render(&mut self, display: Display) -> Next {
        self.surface
            .display(display)
            .expect("native content display configuration failed");
        let next = self
            .engine
            .render(FrameTime::now())
            .await
            .expect("native content rendering failed");
        self.adopt_published_texture();
        next
    }

    fn adopt_published_texture(&mut self) {
        for texture in self.textures.try_iter() {
            self.source = texture;
        }
    }

    /// Composites the surface's texture into a native host's texture.
    /// Float targets carry extended linear Display P3; other targets carry
    /// sRGB.
    fn composite(&mut self, target: &wgpu::Texture, display: Display) {
        let source = self
            .source
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.presenter.texture(
            self.context.device(),
            self.context.queue(),
            &source,
            TextureOutput {
                texture: target,
                color: if target.format() == TextureFormat::Rgba16Float {
                    OutputColor::LinearDisplayP3
                } else {
                    OutputColor::Srgb
                },
                alpha: OutputAlpha::Premultiplied,
                headroom: display.headroom,
            },
        );
    }
}

/// The size of the host texture `present` renders for.
fn target_size(target: &wgpu::Texture) -> OffscreenSize {
    OffscreenSize::try_from_pixels(target.width(), target.height())
        .expect("native target must be nonempty")
}

/// A retained engine surface for GPU content presented by a native host.
///
/// The renderer is bound to the [`SharedGpuContext`] generation it was created
/// under: [`generation`](Self::generation) reports it so the host can drop and
/// recreate the renderer — engine, surface and presenter are all device-bound
/// — when the runtime's context is rebuilt after device loss.
pub struct GpuContentRenderer {
    host: LayerHost,
}

impl fmt::Debug for GpuContentRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContentRenderer").finish_non_exhaustive()
    }
}

impl GpuContentRenderer {
    /// Moves the producer to a retained engine layer on the runtime's current
    /// context.
    ///
    /// # Panics
    /// When engine or surface creation fails.
    #[must_use]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(runtime: &GpuRuntime, content: GpuContentBox, size: OffscreenSize) -> Self {
        Self::install(LayerHost::new(runtime, size), content, size)
    }

    /// Moves the producer to a retained engine layer on the runtime's current
    /// context.
    ///
    /// # Panics
    /// When engine or surface creation fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    pub async fn new(runtime: &GpuRuntime, content: GpuContentBox, size: OffscreenSize) -> Self {
        Self::install(LayerHost::new(runtime, size).await, content, size)
    }

    fn install(host: LayerHost, content: GpuContentBox, size: OffscreenSize) -> Self {
        let pixels = (size.width(), size.height());
        host.surface.update(|tx| {
            tx[host.surface.root()].content(host.engine.gpu_content(pixels, content));
        });
        Self { host }
    }

    /// The context generation this renderer was built under.
    ///
    /// A renderer whose generation no longer matches
    /// `runtime.context().generation()` is bound to a dead device; drop it and
    /// create a new one on the rebuilt context.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.host.context.generation()
    }

    /// Resizes the retained attachment along with the surface.
    fn resize(&self, size: OffscreenSize) {
        if self.host.resize(size) {
            self.host.surface.update(|tx| {
                tx[self.host.surface.root()].gpu_content_size((size.width(), size.height()));
            });
        }
    }

    /// Renders a frame at the current size for the host's display.
    ///
    /// # Panics
    /// When resizing, display configuration, or rendering fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(&mut self, size: OffscreenSize, display: Display) -> Next {
        self.resize(size);
        self.host.render(display)
    }

    /// Renders a frame at the current size for the host's display.
    ///
    /// # Panics
    /// When resizing, display configuration, or rendering fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    pub async fn render(&mut self, size: OffscreenSize, display: Display) -> Next {
        self.resize(size);
        self.host.render(display).await
    }

    /// Renders and composites into a native host's texture.
    /// Float targets carry extended linear Display P3; other targets carry sRGB.
    ///
    /// # Panics
    /// When the destination is empty or rendering fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn present(&mut self, target: &wgpu::Texture, display: Display) -> Next {
        let next = self.render(target_size(target), display);
        self.host.composite(target, display);
        next
    }

    /// Renders and composites into a native host's texture.
    /// Float targets carry extended linear Display P3; other targets carry sRGB.
    ///
    /// # Panics
    /// When the destination is empty or rendering fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    pub async fn present(&mut self, target: &wgpu::Texture, display: Display) -> Next {
        let next = self.render(target_size(target), display).await;
        self.host.composite(target, display);
        next
    }
}

/// A retained engine surface whose layer shows the frames an
/// [`ExternalFrameStream`] publishes, for a native host.
///
/// The frames are the content of a dedicated child layer of the surface's
/// root — a plain layer with the default blend and no backdrop — stretched to
/// the surface. Each pass drains the stream's mailbox before the engine
/// renders, so a published frame reaches the screen without a view rebuild;
/// its planes are sampled in place, never copied.
///
/// Like [`GpuContentRenderer`], the renderer is bound to the
/// [`SharedGpuContext`] generation it was created under. Dropping it retires
/// the output its source was started with; a replacement renderer on a
/// rebuilt context starts the source again on the new device.
pub struct ExternalFrameRenderer {
    host: LayerHost,
    layer: Layer,
    frames: FrameReceiver,
    /// The plane size of the installed frame, in pixels.
    frame_size: Option<(u32, u32)>,
}

impl fmt::Debug for ExternalFrameRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalFrameRenderer")
            .field("frame_size", &self.frame_size)
            .finish_non_exhaustive()
    }
}

impl ExternalFrameRenderer {
    /// Builds the layer on the runtime's current context and starts the
    /// stream's source on that device. `redraw` wakes the host whenever the
    /// source publishes a frame.
    ///
    /// # Panics
    /// When engine or surface creation fails.
    #[must_use]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        runtime: &GpuRuntime,
        stream: &ExternalFrameStream,
        size: OffscreenSize,
        redraw: RedrawHandle,
    ) -> Self {
        Self::install(LayerHost::new(runtime, size), stream, redraw)
    }

    /// Builds the layer on the runtime's current context and starts the
    /// stream's source on that device. `redraw` wakes the host whenever the
    /// source publishes a frame.
    ///
    /// # Panics
    /// When engine or surface creation fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    pub async fn new(
        runtime: &GpuRuntime,
        stream: &ExternalFrameStream,
        size: OffscreenSize,
        redraw: RedrawHandle,
    ) -> Self {
        Self::install(LayerHost::new(runtime, size).await, stream, redraw)
    }

    fn install(host: LayerHost, stream: &ExternalFrameStream, redraw: RedrawHandle) -> Self {
        let layer = host.surface.layer();
        host.surface.update(|tx| {
            tx[host.surface.root()].push(&layer);
        });
        let frames = stream.start(host.context.device(), host.context.queue(), redraw);
        Self {
            host,
            layer,
            frames,
            frame_size: None,
        }
    }

    /// The context generation this renderer was built under.
    ///
    /// A renderer whose generation no longer matches
    /// `runtime.context().generation()` is bound to a dead device; drop it and
    /// create a new one on the rebuilt context.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.host.context.generation()
    }

    /// Resizes the surface, installs the newest published frame, and keeps
    /// the frame stretched to the surface.
    fn prepare(&mut self, size: OffscreenSize) {
        let resized = self.host.resize(size);
        let frame = self.frames.take();
        if frame.is_none() && !resized {
            return;
        }
        let surface = &self.host.surface;
        let engine = &self.host.engine;
        let layer = &self.layer;
        let mut frame_size = self.frame_size;
        surface.update(|tx| {
            if let Some(frame) = frame {
                frame_size = Some(plane_size(&frame));
                tx[layer].content(engine.external_frame(frame));
            }
            if let Some((width, height)) = frame_size {
                tx[layer].transform(Affine::scale_non_uniform(
                    f64::from(size.width()) / f64::from(width),
                    f64::from(size.height()) / f64::from(height),
                ));
            }
        });
        self.frame_size = frame_size;
    }

    /// Renders a frame at the current size for the host's display.
    ///
    /// # Panics
    /// When resizing, display configuration, or rendering fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(&mut self, size: OffscreenSize, display: Display) -> Next {
        self.prepare(size);
        self.host.render(display)
    }

    /// Renders a frame at the current size for the host's display.
    ///
    /// # Panics
    /// When resizing, display configuration, or rendering fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    pub async fn render(&mut self, size: OffscreenSize, display: Display) -> Next {
        self.prepare(size);
        self.host.render(display).await
    }

    /// Renders and composites into a native host's texture.
    /// Float targets carry extended linear Display P3; other targets carry sRGB.
    ///
    /// # Panics
    /// When the destination is empty or rendering fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn present(&mut self, target: &wgpu::Texture, display: Display) -> Next {
        let next = self.render(target_size(target), display);
        self.host.composite(target, display);
        next
    }

    /// Renders and composites into a native host's texture.
    /// Float targets carry extended linear Display P3; other targets carry sRGB.
    ///
    /// # Panics
    /// When the destination is empty or rendering fails.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )]
    pub async fn present(&mut self, target: &wgpu::Texture, display: Display) -> Next {
        let next = self.render(target_size(target), display).await;
        self.host.composite(target, display);
        next
    }
}

/// A frame's plane size in pixels: the luma plane for YUV, the plane for RGB
/// — the size the engine emits the frame's quad at in layer space.
fn plane_size(frame: &ExternalFrame) -> (u32, u32) {
    let plane = match &frame.planes {
        FramePlanes::Yuv { y, .. } => y,
        FramePlanes::Rgb { plane, .. } => plane,
        #[cfg(all(unix, not(target_vendor = "apple")))]
        FramePlanes::Native(native) => return native.size(),
    };
    (plane.width(), plane.height())
}

//! Vulkan external-frame import (#166).
//!
//! A frame's planes are imported in place — a Linux dma-buf or an Android
//! `AHardwareBuffer` — and synchronised with the producer through a Vulkan
//! semaphore on the GPU, never a CPU wait. Three representations exist:
//!
//! - Ordinary single-plane RGB wraps as a `wgpu::Texture` and draws through
//!   the regular external pipeline.
//! - Known-format multiplanar (NV12/P010) binds native per-plane views in
//!   the Vulkan external composition operation.
//! - Opaque external-format YCbCr binds a combined sampled image whose
//!   immutable sampler carries a `VkSamplerYcbcrConversion`; the sampler's
//!   matrix yields encoded `R'G'B'` and the shader still performs the
//!   transfer decode and primaries conversion — the Y'CbCr matrix is never
//!   applied twice.
//!
//! [`Shared`] is the per-`VkDevice` context both the host-side import
//! ([`Device`]) and the renderer-side encode ([`Native`]) resolve to through
//! a registry keyed by the device handle, so generations created by either
//! side are the same synchronization objects.

use std::collections::hash_map::Entry;
use std::ffi::CStr;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, Weak};

use ash::vk;
use ash::vk::Handle as _;
use rustc_hash::FxHashMap;

use crate::interop::{ChromaOffset, FrameColor, RgbAlpha};

mod ahb;
mod dmabuf;
mod sync;
mod ycbcr;

pub use sync::{Generation, PendingAcquire, PendingWait, State, cancel_staged, stage_acquire};
#[cfg(target_os = "android")]
pub use sync::{PlaneAcquire, PlaneSource, import_sync_fd};
pub use sync::{Release, drain_destroys, drain_releases, mark_owned, mark_submitted, submit_waits};

/// `QueueFamily` the producer released the image on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFamily {
    /// `VK_QUEUE_FAMILY_EXTERNAL`: a foreign queue family of another device.
    External,
    /// `VK_QUEUE_FAMILY_FOREIGN_EXT`: a foreign queue family that does not
    /// exist in any Vulkan instance (the Android producer contract).
    Foreign,
    /// A concrete family index.
    Index(u32),
}

impl QueueFamily {
    const fn vk(self) -> u32 {
        match self {
            Self::External => vk::QUEUE_FAMILY_EXTERNAL,
            Self::Foreign => vk::QUEUE_FAMILY_FOREIGN_EXT,
            Self::Index(i) => i,
        }
    }
}

/// A GPU-side wait ordering the first read of an imported frame behind the
/// producer's work.
///
/// Every variant resolves to a `VkSemaphore` + optional timeline value the
/// engine registers through `vulkan::Queue::add_wait_semaphore` on the
/// consuming submission — the wait executes on the GPU.
#[derive(Debug)]
#[non_exhaustive]
pub enum Wait {
    /// An `OPAQUE_FD` semaphore payload. The fd is consumed into a binary
    /// semaphore at first use; the engine takes ownership of the fd.
    OpaqueFd {
        /// The payload descriptor; owned and closed by the engine.
        fd: OwnedFd,
    },
    /// A Linux sync fd / Android fence payload, imported as a one-shot
    /// binary semaphore. The fd is consumed into a binary semaphore at
    /// first use; the engine takes ownership of it — the same contract as
    /// `OpaqueFd` (`vkImportSemaphoreFdKHR` takes the fd on success).
    SyncFd {
        /// The payload descriptor; owned and closed by the engine.
        fd: OwnedFd,
    },
    /// A host-owned `VkSemaphore` (as `u64`) of timeline kind, waited on at
    /// `value`. After acquisition later reads on the same ordered queue do
    /// not wait again.
    Timeline {
        /// The timeline semaphore's `VkSemaphore` handle, as `u64`.
        semaphore: u64,
        /// The wait point.
        value: u64,
    },
}

/// What the engine signals when the frame's last retained owner retires.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReleaseSync {
    /// The engine exports a `SYNC_FD` payload once the release submission is
    /// accepted; `Frame::release_fd` returns it. Requires
    /// `Caps::external_semaphore_sync_fd`.
    FenceFd,
    /// Signal `value` on the host-owned timeline `VkSemaphore` (as `u64`) in
    /// the release submission.
    Timeline {
        /// The timeline semaphore's `VkSemaphore` handle, as `u64`.
        semaphore: u64,
        /// The wait point.
        value: u64,
    },
}

/// One colour-format plane of a dma-buf image.
///
/// Colour planes are not memory planes: a two-plane NV12 image may live in
/// one dma-buf (both entries name `memory` 0) or two file descriptors.
#[derive(Debug)]
pub struct DmaBufPlane {
    /// Which entry of `DmaBuf::memory` the plane's bytes come from.
    pub memory: u32,
    /// Byte offset of the plane inside its memory plane.
    pub offset: u32,
    /// Byte stride between rows of the plane.
    pub stride: u32,
}

/// A complete Linux dma-buf frame descriptor.
///
/// Plane offsets and strides are the producer's own values — they are never
/// inferred from the image extent — and `layout` / `producer_family` are the
/// producer's actual released state, not a convenience value.
#[derive(Debug)]
pub struct DmaBuf {
    /// The DRM fourcc (`DRM_FORMAT_*`), e.g. `NV12`, `P010`, `ARGB8888`.
    pub fourcc: u32,
    /// The `DRM_FORMAT_MOD_*` layout modifier.
    pub modifier: u64,
    /// Pixel extent of the colour image.
    pub size: (u32, u32),
    /// Colour planes in image-aspect order — one entry per plane the
    /// format defines (1 for RGB, 2 for NV12/P010).
    pub planes: Vec<DmaBufPlane>,
    /// One file descriptor per memory plane. Ownership transfers to the
    /// engine: a successful Vulkan memory import consumes the fd; on
    /// failure the engine closes every still-owned fd.
    pub memory: Vec<OwnedFd>,
    /// The `VkImageLayout` value the producer left the image in.
    pub layout: u32,
    /// The queue family the producer released ownership on.
    pub producer_family: QueueFamily,
    /// The producer synchronization the first read waits on.
    pub sync: Option<Wait>,
    /// What the engine signals back when the frame retires.
    pub release: Option<ReleaseSync>,
    /// How the planes decode into the working space.
    pub color: FrameColor,
    /// How the RGB plane's alpha composes; ignored for YUV.
    pub alpha: RgbAlpha,
}

/// An Android `AHardwareBuffer` frame.
///
/// The producer's ownership contract is fixed: foreign queue family and no
/// prior Vulkan layout, per the external-hardware-buffer import rules.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct Ahb {
    /// `AHardwareBuffer *` as a raw pointer; the import retains it for the
    /// frame's lifetime and releases it when the frame retires.
    pub buffer: *mut core::ffi::c_void,
    /// The producer synchronization the first read waits on.
    pub sync: Option<Wait>,
    /// What the engine signals back when the frame retires.
    pub release: Option<ReleaseSync>,
    /// How the planes decode into the working space.
    pub color: FrameColor,
    /// How the RGB plane's alpha composes; ignored for YUV.
    pub alpha: RgbAlpha,
    /// Static HDR metadata, read when the frame is promoted to a system
    /// compositor plane.
    pub hdr: crate::interop::HdrMetadata,
}

/// A producer frame source the engine imports natively on Vulkan.
#[derive(Debug)]
#[non_exhaustive]
pub enum FrameSource {
    /// A Linux dma-buf.
    DmaBuf(Box<DmaBuf>),
    /// An Android `AHardwareBuffer`.
    #[cfg(target_os = "android")]
    Ahb(Box<Ahb>),
}

/// How an imported frame's pixels are bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repr {
    /// A single ordinary plane wrapped as a `wgpu::Texture`.
    Rgb {
        /// The wgpu texture format the plane wraps.
        format: wgpu::TextureFormat,
    },
    /// Known-format multiplanar, bound as integer plane views.
    Planes {
        /// `KIND_EXT_NV12` or `KIND_EXT_P010`, mirrored in the shader.
        kind: u32,
    },
    /// Opaque external-format YCbCr, bound as a combined sampled image
    /// carrying a `VkSamplerYcbcrConversion`.
    ExternalFormat {
        /// The `externalFormat` identifier the image was created with.
        id: u64,
    },
}

/// One imported frame generation.
///
/// `Frame` is `Arc`-backed and `Clone`: cloning it attaches the same
/// generation to several layers, which deduplicates its acquisition and
/// release. Dropping the last clone retires the generation — the producer's
/// release mechanism is signalled by the next engine submission, which may
/// be the dedicated release submission an idle engine still performs.
#[derive(Clone)]
pub struct Frame {
    /// The shared acquisition record — one per imported generation.
    pub generation: Arc<Generation>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("size", &self.generation.size)
            .field("repr", &self.repr())
            .field(
                "state",
                &*self.generation.state.lock().expect("generation state"),
            )
            .finish_non_exhaustive()
    }
}

impl Frame {
    /// The frame's pixel extent.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.generation.size
    }

    /// How the frame's planes are bound.
    #[must_use]
    pub fn repr(&self) -> Repr {
        self.generation.repr
    }

    /// The imported allocation size in bytes, for `Engine::memory`.
    #[must_use]
    pub fn imported_bytes(&self) -> u64 {
        self.generation.bytes
    }

    /// Exports the `FenceFd` release payload once the release submission is
    /// accepted.
    ///
    /// The returned fd becomes signalled when the pending release signal
    /// executes on the GPU; the caller takes ownership of it.
    ///
    /// # Errors
    /// [`NativeError::Unready`] while the release submission has not run,
    /// and [`NativeError::Unsupported`] when the frame has no `FenceFd`
    /// release mechanism.
    pub fn release_fd(&self) -> Result<OwnedFd, NativeError> {
        self.generation.release_fd()
    }

    /// One engine-side retained reference begins (a slot install).
    pub fn lease(&self) {
        self.generation.lease();
    }

    /// One engine-side reference ended; the last one retires the frame.
    pub fn unlease(&self) {
        self.generation.unlease();
    }
}

/// The host-side external-frame import context for Vulkan.
///
/// `Device` resolves to the [`Shared`] context of the `VkDevice` `shared`
/// was created on; it is cheap to clone and `Send + Sync`, so imports run
/// on the producer/host thread while submissions run on the render thread.
/// Imports performed through it must land on the engine's shared device —
/// a frame imported on one `VkDevice` and installed on another's engine is
/// rejected at install.
#[derive(Debug, Clone)]
pub struct Device {
    /// The per-device context imports share.
    pub shared: Arc<Shared>,
}

impl Device {
    /// Probes the Vulkan device behind `shared` for the capabilities native
    /// import needs and resolves the per-device shared context.
    ///
    /// The device must have been created through [`open_device`] (or with an
    /// equivalent extension set) — external-memory, semaphore and YCbCr
    /// capabilities are enabled at device creation, not at first import.
    ///
    /// # Errors
    /// [`NativeError::Unsupported`] when `shared` is not a Vulkan device or
    /// lacks the extensions external-frame import requires.
    ///
    /// # Panics
    /// When the shared-device registry lock or the device handle's `usize`
    /// conversion fails.
    pub fn new(shared: &crate::interop::SharedDevice) -> Result<Self, NativeError> {
        // SAFETY: `shared.device` is a wgpu device; `as_hal` borrows it
        // for the duration of `shared` and reports `None` off-Vulkan.
        let Some(ash_device) = (unsafe { shared.device.as_hal::<wgpu::hal::vulkan::Api>() }) else {
            return Err(NativeError::Unsupported("device is not Vulkan"));
        };
        let hal_device = &*ash_device;
        let raw = hal_device.raw_device().clone();
        let registry_key = usize::try_from(raw.handle().as_raw()).expect("handle fits usize");
        let mut registry = { shared_registry().lock().expect("native registry") };
        let shared_ctx = if let Some(ctx) = registry.get(&registry_key).and_then(Weak::upgrade) {
            ctx
        } else {
            let ctx = Arc::new(Shared::open(shared, hal_device)?);
            registry.insert(registry_key, Arc::downgrade(&ctx));
            ctx
        };
        drop(registry);
        Ok(Self { shared: shared_ctx })
    }

    /// The capability record import was negotiated against.
    #[must_use]
    pub fn caps(&self) -> &Caps {
        &self.shared.caps
    }

    /// Imports `source` as one frame generation.
    ///
    /// The planes are retained in place — nothing copies or converts them —
    /// and the resulting [`Frame`] installs on any layer through
    /// `ExternalFrame::native`. Reusing a producer buffer is a new import:
    /// a new generation with its own synchronization.
    ///
    /// # Errors
    /// [`NativeError`] when the descriptor is incomplete, the capability
    /// record cannot express the contract, or the driver rejects the
    /// format/modifier/usage/handle combination.
    pub fn import(&self, source: FrameSource) -> Result<Frame, NativeError> {
        match source {
            FrameSource::DmaBuf(buf) => dmabuf::import(&self.shared, *buf),
            #[cfg(target_os = "android")]
            FrameSource::Ahb(ahb) => ahb::import(&self.shared, *ahb),
        }
    }
}

/// The extensions and features the engine enabled at device creation for
/// external-frame import, probed honestly from the adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Caps {
    /// `VK_KHR_external_memory` + `VK_KHR_external_memory_fd`.
    pub external_memory_fd: bool,
    /// `VK_EXT_external_memory_dma_buf`.
    pub external_memory_dma_buf: bool,
    /// `VK_KHR_external_semaphore` + `VK_KHR_external_semaphore_fd` with
    /// `OPAQUE_FD` import.
    pub external_semaphore_opaque_fd: bool,
    /// `VK_KHR_external_semaphore_fd` with `SYNC_FD` import/export.
    pub external_semaphore_sync_fd: bool,
    /// `VK_KHR_timeline_semaphore` with the feature enabled.
    pub timeline_semaphore: bool,
    /// `VK_KHR_sampler_ycbcr_conversion` with the feature enabled.
    pub sampler_ycbcr_conversion: bool,
    /// `VK_EXT_image_drm_format_modifier`.
    pub image_drm_format_modifier: bool,
    /// `VK_KHR_external_fence_fd` (sync-fd fences).
    pub external_fence_fd: bool,
    /// `VK_KHR_dedicated_allocation`.
    pub dedicated_allocation: bool,
    /// `VK_ANDROID_external_memory_android_hardware_buffer`.
    #[cfg(target_os = "android")]
    pub external_memory_android_hardware_buffer: bool,
    /// A queue family other than the engine's may own imported images
    /// (`VK_QUEUE_FAMILY_EXTERNAL` / `VK_QUEUE_FAMILY_FOREIGN_EXT`
    /// transfers).
    pub queue_family_foreign: bool,
}

/// The extension names the engine adds to `vkCreateDevice` when supported.
///
/// `wgpu-hal` already enables the FD external-memory, dma-buf modifier,
/// DRM modifier and timeline-semaphore extensions when the physical device
/// offers them; the callback adds the YCbCr conversion and
/// semaphore-fd extensions and — on Android — the hardware-buffer import
/// extension, plus the feature bit wgpu leaves off. The design's release
/// fences are semaphores exported as `SYNC_FD`, so no
/// `VK_KHR_external_fence_fd` device extension is needed.
pub fn extra_device_extensions() -> Vec<&'static CStr> {
    #[allow(unused_mut)]
    let mut exts = vec![
        ash::khr::sampler_ycbcr_conversion::NAME,
        ash::khr::external_semaphore_fd::NAME,
    ];
    #[cfg(target_os = "android")]
    exts.push(ash::android::external_memory_android_hardware_buffer::NAME);
    exts
}

/// Why a native import or frame operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NativeError {
    /// The capability record cannot express the requested contract.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// The descriptor is incomplete or self-inconsistent.
    #[error("invalid: {0}")]
    Invalid(&'static str),
    /// A Vulkan call failed; the payload is the `VkResult` code.
    #[error("vulkan: {0}")]
    Vulkan(i32),
    /// The release payload is not available yet.
    #[error("the release submission has not completed")]
    Unready,
}

impl From<vk::Result> for NativeError {
    fn from(err: vk::Result) -> Self {
        Self::Vulkan(err.as_raw())
    }
}

/// A `Vk` is the shared raw context: the `ash` device, the queue and family
/// the engine submits on, and the per-device retire queue every generation
/// pushes into when its last retained owner drops.
#[derive(Clone)]
pub struct Vk {
    pub device: ash::Device,
    /// `VK_KHR_sampler_ycbcr_conversion` entry points, when enabled.
    pub ycbcr: Option<ash::khr::sampler_ycbcr_conversion::Device>,
    /// `VK_KHR_external_semaphore_fd` entry points, when enabled.
    pub external_semaphore_fd: Option<ash::khr::external_semaphore_fd::Device>,
    /// `VK_ANDROID_external_memory_android_hardware_buffer` entry points.
    #[cfg(target_os = "android")]
    pub ahb: Option<ash::android::external_memory_android_hardware_buffer::Device>,
    /// The engine's queue family — the destination of acquire barriers and
    /// the source of release barriers.
    pub queue_family: u32,
    /// The retire queue render-thread flushes drain.
    pub pending_release: Arc<Mutex<Vec<Release>>>,
    /// Cache objects evicted by bound enforcement, destroyed on the next
    /// submission-completion callback.
    pub pending_destroy: Arc<Mutex<Vec<sync::DestroyItem>>>,
}

impl std::fmt::Debug for Vk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vk")
            .field("queue_family", &self.queue_family)
            .field("ycbcr", &self.ycbcr.is_some())
            .field(
                "external_semaphore_fd",
                &self.external_semaphore_fd.is_some(),
            )
            .finish_non_exhaustive()
    }
}

/// The per-`VkDevice` context shared between host-side imports and the
/// render thread.
pub struct Shared {
    pub vk: Vk,
    /// The `wgpu::Device` the raw device wraps — RGB imports wrap planes
    /// as `wgpu::Texture`s on it and it keeps the `VkDevice` alive.
    pub wgpu: wgpu::Device,
    pub caps: Caps,
    /// `vkGetPhysicalDeviceFormatProperties`/`vkGetPhysicalDeviceImageFormatProperties2`
    /// and friends run on the physical device behind this instance.
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    /// Conversion/layout/pipeline objects cached by their complete
    /// conversion key — bounded, see `ycbcr`.
    pub convs: Mutex<FxHashMap<ycbcr::ConvKey, Weak<ycbcr::Conv>>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("caps", &self.caps)
            .field("physical_device", &self.physical_device)
            .finish_non_exhaustive()
    }
}

/// Resolves (or probes) the [`Shared`] context behind `shared`'s
/// `VkDevice` — the render-thread counterpart of [`Device::new`].
pub fn shared_for(shared: &crate::interop::SharedDevice) -> Result<Arc<Shared>, NativeError> {
    Ok(Device::new(shared)?.shared)
}

impl Shared {
    /// Probes `hal_device`'s adapter and builds the shared context.
    fn open(
        shared: &crate::interop::SharedDevice,
        hal_device: &wgpu::hal::vulkan::Device,
    ) -> Result<Self, NativeError> {
        // SAFETY: `shared.adapter` is the adapter `shared.device` was
        // opened on; `as_hal` borrows it for the duration of `shared`.
        let Some(hal_adapter) = (unsafe { shared.adapter.as_hal::<wgpu::hal::vulkan::Api>() })
        else {
            return Err(NativeError::Unsupported("adapter is not Vulkan"));
        };
        let hal_adapter = &*hal_adapter;
        let instance = hal_adapter.shared_instance().raw_instance().clone();
        let physical_device = hal_adapter.raw_physical_device();
        let device = hal_device.raw_device().clone();
        let queue_family = hal_device.queue_family_index();
        let caps = probe(hal_device, &instance, physical_device);
        let vk = Vk {
            ycbcr: caps
                .sampler_ycbcr_conversion
                .then(|| ash::khr::sampler_ycbcr_conversion::Device::new(&instance, &device)),
            external_semaphore_fd: (caps.external_semaphore_opaque_fd
                || caps.external_semaphore_sync_fd)
                .then(|| ash::khr::external_semaphore_fd::Device::new(&instance, &device)),
            #[cfg(target_os = "android")]
            ahb: caps.external_memory_android_hardware_buffer.then(|| {
                ash::android::external_memory_android_hardware_buffer::Device::new(
                    &instance, &device,
                )
            }),
            queue_family,
            pending_release: Arc::new(Mutex::new(Vec::new())),
            pending_destroy: Arc::new(Mutex::new(Vec::new())),
            device,
        };
        Ok(Self {
            vk,
            wgpu: shared.device.clone(),
            caps,
            instance,
            physical_device,
            convs: Mutex::new(FxHashMap::default()),
        })
    }

    /// Queues `release` for the next flush — the renderer submits it after
    /// the last recorded read, retaining every object until the submission
    /// completes.
    pub fn retire(&self, release: Release) {
        self.vk
            .pending_release
            .lock()
            .expect("pending release")
            .push(release);
    }
}

/// Probes `device`'s enabled extension set plus the physical device's
/// feature chain for the capabilities external-frame import relies on.
///
/// Called once per `VkDevice` — device creation is where extensions and
/// features become available, so this is the point the capability record
/// is honest about.
fn probe(
    device: &wgpu::hal::vulkan::Device,
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Caps {
    let enabled = device.enabled_device_extensions();
    let has = |name: &CStr| enabled.contains(&name);

    let mut features2 = vk::PhysicalDeviceFeatures2::default();
    let mut timeline = vk::PhysicalDeviceTimelineSemaphoreFeatures::default();
    let mut ycbcr = vk::PhysicalDeviceSamplerYcbcrConversionFeatures::default();
    let mut buf_addr = vk::PhysicalDeviceVulkan12Features::default();
    features2 = features2
        .push_next(&mut timeline)
        .push_next(&mut ycbcr)
        .push_next(&mut buf_addr);
    // SAFETY: `instance`/`physical_device` are the engine's own handles
    // and `features2` points at the live chain of out structs above.
    unsafe {
        instance.get_physical_device_features2(physical_device, &mut features2);
    }

    // Semaphore handle-type support comes from the external-semaphore
    // capability query; OPAQUE_FD needs import+export for acquire payloads,
    // SYNC_FD for both wait and release-fence directions.
    let (mut opaque_fd, mut sync_fd) = (false, false);
    if has(ash::khr::external_semaphore_fd::NAME) {
        let semaphore_info = vk::PhysicalDeviceExternalSemaphoreInfo::default()
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
        let mut out = vk::ExternalSemaphoreProperties::default();
        // SAFETY: `instance`/`physical_device` are the engine's own
        // handles and `out` is a live out struct for this query.
        unsafe {
            instance.get_physical_device_external_semaphore_properties(
                physical_device,
                &semaphore_info,
                &mut out,
            );
        }
        let props = out;
        opaque_fd = props
            .external_semaphore_features
            .contains(vk::ExternalSemaphoreFeatureFlags::IMPORTABLE);
        let semaphore_info =
            semaphore_info.handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let mut out = vk::ExternalSemaphoreProperties::default();
        // SAFETY: `instance`/`physical_device` are the engine's own
        // handles and `out` is a live out struct for this query.
        unsafe {
            instance.get_physical_device_external_semaphore_properties(
                physical_device,
                &semaphore_info,
                &mut out,
            );
        }
        sync_fd = out
            .external_semaphore_features
            .contains(vk::ExternalSemaphoreFeatureFlags::IMPORTABLE)
            && out
                .external_semaphore_features
                .contains(vk::ExternalSemaphoreFeatureFlags::EXPORTABLE);
    }

    Caps {
        external_memory_fd: has(ash::khr::external_memory_fd::NAME),
        external_memory_dma_buf: has(ash::ext::external_memory_dma_buf::NAME),
        external_semaphore_opaque_fd: opaque_fd,
        external_semaphore_sync_fd: sync_fd,
        timeline_semaphore: timeline.timeline_semaphore == vk::TRUE,
        sampler_ycbcr_conversion: ycbcr.sampler_ycbcr_conversion == vk::TRUE,
        image_drm_format_modifier: has(ash::ext::image_drm_format_modifier::NAME),
        external_fence_fd: has(ash::khr::external_fence_fd::NAME),
        dedicated_allocation: has(ash::khr::dedicated_allocation::NAME)
            || device.shared_instance().instance_api_version() >= vk::API_VERSION_1_1,
        #[cfg(target_os = "android")]
        external_memory_android_hardware_buffer: has(
            ash::android::external_memory_android_hardware_buffer::NAME,
        ),
        // `VK_QUEUE_FAMILY_FOREIGN_EXT` is defined by the platform's
        // AHardwareBuffer contract itself — usable on Android without the
        // extension being requested.
        queue_family_foreign: has(ash::ext::queue_family_foreign::NAME)
            || cfg!(target_os = "android"),
    }
}

// The static registry: `Weak` upgrade keeps a `Shared` alive exactly as long
// as any generation, `Device` or renderer-side `Native` references it.
fn shared_registry() -> &'static Mutex<FxHashMap<usize, Weak<Shared>>> {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<Mutex<FxHashMap<usize, Weak<Shared>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(FxHashMap::default()))
}

// `vk.rs` helper on the hal `Device` name is `shared_instance()` — the
// probe above uses it for the API-version promotion check.

/// Per-renderer native context: the shared device context plus every object
/// the encode path caches — shader module, set-0/set-1 layouts, render
/// passes, pipelines and framebuffers.
#[derive(Debug)]
pub struct Native {
    /// The per-device context.
    pub shared: Arc<Shared>,
    /// `external_native.spv`, loaded as a raw `VkShaderModule` exposing
    /// `vs_main`, `fs_external` and `fs_external_format`.
    pub module: vk::ShaderModule,
    /// Group-0 mirror of `ENGINE_GROUP0` for the native pipeline layout.
    pub set0: vk::DescriptorSetLayout,
    /// Group-1 layout for integer plane views (bindings 0–4, plain sampled
    /// images + uniform).
    pub set1_uv: vk::DescriptorSetLayout,
    /// Pipeline layouts per set-1 kind.
    pub pipe_uv: vk::PipelineLayout,
    /// Pipeline layouts for external-format conversions, keyed by the
    /// conversion's set-1 layout handle.
    pub conv_layouts: FxHashMap<vk::DescriptorSetLayout, vk::PipelineLayout>,
    /// The pool the renderer's single `set0` descriptor set allocates from.
    pub desc_pool0: vk::DescriptorPool,
    /// The renderer's single `set0` descriptor set, allocated once and
    /// rewritten in place when the buffer/view identities change.
    pub set0_set: vk::DescriptorSet,
    /// Buffer/view handles `set0_set` was last written for.
    pub set0_key: Option<(u64, u64, u64, u64)>,
    /// Dummy `texture_2d<f32>` sampled view for bindings the active shader
    /// path declares but does not use.
    /// The 1x1 `R8Unorm` view unused sampled bindings point at.
    pub dummy_f32: vk::ImageView,
    /// The image the dummy view points at.
    pub dummy_image: vk::Image,
    /// The dummy image's memory.
    pub dummy_memory: vk::DeviceMemory,
    /// Render passes keyed by target format.
    pub render_passes: FxHashMap<vk::Format, vk::RenderPass>,
    /// Framebuffers keyed by (render pass, target view, extent, target
    /// generation) — the generation invalidates entries when the surface
    /// re-creates the view a recycled handle could alias.
    pub framebuffers: FxHashMap<(u64, u64, u32, u32, u64), vk::Framebuffer>,
    /// Pipelines keyed by (layout kind, target format).
    pub pipelines: FxHashMap<PipeKey, vk::Pipeline>,
    /// Generations staged for acquisition in the encoder currently being
    /// built; the submit registers their waits and finalises states.
    pub staged: Vec<sync::PendingAcquire>,
    /// State records of generations whose consuming submission was
    /// accepted; the completion callback promotes them to `OwnedForRead`.
    pub acquiring: Vec<Arc<std::sync::Mutex<sync::State>>>,
    /// Releases ready to submit on this submission.
    pub releases: Vec<Release>,
}

/// Pipeline cache key: which set-1 layout kind and which target format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipeKey {
    /// Integer plane views (`fs_external`).
    Planes(vk::Format),
    /// External-format combined sampler (`fs_external_format`), keyed by
    /// the conversion's set-1 layout.
    ExternalFormat(vk::DescriptorSetLayout, vk::Format),
}

impl Native {
    /// Builds the render-side context for `shared`, or reports unsupported.
    ///
    /// The module, layouts and the dummy view are created once; everything
    /// else is lazily keyed caches. On failure any partially created objects
    /// are destroyed before returning.
    ///
    /// # Errors
    /// [`NativeError`] when any Vulkan object fails creation.
    ///
    /// # Panics
    /// When the build-time `external_native.spv` is malformed.
    #[expect(
        clippy::too_many_lines,
        reason = "pipeline layout, dummy objects and render state share one"
    )]
    pub fn new(shared: Arc<Shared>) -> Result<Self, NativeError> {
        let dev = &shared.vk.device;
        let code = {
            let mut cursor = std::io::Cursor::new(crate::render::shaders::spirv::EXTERNAL_NATIVE);
            ash::util::read_spv(&mut cursor).expect("external_native.spv")
        };
        // SAFETY: `dev` is live and `code` is SPIR-V read from the
        // compiled engine binary — valid bytes by construction.
        let module = unsafe {
            dev.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
        }
        .map_err(NativeError::from)?;

        let make_layout = |bindings: &[vk::DescriptorSetLayoutBinding],
                           flags: &[vk::DescriptorBindingFlags]| {
            let mut flags_info =
                vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(flags);
            let info = vk::DescriptorSetLayoutCreateInfo::default()
                .bindings(bindings)
                .push_next(&mut flags_info);
            // SAFETY: `dev` is live and `info` points at the caller's
            // bindings and flags slices, both valid for the call.
            unsafe { dev.create_descriptor_set_layout(&info, None) }
        };
        let no_flags = |n: usize| vec![vk::DescriptorBindingFlags::empty(); n];
        let sampler = |stage| {
            vk::DescriptorSetLayoutBinding::default()
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(stage)
        };
        let fs = vk::ShaderStageFlags::FRAGMENT;
        let vf = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;

        // set0 mirrors `ENGINE_GROUP0`: a dynamic Globals uniform,
        // the instances and stops storage buffers, and the atlas texture.
        let set0 = make_layout(
            &[
                vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .descriptor_count(1)
                    .stage_flags(vf),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vf),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(2)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(fs),
                sampler(fs).binding(3),
            ],
            &no_flags(4),
        );
        // set1 mirrors `EXTERNAL_GROUP1`: luma, chroma, rgb and mask sampled
        // images plus the params uniform.
        let set1_uv = make_layout(
            &[
                sampler(fs).binding(0),
                sampler(fs).binding(1),
                sampler(fs).binding(2),
                sampler(fs).binding(3),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(4)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(fs),
            ],
            &no_flags(5),
        );
        let (Ok(set0), Ok(set1_uv)) = (set0, set1_uv) else {
            // SAFETY: each destroyed object was created above on `dev` and
            // is torn down exactly once on this failure path.
            unsafe {
                if let Ok(l) = set0 {
                    dev.destroy_descriptor_set_layout(l, None);
                }
                if let Ok(l) = set1_uv {
                    dev.destroy_descriptor_set_layout(l, None);
                }
                dev.destroy_shader_module(module, None);
            }
            return Err(NativeError::Unsupported("descriptor set layouts"));
        };
        // SAFETY: `dev` is live and `set0`/`set1_uv` are live layouts
        // created above.
        let pipe_uv = unsafe {
            dev.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&[set0, set1_uv]),
                None,
            )
        }
        .map_err(NativeError::from)?;

        // The pool the renderer's single set-0 set allocates from; the
        // set is rewritten (never reallocated) when buffer identities
        // change, so the sizes cover exactly one set.
        // SAFETY: `dev` is live and the pool sizes are compile-time
        // constants matching set-0's layout.
        let desc_pool0 = unsafe {
            dev.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&[
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
                            descriptor_count: 1,
                        },
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::STORAGE_BUFFER,
                            descriptor_count: 2,
                        },
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::SAMPLED_IMAGE,
                            descriptor_count: 1,
                        },
                    ]),
                None,
            )
        }
        .map_err(NativeError::from)?;
        // A 1x1 R8Unorm the unused sampled bindings point at.
        // SAFETY: `dev` is live; `create_dummy` upholds its contract by
        // using only `dev` handles it owns.
        let (dummy_image, dummy_memory, dummy_f32) = unsafe { create_dummy(dev)? };
        Ok(Self {
            shared,
            module,
            set0,
            set1_uv,
            pipe_uv,
            conv_layouts: FxHashMap::default(),
            desc_pool0,
            set0_set: vk::DescriptorSet::null(),
            set0_key: None,
            dummy_f32,
            dummy_image,
            dummy_memory,
            render_passes: FxHashMap::default(),
            framebuffers: FxHashMap::default(),
            pipelines: FxHashMap::default(),
            staged: Vec::new(),
            acquiring: Vec::new(),
            releases: Vec::new(),
        })
    }

    /// Destroys every cached object; called at renderer teardown.
    ///
    /// # Panics
    /// On a poisoned deferred-destroy queue.
    pub fn destroy(&mut self) {
        let dev = &self.shared.vk.device;
        // SAFETY: every object destroyed here was created on `dev` by this
        // `Native`, is drained (never destroyed twice), and the deferred
        // queue items' `destroy` contracts are met by their own producers.
        unsafe {
            for (_, pipeline) in self.pipelines.drain() {
                dev.destroy_pipeline(pipeline, None);
            }
            for (_, fb) in self.framebuffers.drain() {
                dev.destroy_framebuffer(fb, None);
            }
            for item in self
                .shared
                .vk
                .pending_destroy
                .lock()
                .expect("pending destroy")
                .drain(..)
            {
                item.destroy(dev);
            }
            for (_, rp) in self.render_passes.drain() {
                dev.destroy_render_pass(rp, None);
            }
            dev.destroy_image_view(self.dummy_f32, None);
            dev.destroy_image(self.dummy_image, None);
            dev.free_memory(self.dummy_memory, None);
            for (_, layout) in self.conv_layouts.drain() {
                dev.destroy_pipeline_layout(layout, None);
            }
            dev.destroy_descriptor_pool(self.desc_pool0, None);
            dev.destroy_pipeline_layout(self.pipe_uv, None);
            dev.destroy_descriptor_set_layout(self.set1_uv, None);
            dev.destroy_descriptor_set_layout(self.set0, None);
            dev.destroy_shader_module(self.module, None);
        }
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Creates the 1x1 `R8Unorm` image+view unused sampled bindings point at.
unsafe fn create_dummy(
    dev: &ash::Device,
) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), NativeError> {
    // SAFETY: `dev` is live and the create info describes a plain 1x1
    // image — nothing external is referenced.
    let image = unsafe {
        dev.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::R8_UNORM)
                .extent(vk::Extent3D {
                    width: 1,
                    height: 1,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::SAMPLED)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED),
            None,
        )
    }
    .map_err(NativeError::from)?;
    // SAFETY: `image` was created above on `dev` — a pure query.
    let reqs = unsafe { dev.get_image_memory_requirements(image) };
    // Any device-local-capable type works for a never-read dummy.
    // SAFETY: `reqs` was just queried for `image` and a supported type
    // index is selected by `trailing_zeros` of its own bits.
    let memory = unsafe {
        dev.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(reqs.memory_type_bits.trailing_zeros()),
            None,
        )
    }
    .map_err(NativeError::from)?;
    // SAFETY: `image` and `memory` were created above on `dev`; offset 0
    // binds the whole requirement.
    unsafe { dev.bind_image_memory(image, memory, 0) }.map_err(NativeError::from)?;
    // SAFETY: `image` is live on `dev` — the view covers its only mip.
    let view = unsafe {
        dev.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(vk::Format::R8_UNORM)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                }),
            None,
        )
    }
    .map_err(NativeError::from)?;
    Ok((image, memory, view))
}

impl FrameSource {
    /// The frame's colour metadata, whichever source variant carries it.
    #[allow(dead_code)]
    #[must_use]
    pub fn color(&self) -> FrameColor {
        match self {
            Self::DmaBuf(buf) => buf.color,
            #[cfg(target_os = "android")]
            Self::Ahb(ahb) => ahb.color,
        }
    }

    /// The frame's alpha contract, whichever source variant carries it.
    #[allow(dead_code)]
    #[must_use]
    pub fn alpha(&self) -> RgbAlpha {
        match self {
            Self::DmaBuf(buf) => buf.alpha,
            #[cfg(target_os = "android")]
            Self::Ahb(ahb) => ahb.alpha,
        }
    }
}

/// Maps a frame's declared `FrameColor` to the conversion model the
/// `VkSamplerYcbcrConversion` must implement — the value the sampler's
/// suggestion is validated against rather than silently replaced with.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub const fn required_model(color: &FrameColor) -> vk::SamplerYcbcrModelConversion {
    match color.matrix {
        crate::interop::YuvMatrix::Bt601 => vk::SamplerYcbcrModelConversion::YCBCR_601,
        crate::interop::YuvMatrix::Bt709 => vk::SamplerYcbcrModelConversion::YCBCR_709,
        crate::interop::YuvMatrix::Bt2020 => vk::SamplerYcbcrModelConversion::YCBCR_2020,
    }
}

/// `NARROW` for studio-range frames, `FULL` for full range.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub const fn required_range(color: &FrameColor) -> vk::SamplerYcbcrRange {
    match color.range {
        crate::interop::YuvRange::Video => vk::SamplerYcbcrRange::ITU_NARROW,
        crate::interop::YuvRange::Full => vk::SamplerYcbcrRange::ITU_FULL,
    }
}

/// One axis of the frame's chroma siting as a `VkChromaLocation`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub const fn chroma_location(offset: ChromaOffset) -> vk::ChromaLocation {
    match offset {
        ChromaOffset::Cosited => vk::ChromaLocation::COSITED_EVEN,
        ChromaOffset::Centered => vk::ChromaLocation::MIDPOINT,
    }
}

// The submit path hands `Release`s to `queue.on_submitted_work_done`,
// which requires `Send`: vk handles are plain integers and the lease's
// raw AHardwareBuffer pointer only travels to the queue-completion thread.
// SAFETY: `Lease` carries plain vk handles (integers) and an NDK pointer
// that is only read on the queue-completion thread — no aliasing or
// thread-affine state.
unsafe impl Send for sync::Lease {}
// SAFETY: `Release` is a bag of vk handles (plain integers) plus the
// lease above — movable to the completion thread safely.
unsafe impl Send for sync::Release {}
#[cfg(target_os = "android")]
// SAFETY: an `AHardwareBuffer` is reference-counted and its NDK entry points
// are callable from any thread; a plane source only reads the handle.
unsafe impl Send for sync::PlaneSource {}
#[cfg(target_os = "android")]
// SAFETY: as above.
unsafe impl Sync for sync::PlaneSource {}
// SAFETY: every field of `Generation` is behind its own `Mutex`/`Arc` or
// is plain data; the raw vk handles it carries only travel between the
// render thread and the queue-completion thread under the state machine
// `sync` documents — no unsynchronized shared access.
unsafe impl Send for sync::Generation {}
// SAFETY: as for `Send`: shared access only reaches mutex-guarded or
// immutable state.
unsafe impl Sync for sync::Generation {}

/// One draw inside a native composition op.
#[derive(Debug)]
pub struct OpDraw {
    /// The generation being sampled.
    pub generation: Arc<Generation>,
    /// First instance index (`inst_base + range.instances.start`).
    pub first_instance: u32,
    /// Instances drawn.
    pub instance_count: u32,
    /// The clip-mask view (`None` binds the dummy).
    pub mask: Option<vk::ImageView>,
    /// The atlas mask-texture generation at encode time — part of the
    /// set-1 cache key so a re-created mask texture rebinds.
    pub mask_gen: u64,
    /// The frame's params uniform buffer.
    pub params: vk::Buffer,
}

/// Writes a `set-1` descriptor for `generation` under `mask`, allocating from the
/// generation's pool. The cache key is `(mask view, mask generation)` so a
/// re-created mask texture — whose raw handle a driver may recycle —
/// misses and rebinds; the map is bounded at the pool's `max_sets`, with
/// evicted sets freed once the referencing submissions complete.
#[expect(
    clippy::too_many_lines,
    reason = "the bounded cache check, allocate and write are one object"
)]
pub fn write_set1(
    native: &Native,
    generation: &Arc<Generation>,
    mask: Option<vk::ImageView>,
    params: vk::Buffer,
    mask_key: (u64, u64),
) -> Result<vk::DescriptorSet, NativeError> {
    if let Some(&set) = generation.sets.lock().expect("frame sets").get(&mask_key) {
        return Ok(set);
    }
    let Some(pool) = generation.pool else {
        return Err(NativeError::Invalid("frame has no descriptor pool"));
    };
    let dev = &native.shared.vk.device;
    let (layout, writes): (vk::DescriptorSetLayout, Vec<(u32, vk::DescriptorImageInfo)>) =
        match generation.repr {
            Repr::Planes { .. } => {
                let sync::Views::Planes { y, uv } = generation.views else {
                    return Err(NativeError::Invalid("plane views missing"));
                };
                let sub = |view| vk::DescriptorImageInfo {
                    sampler: vk::Sampler::null(),
                    image_view: view,
                    image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                };
                (
                    native.set1_uv,
                    vec![
                        (0, sub(y)),
                        (1, sub(uv)),
                        // `ext_rgb` is statically used by `fs_external` —
                        // bind the float dummy.
                        (2, sub(native.dummy_f32)),
                        (3, sub(mask.unwrap_or(native.dummy_f32))),
                    ],
                )
            }
            Repr::ExternalFormat { .. } => {
                let (sync::Views::ExternalFormat { view }, Some(conv)) =
                    (generation.views, generation.conv.as_ref())
                else {
                    return Err(NativeError::Invalid("conversion views missing"));
                };
                let sub = |view| vk::DescriptorImageInfo {
                    sampler: vk::Sampler::null(),
                    image_view: view,
                    image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                };
                (
                    conv.set1,
                    vec![
                        (3, sub(mask.unwrap_or(native.dummy_f32))),
                        // The combined sampled image: image view, layout and
                        // the immutable sampler carried by the layout.
                        (
                            5,
                            vk::DescriptorImageInfo {
                                sampler: conv.sampler,
                                image_view: view,
                                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                            },
                        ),
                    ],
                )
            }
            Repr::Rgb { .. } => return Err(NativeError::Invalid("rgb frames bind on wgpu")),
        };
    let layouts = [layout];
    let alloc = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);
    // Bounded like the conversion cache: a generation's pool is
    // `max_sets(16)`, so the whole map is retired into the
    // deferred-free queue once the cap is reached — the evicted sets may
    // still be referenced by in-flight submissions and are freed on the
    // next completion callback, before the pool itself is destroyed at
    // release completion.
    if generation.sets.lock().expect("frame sets").len() >= 16 {
        let mut pending = native
            .shared
            .vk
            .pending_destroy
            .lock()
            .expect("pending destroy");
        for (_, evicted) in generation.sets.lock().expect("frame sets").drain() {
            pending.push(sync::DestroyItem::DescriptorSet { pool, set: evicted });
        }
    }
    // SAFETY: `alloc` references the live pool and set layouts built
    // above on `dev`.
    let sets = unsafe { dev.allocate_descriptor_sets(&alloc) }.map_err(NativeError::from)?;
    let set = sets[0];
    let image_writes: Vec<vk::WriteDescriptorSet> = writes
        .iter()
        .map(|(binding, image)| {
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(*binding)
                .descriptor_type(if layout == native.set1_uv {
                    vk::DescriptorType::SAMPLED_IMAGE
                } else if *binding == 5 {
                    vk::DescriptorType::COMBINED_IMAGE_SAMPLER
                } else {
                    vk::DescriptorType::SAMPLED_IMAGE
                })
                .image_info(std::slice::from_ref(image))
        })
        .collect();
    let params_info = vk::DescriptorBufferInfo {
        buffer: params,
        offset: 0,
        range: vk::WHOLE_SIZE,
    };
    let params_write = vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(4)
        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
        .buffer_info(std::slice::from_ref(&params_info));
    let mut all = image_writes;
    all.push(params_write);
    // SAFETY: `set` was allocated from `native`'s pool; the writes
    // reference live views, buffers and layouts matching the set's
    // layout bindings by construction of `writes`/`params_info`.
    unsafe { dev.update_descriptor_sets(&all, &[]) };
    generation
        .sets
        .lock()
        .expect("frame sets")
        .insert(mask_key, set);
    Ok(set)
}

impl Native {
    /// The pipeline layout a generation's draws bind.
    ///
    /// # Panics
    /// When an external-format generation lacks its conversion.
    pub fn pipeline_layout(&mut self, generation: &Generation) -> vk::PipelineLayout {
        match generation.repr {
            Repr::Planes { .. } => self.pipe_uv,
            Repr::ExternalFormat { .. } => {
                let conv = generation.conv.as_ref().expect("conv");
                let dev = &self.shared.vk.device;
                *self.conv_layouts.entry(conv.set1).or_insert_with(|| {
                    // SAFETY: `dev` is live and `self.set0`/`conv.set1`
                    // are live descriptor set layouts on it.
                    unsafe {
                        dev.create_pipeline_layout(
                            &vk::PipelineLayoutCreateInfo::default()
                                .set_layouts(&[self.set0, conv.set1]),
                            None,
                        )
                    }
                    .expect("pipeline layout")
                })
            }
            Repr::Rgb { .. } => vk::PipelineLayout::null(),
        }
    }

    /// The render pass for `format` — a single colour attachment loaded and
    /// stored, in `COLOR_ATTACHMENT_OPTIMAL` throughout.
    fn render_pass(&mut self, format: vk::Format) -> Result<vk::RenderPass, NativeError> {
        if let Some(&rp) = self.render_passes.get(&format) {
            return Ok(rp);
        }
        let attachment = vk::AttachmentDescription::default()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let color_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_ref));
        let info = vk::RenderPassCreateInfo::default()
            .attachments(std::slice::from_ref(&attachment))
            .subpasses(std::slice::from_ref(&subpass));
        // SAFETY: `self.shared.vk.device` is live and `info` points at
        // the attachment/subpass structs built above.
        let rp = unsafe { self.shared.vk.device.create_render_pass(&info, None) }
            .map_err(NativeError::from)?;
        self.render_passes.insert(format, rp);
        Ok(rp)
    }

    /// The framebuffer for (`view`, `extent`) on the `format` render
    /// pass. `view_gen` is the surface's bind generation — a re-created
    /// target view (whose raw handle a driver may recycle) keys a fresh
    /// entry instead of hitting a stale one.
    fn framebuffer(
        &mut self,
        format: vk::Format,
        view: vk::ImageView,
        extent: (u32, u32),
        view_gen: u64,
    ) -> Result<vk::Framebuffer, NativeError> {
        let rp = self.render_pass(format)?;
        let key = (rp.as_raw(), view.as_raw(), extent.0, extent.1, view_gen);
        if let Some(&fb) = self.framebuffers.get(&key) {
            return Ok(fb);
        }
        // SAFETY: `rp` is a live render pass this `Native` created and
        // `view`/`extent` come from the caller's live target.
        let fb = unsafe {
            self.shared.vk.device.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(rp)
                    .attachments(std::slice::from_ref(&view))
                    .width(extent.0)
                    .height(extent.1)
                    .layers(1),
                None,
            )
        }
        .map_err(NativeError::from)?;
        // Bounded like the conversion cache: past the cap, every entry is
        // retired into the deferred-destroy queue — the objects may still
        // be referenced by in-flight submissions, so destruction lands on
        // the next completion callback.
        if self.framebuffers.len() >= 32 {
            let mut pending = self
                .shared
                .vk
                .pending_destroy
                .lock()
                .expect("pending destroy");
            for (_, evicted) in self.framebuffers.drain() {
                pending.push(sync::DestroyItem::Framebuffer(evicted));
            }
        }
        self.framebuffers.insert(key, fb);
        Ok(fb)
    }

    /// The pipeline for `key`, created on demand.
    fn pipeline(&mut self, key: PipeKey) -> Result<vk::Pipeline, NativeError> {
        if let Some(&pipeline) = self.pipelines.get(&key) {
            return Ok(pipeline);
        }
        let (layout, entry) = match key {
            PipeKey::Planes(_) => (self.pipe_uv, c"fs_external"),
            PipeKey::ExternalFormat(set1, _) => {
                let dev = &self.shared.vk.device;
                let layout = match self.conv_layouts.entry(set1) {
                    Entry::Occupied(e) => *e.get(),
                    Entry::Vacant(e) => *e.insert(
                        // SAFETY: `dev` is live and `self.set0`/`set1`
                        // are live descriptor set layouts on it.
                        unsafe {
                            dev.create_pipeline_layout(
                                &vk::PipelineLayoutCreateInfo::default()
                                    .set_layouts(&[self.set0, set1]),
                                None,
                            )
                        }
                        .map_err(NativeError::from)?,
                    ),
                };
                (layout, c"fs_external_format")
            }
        };
        let format = match key {
            PipeKey::Planes(f) | PipeKey::ExternalFormat(_, f) => f,
        };
        let render_pass = self.render_pass(format)?;
        let vs = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(self.module)
            .name(c"vs_main");
        let fs = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(self.module)
            .name(entry);
        let vertex = vk::PipelineVertexInputStateCreateInfo::default();
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewports = [vk::Viewport::default()];
        let scissors = [vk::Rect2D::default()];
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let msaa = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let component = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let blend = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(std::slice::from_ref(&component));
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_info = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
        let stages = [vs, fs];
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&msaa)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic_info)
            .layout(layout)
            .render_pass(render_pass)
            .subpass(0);
        // SAFETY: `dev` is live; the create info references only the
        // module, layout and render pass this `Native` owns.
        let pipeline = unsafe {
            self.shared.vk.device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&info),
                None,
            )
        }
        .map_err(|_| NativeError::Vulkan(vk::Result::ERROR_INITIALIZATION_FAILED.as_raw()))?;
        let pipeline = pipeline[0];
        self.pipelines.insert(key, pipeline);
        Ok(pipeline)
    }

    /// The `set0` descriptor set for the engine buffers, rebuilt when any
    /// buffer or the atlas view changes identity.
    ///
    /// # Errors
    /// [`NativeError`] when allocation or the writes fail.
    ///
    /// # Panics
    /// On a poisoned descriptor-pool mutex.
    pub fn set0_set(
        &mut self,
        globals: vk::Buffer,
        instances: vk::Buffer,
        stops: vk::Buffer,
        atlas: vk::ImageView,
    ) -> Result<vk::DescriptorSet, NativeError> {
        let key = (
            globals.as_raw(),
            instances.as_raw(),
            stops.as_raw(),
            atlas.as_raw(),
        );
        if self.set0_key == Some(key) {
            return Ok(self.set0_set);
        }
        let dev = &self.shared.vk.device;
        if self.set0_set.is_null() {
            // One-time allocation: the pool is `max_sets(1)` because this
            // set is rewritten in place, never reallocated.
            // SAFETY: `self.desc_pool0` is a live pool created for
            // exactly this allocation and `self.set0` is a live layout.
            let sets = unsafe {
                dev.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(self.desc_pool0)
                        .set_layouts(std::slice::from_ref(&self.set0)),
                )
            }
            .map_err(NativeError::from)?;
            self.set0_set = sets[0];
        }
        let set = self.set0_set;
        let infos = [
            vk::DescriptorBufferInfo {
                buffer: globals,
                offset: 0,
                range: std::mem::size_of::<crate::render::instance::Globals>() as u64,
            },
            vk::DescriptorBufferInfo {
                buffer: instances,
                offset: 0,
                range: vk::WHOLE_SIZE,
            },
            vk::DescriptorBufferInfo {
                buffer: stops,
                offset: 0,
                range: vk::WHOLE_SIZE,
            },
        ];
        let image = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: atlas,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        // SAFETY: `set` is this `Native`'s set-0 set; the writes match
        // its layout bindings and reference live buffers/view passed in.
        unsafe {
            dev.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                        .buffer_info(std::slice::from_ref(&infos[0])),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(std::slice::from_ref(&infos[1])),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(2)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(std::slice::from_ref(&infos[2])),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(3)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&image),
                ],
                &[],
            );
        }
        self.set0_key = Some(key);
        Ok(set)
    }

    /// Records the native composition op: acquire barriers for first-use
    /// generations, then a render pass drawing every staged frame.
    /// `set0` is the descriptor set `set0_set` returned for the current
    /// engine buffers.
    ///
    /// # Errors
    /// [`NativeError`] when a draw's set or pipeline cannot be produced.
    ///
    /// # Safety
    /// `cb` must be a recording `VkCommandBuffer` on the shared device, with
    /// no open render pass.
    ///
    /// # Panics
    /// When an external-format generation lacks its conversion.
    #[expect(
        clippy::too_many_arguments,
        clippy::cast_precision_loss,
        reason = "viewport extents are pixel counts and the signature"
    )]
    pub unsafe fn record(
        &mut self,
        cb: vk::CommandBuffer,
        view: vk::ImageView,
        extent: (u32, u32),
        format: vk::Format,
        set0: vk::DescriptorSet,
        draws: &[OpDraw],
        dynamic_offset: u32,
        view_gen: u64,
    ) -> Result<(), NativeError> {
        // `dev` clones cheaply (a handle): holding it across the cache
        // lookups below keeps them borrowable on `self`.
        let dev = self.shared.vk.device.clone();
        // Acquire barriers for first-use generations run ahead of the pass.
        let mut staged = Vec::new();
        for draw in draws {
            // SAFETY: `cb` is a recording command buffer on the shared
            // device — `stage_acquire`'s contract — and `draw.generation`'s
            // handles are live.
            if let Some(pending) = unsafe { sync::stage_acquire(&draw.generation, cb) }? {
                staged.push(pending);
            }
        }
        self.staged.extend(staged);

        let render_pass = self.render_pass(format)?;
        let framebuffer = self.framebuffer(format, view, extent, view_gen)?;
        // SAFETY: `cb` is recording; `render_pass`/`framebuffer` were
        // created on `dev` for this format and view, and the viewport/
        // scissor cover `extent`.
        unsafe {
            dev.cmd_begin_render_pass(
                cb,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(render_pass)
                    .framebuffer(framebuffer)
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D { x: 0, y: 0 },
                        extent: vk::Extent2D {
                            width: extent.0,
                            height: extent.1,
                        },
                    })
                    .clear_values(&[]),
                vk::SubpassContents::INLINE,
            );
            // wgpu's clip convention needs the Y-flip viewport —
            // wgpu-hal issues `y + h / -h` for every wgpu pass and the
            // native pass draws the same instances.
            dev.cmd_set_viewport(
                cb,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: extent.1 as f32,
                    width: extent.0 as f32,
                    height: -(extent.1 as f32),
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            dev.cmd_set_scissor(
                cb,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: extent.0,
                        height: extent.1,
                    },
                }],
            );
        }
        for draw in draws {
            let pipe_key = match draw.generation.repr {
                Repr::Planes { .. } => PipeKey::Planes(format),
                Repr::ExternalFormat { .. } => PipeKey::ExternalFormat(
                    draw.generation.conv.as_ref().expect("conv").set1,
                    format,
                ),
                Repr::Rgb { .. } => return Err(NativeError::Invalid("rgb on native path")),
            };
            let pipeline = self.pipeline(pipe_key)?;
            let layout = self.pipeline_layout(&draw.generation);
            let mask_key = (
                draw.mask.map_or(u64::MAX, ash::vk::Handle::as_raw),
                draw.mask_gen,
            );
            let set1 = write_set1(self, &draw.generation, draw.mask, draw.params, mask_key)?;
            // SAFETY: `cb` is recording inside the render pass;
            // `pipeline`, `layout` and `set0`/`set1` are live objects on
            // `dev` compatible with this pass.
            unsafe {
                dev.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, pipeline);
                dev.cmd_bind_descriptor_sets(
                    cb,
                    vk::PipelineBindPoint::GRAPHICS,
                    layout,
                    0,
                    &[set0, set1],
                    &[dynamic_offset],
                );
                dev.cmd_draw(cb, 6, draw.instance_count, 0, draw.first_instance);
            }
        }
        // SAFETY: `cb` is inside the render pass begun above.
        unsafe { dev.cmd_end_render_pass(cb) };
        Ok(())
    }
}

#[cfg(all(test, unix, not(target_vendor = "apple")))]
mod lavapipe;

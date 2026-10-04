//! The acquisition and release state machine for one frame generation.
//!
//! One immutable generation has one shared acquisition state regardless of
//! how many layers or surfaces reference it. The first consuming submission
//! records the acquire barrier — the producer's actual layout to the
//! sampling layout, and the queue-family transfer from the producer's real
//! family (`FOREIGN_EXT` for the Android contract) to the engine's — and
//! registers the producer semaphore waits immediately before the consuming
//! `queue.submit`. A cancelled, unsubmitted plan leaves the frame
//! unacquired. Retirement schedules a release submission that transitions
//! back and signals the producer's release mechanism; every object is
//! retained until that submission completes. There is no
//! `vkQueueWaitIdle`, no device wait and no render-thread fence wait.

use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::Arc;

use ash::vk;
use ash::vk::Handle as _;
use rustc_hash::FxHashMap;

use super::{Native, NativeError, QueueFamily, ReleaseSync, Shared, Wait, ycbcr};

/// The acquisition state of one frame generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Imported, never consumed.
    Registered,
    /// Acquire barrier + wait staged for the encoder being built.
    AcquisitionPlanned,
    /// Consuming submission accepted; the wait executes on the GPU.
    AcquisitionSubmitted,
    /// Acquisition complete on the queue; later reads need nothing.
    OwnedForRead,
    /// Last retained owner dropped; release is queued.
    Retiring,
    /// The release submission is in flight.
    ReleaseSubmitted,
    /// The release submission completed; objects destroyed.
    Released,
}

/// The one-shot `SYNC_FD` export slot a `ReleaseSync::FenceFd` release
/// fills once its submission is accepted — while the fence semaphore's
/// signal is still pending — and `Generation::release_fd` takes from.
#[derive(Debug)]
pub enum FenceFd {
    /// The release submission has not been accepted yet.
    Pending,
    /// The exported fence fd, taken by exactly one caller.
    Ready(OwnedFd),
    /// The export call itself failed; the payload is the `VkResult` code.
    Failed(i32),
    /// A caller already took the fd (or consumed the failure).
    Taken,
}

/// An object evicted from a bounded encode cache, destroyed once the
/// submission it was encoded for — or any later one — completes.
#[derive(Debug)]
pub enum DestroyItem {
    /// A `VkFramebuffer` evicted from `Native::framebuffers`.
    Framebuffer(vk::Framebuffer),
    /// A set-1 descriptor evicted from `Generation::sets`.
    DescriptorSet {
        /// The pool the set allocates from.
        pool: vk::DescriptorPool,
        /// The set being freed.
        set: vk::DescriptorSet,
    },
}

impl DestroyItem {
    /// Destroys the object on `dev`.
    ///
    /// # Safety
    /// `dev` must be the device the object was created on, and the object
    /// must no longer be referenced by executing or pending work.
    pub unsafe fn destroy(&self, dev: &ash::Device) {
        match *self {
            // SAFETY: the `destroy` contract — `fb` was created on `dev`
            // and is unreferenced by pending work.
            Self::Framebuffer(fb) => unsafe { dev.destroy_framebuffer(fb, None) },
            Self::DescriptorSet { pool, set } => unsafe {
                // SAFETY: the `destroy` contract — `set` came from the
                // live `pool` on `dev` and is unreferenced.
                dev.free_descriptor_sets(pool, &[set])
                    .expect("freeing a pooled set cannot fail");
            },
        }
    }
}

/// A wait payload resolved to its `VkSemaphore` form, ready for
/// `add_wait_semaphore` at submit time.
#[derive(Debug)]
pub struct PendingWait {
    /// The semaphore the consuming submission waits on.
    pub semaphore: vk::Semaphore,
    /// `Some(value)` waits a timeline payload; `None` a binary one.
    pub value: Option<u64>,
    /// Whether the engine created (and so destroys) the semaphore.
    pub owned: bool,
}

/// One generation staged for acquisition in the encoder being built.
#[derive(Debug)]
pub struct PendingAcquire {
    /// The generation being acquired.
    pub generation: Arc<Generation>,
    /// The resolved wait, registered on the queue at submit.
    pub wait: Option<PendingWait>,
}

/// How the frame's planes are bound; kept for the params/repr contract.
#[derive(Debug, Clone, Copy)]
pub enum Views {
    /// `fs_external`: integer plane views (`y`, `uv`).
    Planes {
        /// The luma plane's integer view.
        y: vk::ImageView,
        /// The interleaved chroma plane's integer view.
        uv: vk::ImageView,
    },
    /// `fs_external_format`: the conversion-carrying sampled view.
    /// `Repr::ExternalFormat` — the conversion-attached image view.
    /// Constructed only on Android, read by `write_set1` everywhere.
    #[allow(dead_code)]
    ExternalFormat {
        /// The conversion-attached image view.
        view: vk::ImageView,
    },
    /// `Repr::Rgb` wraps the plane as a `wgpu::Texture`; nothing native.
    Wrapped,
}

/// The producer lease: handles whose lifetime the frame's ownership
/// contract ties to, released last — after every Vulkan object built on
/// them is destroyed.
pub enum Lease {
    /// Nothing extra; the image's own memory import consumed the fds.
    None,
    /// File descriptors kept open through the frame's lifetime (a sync-fd
    /// payload whose fd stays caller-owned, an AHB leak on Android).
    /// Open dmabuf descriptors retained until release completes —
    /// constructed by the Linux dmabuf importer's failures path only on
    /// platforms where `NativeFd::Owned` is possible.
    #[allow(dead_code)]
    Fds(Vec<OwnedFd>),
    /// A retained `AHardwareBuffer`, released on teardown.
    #[cfg(target_os = "android")]
    Ahb(*mut ndk_sys::AHardwareBuffer),
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Fds(fds) => write!(f, "Fds({} fds)", fds.len()),
            #[cfg(target_os = "android")]
            Self::Ahb(_) => write!(f, "Ahb"),
        }
    }
}

#[cfg(target_os = "android")]
impl Drop for Lease {
    fn drop(&mut self) {
        if let Self::Ahb(ptr) = self {
            // SAFETY: `ptr` is this lease's AHardwareBuffer handle,
            // released exactly once here in `Drop`.
            unsafe { ndk_sys::AHardwareBuffer_release(*ptr) };
        }
    }
}

/// Everything the release submission and the eventual destroy need,
/// packaged at the point the last retained owner drops.
#[derive(Debug)]
pub struct Release {
    /// The imported image the release barrier unwinds.
    pub image: vk::Image,
    /// Destroy order: descriptor sets (via the pool), then views, then the
    /// conversion's sampler/conversion/layout, then the image, memory, and
    /// finally the semaphore payloads and producer lease.
    pub pool: Option<vk::DescriptorPool>,
    /// The image views the generation created.
    pub views: Vec<vk::ImageView>,
    /// The shared conversion object, when this generation held the last ref.
    pub conv: Option<Arc<ycbcr::Conv>>,
    /// Imported allocations to free.
    pub memory: Vec<vk::DeviceMemory>,
    /// Semaphores the engine imported or created for this frame.
    pub semaphores: Vec<vk::Semaphore>,
    /// How the release submission acknowledges the producer.
    pub sync_payload: Option<ReleaseSync>,
    /// The `FenceFd` export semaphore, signalled by the release submission
    /// and exported by [`Release::export_fence`].
    pub fence_semaphore: Option<vk::Semaphore>,
    /// The cell `export_fence` fills at submit-acceptance; shared with the
    /// generation so `release_fd` can hand the fd out after the
    /// generation's `Arc` has unwound.
    pub fence_fd: Option<Arc<std::sync::Mutex<FenceFd>>>,
    /// Set when the release submission is accepted; `release_fd` exports
    /// only after this. Shared with the generation's flag.
    pub submitted_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// The generation's state record, so the submit path can advance it
    /// through `ReleaseSubmitted` and `Released` after the generation's
    /// `Arc` has unwound.
    pub state: Option<Arc<std::sync::Mutex<State>>>,
    /// Whether the acquisition barrier ran — the release barrier only
    /// unwinds state the acquire established.
    pub acquired: bool,
    /// The layout the release barrier transitions back to.
    pub producer_layout: vk::ImageLayout,
    /// The family the release barrier hands ownership to.
    pub producer_family: QueueFamily,
    /// The aspects the release barrier covers.
    pub aspects: vk::ImageAspectFlags,
    /// The producer lease, dropped after every object built on it.
    pub lease: Lease,
    /// Release fences of the system compositor planes that showed the
    /// frame, merged into the producer's release fence (#90).
    pub plane_fences: Vec<OwnedFd>,
}

impl Release {
    /// Records the release barrier — sampling layout back to the producer's
    /// layout, and ownership released to its queue family — into `cb`.
    ///
    /// # Safety
    /// `cb` must be a recording `VkCommandBuffer` on the shared device.
    pub unsafe fn encode_barrier(&self, shared: &Shared, cb: vk::CommandBuffer) {
        if !self.acquired {
            return;
        }
        let barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(self.producer_layout)
            .src_queue_family_index(shared.vk.queue_family)
            .dst_queue_family_index(self.producer_family.vk())
            .image(self.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: self.aspects,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: `cb` is recording on `shared`'s device — the caller's
        // contract — and `barrier` references this release's live image.
        unsafe {
            shared.vk.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    /// The semaphores the release submission must signal — the release
    /// sync's payload plus, for `FenceFd`, the export semaphore itself.
    #[must_use]
    pub fn signals(&self) -> Vec<(vk::Semaphore, Option<u64>)> {
        let mut out = Vec::new();
        match &self.sync_payload {
            Some(ReleaseSync::FenceFd) => {
                if let Some(sem) = self.fence_semaphore {
                    out.push((sem, None));
                }
            }
            Some(ReleaseSync::Timeline { semaphore, value }) => {
                out.push((vk::Semaphore::from_raw(*semaphore), Some(*value)));
            }
            None => {}
        }
        out
    }

    /// Exports the `SYNC_FD` fence into the cell `release_fd` reads. Runs
    /// at submit-acceptance — the semaphore's signal is still pending —
    /// before the completion callback destroys the semaphore. On Android
    /// the system compositor's plane release fences are merged in, so the
    /// producer reuses the buffer only once every reader has let go.
    ///
    /// # Panics
    /// On a poisoned export cell.
    pub fn export_fence(&mut self, shared: &Shared) {
        let (Some(semaphore), Some(cell)) = (self.fence_semaphore, self.fence_fd.clone()) else {
            return;
        };
        // SAFETY: `semaphore` is this release's live export semaphore and
        // the info requests its SYNC_FD handle — checked at import.
        let result = unsafe {
            shared
                .vk
                .external_semaphore_fd
                .as_ref()
                .expect("sync-fd export checked at import")
                .get_semaphore_fd(
                    &vk::SemaphoreGetFdInfoKHR::default()
                        .semaphore(semaphore)
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                )
        };
        let exported = match result {
            Ok(fd) => {
                // SAFETY: `get_semaphore_fd` returned a new fd owned by
                // the caller.
                let fence = unsafe { OwnedFd::from_raw_fd(fd) };
                FenceFd::Ready(merge_plane_fences(
                    std::mem::take(&mut self.plane_fences),
                    fence,
                ))
            }
            Err(err) => FenceFd::Failed(err.as_raw()),
        };
        *cell.lock().expect("fence fd cell") = exported;
    }

    /// Destroys the objects in dependency order. Runs once the release
    /// submission has completed, on whichever thread observed completion —
    /// always the render thread in practice, which serializes device calls.
    ///
    /// # Panics
    /// On a poisoned state mutex.
    pub fn destroy(self, shared: &Shared) {
        let dev = &shared.vk.device;
        // SAFETY: every object here was created on `dev` for this
        // generation, and this `destroy` runs exactly once after the
        // release submission completed — no pending work references them.
        unsafe {
            if let Some(pool) = self.pool {
                // Destroying the pool frees every set allocated from it.
                dev.destroy_descriptor_pool(pool, None);
            }
            for view in self.views {
                dev.destroy_image_view(view, None);
            }
        }
        // The conversion's layout, sampler and conversion outlive the sets
        // and views that referenced them.
        if let Some(conv) = self.conv
            && let Ok(conv) = Arc::try_unwrap(conv)
        {
            conv.destroy(&shared.vk);
        }
        // Shared by another live generation: its release owns the last
        // reference and performs the destroy.
        // SAFETY: same contract — `self` was consumed, so these are the
        // objects' only destroys.
        unsafe {
            dev.destroy_image(self.image, None);
            for memory in self.memory {
                dev.free_memory(memory, None);
            }
            for semaphore in self.semaphores {
                dev.destroy_semaphore(semaphore, None);
            }
            if let Some(semaphore) = self.fence_semaphore {
                dev.destroy_semaphore(semaphore, None);
            }
        }
        if let Some(state) = &self.state {
            *state.lock().expect("generation state") = State::Released;
        }
        // The producer lease drops after every Vulkan object built on it.
        drop(self.lease);
    }
}

/// Merges system-compositor plane release fences into the producer's
/// exported fence. On other platforms a plane list is never filled.
fn merge_plane_fences(planes: Vec<OwnedFd>, fence: OwnedFd) -> OwnedFd {
    #[cfg(not(target_os = "android"))]
    {
        drop(planes);
        fence
    }
    #[cfg(target_os = "android")]
    {
        use std::os::fd::AsFd as _;
        planes.into_iter().fold(fence, |merged, plane| {
            ndk::sync::sync_merge(c"cherenkov release", merged.as_fd(), plane.as_fd())
        })
    }
}

/// One imported frame generation — the shared acquisition record every
/// layer attachment deduplicates against.
#[derive(Debug)]
pub struct Generation {
    /// The per-device context this generation was imported on.
    pub shared: Arc<Shared>,
    /// Pixel extent of the frame.
    pub size: (u32, u32),
    /// The decode contract baked into the params uniform.
    pub color: crate::interop::FrameColor,
    /// The alpha contract for RGB planes.
    pub alpha: crate::interop::RgbAlpha,
    /// How the planes bind.
    pub repr: super::Repr,
    /// The imported image; barriers run on it.
    pub image: vk::Image,
    /// The views the native operation binds.
    pub views: Views,
    /// The external-format conversion object, for `Repr::ExternalFormat`.
    pub conv: Option<Arc<ycbcr::Conv>>,
    /// The `Repr::Rgb` wgpu wrapper texture, bound on the ordinary path.
    pub rgb_wrap: Option<wgpu::Texture>,
    /// Imported allocation bytes, charged separately in `Engine::memory`.
    pub bytes: u64,
    /// The producer's actual release-time layout.
    pub producer_layout: vk::ImageLayout,
    /// The producer's actual queue family (`FOREIGN_EXT` for Android).
    pub producer_family: QueueFamily,
    /// The aspects the plane views and barriers cover.
    pub aspects: vk::ImageAspectFlags,
    /// The pool this generation's set-1 descriptors allocate from.
    pub pool: Option<vk::DescriptorPool>,
    /// Set-1 descriptors cached per `(mask view, mask generation)` — the
    /// generation number invalidates entries when the atlas re-creates a
    /// mask texture whose handle a driver could recycle.
    pub sets: std::sync::Mutex<FxHashMap<(u64, u64), vk::DescriptorSet>>,
    /// The shared acquisition state.
    pub state: Arc<std::sync::Mutex<State>>,
    /// The un-imported producer wait descriptor, taken at first use.
    pub wait: std::sync::Mutex<Option<Wait>>,
    /// The resolved wait semaphore, restored by a cancelled plan and
    /// surrendered to the release submission once consumed.
    pub resolved_wait: std::sync::Mutex<Option<PendingWait>>,
    /// The producer's release mechanism.
    pub release_sync: std::sync::Mutex<Option<ReleaseSync>>,
    /// The `FenceFd` export semaphore, created at import when requested
    /// and moved into the `Release` parts at retirement — the release path
    /// owns the export, not the generation.
    pub fence_semaphore: std::sync::Mutex<Option<vk::Semaphore>>,
    /// The export cell `Release::export_fence` fills at submit-acceptance
    /// and `release_fd` takes from. `Some` only for `FenceFd` frames.
    pub fence_fd: Option<Arc<std::sync::Mutex<FenceFd>>>,
    /// True once the release submission has been accepted; shared with
    /// the `Release` parts so the submit path can set it.
    pub release_submitted: Arc<std::sync::atomic::AtomicBool>,
    /// Engine-side retained references — the slots currently holding this
    /// generation. The producer's own `Frame` handles are not counted, so
    /// they may keep the generation alive for `release_fd` without
    /// delaying retirement.
    pub leases: std::sync::atomic::AtomicUsize,
    /// The destroy-time object set, moved out at retirement.
    pub parts: std::sync::Mutex<Option<Release>>,
    /// What a system compositor plane needs to show the buffer directly;
    /// `None` for sources a plane cannot take.
    #[cfg(target_os = "android")]
    pub plane: Option<PlaneSource>,
    /// Release fences delivered by the planes that showed this frame, moved
    /// into the release at retirement.
    pub plane_fences: std::sync::Mutex<Vec<OwnedFd>>,
}

/// An Android buffer as a system compositor plane takes it (#90).
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct PlaneSource {
    /// The buffer, kept alive by the generation's producer lease.
    pub buffer: std::ptr::NonNull<ndk_sys::AHardwareBuffer>,
    /// The fence the plane's transaction hands the system compositor.
    pub acquire: PlaneAcquire,
    /// Whether the buffer was allocated for hardware overlays
    /// (`AHARDWAREBUFFER_USAGE_COMPOSER_OVERLAY`); without it the system
    /// composites the plane on its GPU, which saves nothing.
    pub overlay: bool,
    /// Static HDR metadata for the system's tone mapping.
    pub hdr: crate::interop::HdrMetadata,
}

/// How a plane orders its read behind the producer.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub enum PlaneAcquire {
    /// The producer's work is complete at import.
    Ready,
    /// A sync fence; each transaction receives a duplicate.
    Fence(OwnedFd),
    /// A Vulkan semaphore payload no system compositor can wait on.
    Semaphore,
}

impl Generation {
    /// The generation's current acquisition state.
    ///
    /// # Panics
    /// On a poisoned state mutex.
    pub fn state(&self) -> State {
        *self.state.lock().expect("generation state")
    }

    /// Engine-side references currently held (slot installs).
    #[must_use]
    pub fn lease_count(&self) -> usize {
        self.leases.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl Generation {
    /// The resolved wait payload for the first consuming submission, or
    /// `None` when the frame carries no synchronization.
    ///
    /// fd payloads import here — once, at first use — into a fresh binary
    /// semaphore owned by the generation; timeline payloads carry the
    /// host's semaphore.
    fn resolve_wait(&self) -> Result<Option<PendingWait>, NativeError> {
        let pending = self.resolved_wait.lock().expect("resolved wait").take();
        if let Some(pending) = pending {
            return Ok(Some(pending));
        }
        let Some(wait) = self.wait.lock().expect("frame wait").take() else {
            return Ok(None);
        };
        let dev = &self.shared.vk.device;
        let pending = match wait {
            Wait::Timeline { semaphore, value } => PendingWait {
                semaphore: vk::Semaphore::from_raw(semaphore),
                value: Some(value),
                owned: false,
            },
            Wait::OpaqueFd { fd } => {
                if self.shared.vk.external_semaphore_fd.is_none() {
                    return Err(NativeError::Unsupported("OPAQUE_FD semaphore import"));
                }
                // The imported handle type must be declared exportable at
                // creation (VUID-VkImportSemaphoreFdInfoKHR-handleType-01133).
                let mut export = vk::ExportSemaphoreCreateInfo::default()
                    .handle_types(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
                let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
                let semaphore =
                    // SAFETY: `dev` is live and `info` chains the
                    // OPAQUE_FD export declaration the import needs.
                    unsafe { dev.create_semaphore(&info, None) }.map_err(NativeError::from)?;
                let info = vk::ImportSemaphoreFdInfoKHR::default()
                    .semaphore(semaphore)
                    .flags(vk::SemaphoreImportFlags::TEMPORARY)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD)
                    .fd(fd.as_raw_fd());
                // SAFETY: `semaphore` is live on `dev` and `fd` is the
                // producer's owned wait descriptor — TEMPORARY import
                // leaves it valid for one wait.
                let res = unsafe {
                    self.shared
                        .vk
                        .external_semaphore_fd
                        .as_ref()
                        .expect("checked above")
                        .import_semaphore_fd(&info)
                };
                match res {
                    Ok(()) => {
                        // The driver consumed the fd.
                        let _ = fd.into_raw_fd();
                    }
                    Err(err) => {
                        // SAFETY: `semaphore` was created above and the
                        // failed import leaves it unreferenced —
                        // destroyed exactly once.
                        unsafe { dev.destroy_semaphore(semaphore, None) };
                        return Err(err.into());
                    }
                }
                PendingWait {
                    semaphore,
                    value: None,
                    owned: true,
                }
            }
            Wait::SyncFd { fd } => {
                if self.shared.vk.external_semaphore_fd.is_none() {
                    return Err(NativeError::Unsupported("SYNC_FD semaphore import"));
                }
                let mut export = vk::ExportSemaphoreCreateInfo::default()
                    .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
                let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
                let semaphore =
                    // SAFETY: `dev` is live and `info` chains the
                    // SYNC_FD export declaration the import needs.
                    unsafe { dev.create_semaphore(&info, None) }.map_err(NativeError::from)?;
                let info = vk::ImportSemaphoreFdInfoKHR::default()
                    .semaphore(semaphore)
                    .flags(vk::SemaphoreImportFlags::TEMPORARY)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
                    .fd(fd.as_raw_fd());
                // SAFETY: `semaphore` is live on `dev` and `fd` is the
                // producer's owned wait descriptor — TEMPORARY import
                // leaves it valid for one wait.
                let res = unsafe {
                    self.shared
                        .vk
                        .external_semaphore_fd
                        .as_ref()
                        .expect("checked above")
                        .import_semaphore_fd(&info)
                };
                match res {
                    Ok(()) => {
                        // The driver consumed the fd — same contract as
                        // `OpaqueFd`.
                        let _ = fd.into_raw_fd();
                    }
                    Err(err) => {
                        // SAFETY: `semaphore` was created above and the
                        // failed import leaves it unreferenced —
                        // destroyed exactly once.
                        unsafe { dev.destroy_semaphore(semaphore, None) };
                        return Err(err.into());
                    }
                }
                PendingWait {
                    semaphore,
                    value: None,
                    owned: true,
                }
            }
        };
        Ok(Some(pending))
    }

    /// Records the acquire barrier into `cb` and returns the staged wait.
    ///
    /// # Errors
    /// [`NativeError`] when the frame is retiring or its wait payload is
    /// unexpressible on this device.
    ///
    /// # Safety
    /// `cb` must be a recording `VkCommandBuffer` on the shared device.
    ///
    /// # Panics
    /// On a poisoned state mutex.
    #[allow(clippy::significant_drop_tightening)]
    pub unsafe fn acquire(
        self: &Arc<Self>,
        cb: vk::CommandBuffer,
    ) -> Result<Option<PendingWait>, NativeError> {
        let mut state = self.state.lock().expect("generation state");
        match *state {
            State::OwnedForRead | State::AcquisitionSubmitted => return Ok(None),
            State::Registered | State::AcquisitionPlanned => {}
            _ => return Err(NativeError::Invalid("external frame read while retiring")),
        }
        let wait = self.resolve_wait()?;
        let needs_barrier = self.producer_layout != vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
            || self.producer_family.vk() != self.shared.vk.queue_family;
        if needs_barrier {
            let barrier = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(self.producer_layout)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(self.producer_family.vk())
                .dst_queue_family_index(self.shared.vk.queue_family)
                .image(self.image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: self.aspects,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            // SAFETY: `cb` is recording on `shared`'s device — the
            // caller's contract — and `barrier` references this
            // generation's live image.
            unsafe {
                self.shared.vk.device.cmd_pipeline_barrier(
                    cb,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            }
        }
        *state = State::AcquisitionPlanned;
        Ok(wait)
    }
}

impl Generation {
    /// One engine-side reference (a slot install) began or ended.
    /// Retirement fires when the last engine reference ends; the
    /// producer's own `Frame` handles keep the generation alive for
    /// `release_fd` without being counted here.
    pub fn lease(&self) {
        self.leases
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// Records the fence a system compositor plane releases this frame's
    /// buffer on; the producer's release fence merges it. Deliver it before
    /// the plane's lease ends.
    ///
    /// # Panics
    /// On a poisoned plane-fence mutex.
    pub fn add_plane_release(&self, fence: OwnedFd) {
        self.plane_fences.lock().expect("plane fences").push(fence);
    }

    /// Drops one engine-side reference; the last one retires the frame.
    pub fn unlease(&self) {
        if self
            .leases
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
            == 1
        {
            self.retire_inner();
        }
    }

    /// Packages the destroy-time object set and queues the release
    /// submission. Idempotent: the first call wins; a generation dropped
    /// while `parts` are already out queues nothing.
    fn retire_inner(&self) {
        let mut parts = self.parts.lock().expect("release parts").take();
        if let Some(release) = parts.as_mut() {
            // A resolved-but-unconsumed wait semaphore dies with the frame.
            if let Some(wait) = self.resolved_wait.lock().expect("resolved wait").take()
                && wait.owned
            {
                release.semaphores.push(wait.semaphore);
            }
            release.acquired = matches!(
                *self.state.lock().expect("generation state"),
                State::OwnedForRead | State::AcquisitionSubmitted | State::Retiring
            );
            release.sync_payload = self.release_sync.lock().expect("release sync").take();
            release.fence_semaphore = self.fence_semaphore.lock().expect("fence semaphore").take();
            release.fence_fd.clone_from(&self.fence_fd);
            release.plane_fences =
                std::mem::take(&mut *self.plane_fences.lock().expect("plane fences"));
            release.submitted_flag = Some(Arc::clone(&self.release_submitted));
            release.state = Some(Arc::clone(&self.state));
            for (_, set) in self.sets.lock().expect("frame sets").drain() {
                // Sets are freed with the pool; the map drain is bookkeeping.
                let _ = set;
            }
        }
        if let Some(release) = parts.take() {
            *self.state.lock().expect("generation state") = State::Retiring;
            self.shared.retire(release);
        }
    }
}

impl Drop for Generation {
    /// A generation that was never leased — imported but never installed
    /// — still retires its lease and objects; a leased generation already
    /// retired when its last slot dropped.
    fn drop(&mut self) {
        self.retire_inner();
    }
}

impl Generation {
    /// Takes the `FenceFd` release payload the release path exported at
    /// submit-acceptance. The returned fd signals when the producer may
    /// reuse the buffer: the release submission's pending signal has
    /// executed, and every system compositor plane that showed the frame
    /// has released it. Exactly one caller can take it.
    ///
    /// # Errors
    /// [`NativeError::Unready`] before the release submission is accepted;
    /// [`NativeError::Unsupported`] when the frame has no `FenceFd`
    /// release mechanism; [`NativeError::Vulkan`] when the driver's export
    /// failed; [`NativeError::Invalid`] on a second take.
    ///
    /// # Panics
    /// On a poisoned export cell.
    pub fn release_fd(&self) -> Result<OwnedFd, NativeError> {
        let Some(cell) = &self.fence_fd else {
            return Err(NativeError::Unsupported("frame has no fence release"));
        };
        let mut cell = cell.lock().expect("fence fd cell");
        let result = match std::mem::replace(&mut *cell, FenceFd::Taken) {
            FenceFd::Ready(fd) => Ok(fd),
            FenceFd::Failed(code) => Err(NativeError::Vulkan(code)),
            FenceFd::Taken => Err(NativeError::Invalid("release fence already taken")),
            FenceFd::Pending => {
                *cell = FenceFd::Pending;
                if self
                    .release_submitted
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    Err(NativeError::Invalid(
                        "release submission accepted without a fence export",
                    ))
                } else {
                    Err(NativeError::Unready)
                }
            }
        };
        drop(cell);
        result
    }
}

/// Imports a sync fence into a new binary semaphore, temporarily. A
/// successful import hands the descriptor to the driver; on failure it
/// closes with `fd`.
///
/// # Errors
/// [`NativeError::Unsupported`] when sync-fd import is absent;
/// [`NativeError`] from semaphore creation or the driver's import.
#[cfg(target_os = "android")]
pub fn import_sync_fd(shared: &Shared, fd: OwnedFd) -> Result<vk::Semaphore, NativeError> {
    let Some(loader) = shared.vk.external_semaphore_fd.as_ref() else {
        return Err(NativeError::Unsupported("SYNC_FD semaphore import"));
    };
    let dev = &shared.vk.device;
    // The imported handle type must be declared at creation
    // (VUID-VkImportSemaphoreFdInfoKHR-handleType-01133).
    let mut export = vk::ExportSemaphoreCreateInfo::default()
        .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
    // SAFETY: `dev` is live and `info` chains the SYNC_FD export
    // declaration the import needs.
    let semaphore = unsafe { dev.create_semaphore(&info, None) }.map_err(NativeError::from)?;
    let info = vk::ImportSemaphoreFdInfoKHR::default()
        .semaphore(semaphore)
        .flags(vk::SemaphoreImportFlags::TEMPORARY)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
        .fd(fd.as_raw_fd());
    // SAFETY: `semaphore` is live on `dev` and `fd` is the producer's
    // owned descriptor — TEMPORARY import leaves it valid for one wait.
    match unsafe { loader.import_semaphore_fd(&info) } {
        Ok(()) => {
            // The driver owns the descriptor now.
            let _ = fd.into_raw_fd();
            Ok(semaphore)
        }
        Err(err) => {
            // SAFETY: `semaphore` was created above and the failed
            // import leaves it unreferenced — destroyed exactly once.
            unsafe { dev.destroy_semaphore(semaphore, None) };
            Err(err.into())
        }
    }
}

/// Stages `generation`'s acquisition into the encoder: records the barrier and
/// returns the pending-wait entry to stage.
///
/// # Errors
/// [`NativeError`] propagated from [`Generation::acquire`].
///
/// # Safety
/// `cb` must be a recording `VkCommandBuffer` on the shared device.
///
/// # Panics
/// On a poisoned state mutex.
pub unsafe fn stage_acquire(
    generation: &Arc<Generation>,
    cb: vk::CommandBuffer,
) -> Result<Option<PendingAcquire>, NativeError> {
    // SAFETY: `cb` is a recording command buffer on the shared device —
    // this function's own contract, which `acquire` requires.
    let wait = unsafe { generation.acquire(cb) }?;
    let staged = wait.map(|wait| PendingAcquire {
        generation: Arc::clone(generation),
        wait: Some(wait),
    });
    if staged.is_none()
        && *generation.state.lock().expect("generation state") == State::AcquisitionPlanned
    {
        // No wait payload — still mark acquisition as staged so the submit
        // finalises it.
        return Ok(Some(PendingAcquire {
            generation: Arc::clone(generation),
            wait: None,
        }));
    }
    Ok(staged)
}

/// Registers every staged generation's wait on the queue immediately
/// before the consuming submission.
pub fn submit_waits(native: &Native, queue: &wgpu::hal::vulkan::Queue) {
    for pending in &native.staged {
        if let Some(wait) = &pending.wait {
            queue.add_wait_semaphore(
                wait.semaphore,
                wait.value,
                // Conservative stage covering the acquire and all dependent
                // work; narrowing it is a separately measured change.
                vk::PipelineStageFlags::ALL_COMMANDS,
            );
        }
    }
}

/// Finalises staged generations after the consuming submission is
/// accepted: binary waits are consumed once for the generation, and each
/// staged generation's state record lands in `native.acquiring` so the
/// submission-completion callback can promote it to `OwnedForRead`.
/// Timeline waits may legally repeat but are unnecessary on the ordered
/// queue.
///
/// # Panics
/// On a poisoned state or parts mutex.
pub fn mark_submitted(native: &mut Native) {
    for pending in native.staged.drain(..) {
        if let Some(wait) = pending.wait
            && wait.owned
        {
            // The consumed binary semaphore stays alive through the
            // release submission, then dies with the frame's objects.
            let mut parts = pending.generation.parts.lock().expect("release parts");
            if let Some(release) = parts.as_mut() {
                release.semaphores.push(wait.semaphore);
            } else {
                // SAFETY: `wait.semaphore` was imported for this
                // generation, is consumed by the submission just made,
                // and has no release to die with — destroyed exactly
                // once here.
                unsafe {
                    native
                        .shared
                        .vk
                        .device
                        .destroy_semaphore(wait.semaphore, None);
                };
            }
        }
        let mut state = pending.generation.state.lock().expect("generation state");
        if *state == State::AcquisitionPlanned {
            *state = State::AcquisitionSubmitted;
        }
        drop(state);
        native.acquiring.push(Arc::clone(&pending.generation.state));
    }
}

/// Promotes generations whose consuming submission just completed from
/// `AcquisitionSubmitted` to `OwnedForRead`. Runs on the queue's
/// completion callback.
///
/// # Panics
/// On a poisoned state mutex.
pub fn mark_owned(states: Vec<Arc<std::sync::Mutex<State>>>) {
    for state in states {
        let mut state = state.lock().expect("generation state");
        if *state == State::AcquisitionSubmitted {
            *state = State::OwnedForRead;
        }
    }
}

/// Cancels an unsubmitted plan: frames go back to unacquired and staged
/// queue waits are dropped — nothing was registered on the queue yet.
///
/// # Panics
/// On a poisoned state or wait mutex.
pub fn cancel_staged(native: &mut Native) {
    for pending in native.staged.drain(..) {
        let mut state = pending.generation.state.lock().expect("generation state");
        if *state == State::AcquisitionPlanned {
            *state = State::Registered;
        }
        drop(state);
        if let Some(wait) = pending.wait {
            // The resolved payload goes back to the generation so a later
            // plan reuses it — an imported fd cannot be imported twice.
            *pending
                .generation
                .resolved_wait
                .lock()
                .expect("resolved wait") = Some(wait);
        }
    }
}

/// Drains the retire queue into release submissions.
///
/// Called before each submission so an idle engine still processes a
/// pending retirement the next time anything is submitted; the renderer
/// calls it from `flush_releases` with a fresh encoder.
///
/// # Panics
/// On a poisoned release-queue mutex.
#[must_use]
pub fn drain_releases(native: &Native) -> Vec<Release> {
    native
        .shared
        .vk
        .pending_release
        .lock()
        .expect("pending release")
        .drain(..)
        .collect()
}

/// Drains the deferred-destroy queue of evicted cache objects. The submit
/// path hands them to `queue.on_submitted_work_done`, where destruction
/// provably cannot race in-flight references.
///
/// # Panics
/// On a poisoned destroy-queue mutex.
#[must_use]
pub fn drain_destroys(native: &Native) -> Vec<DestroyItem> {
    native
        .shared
        .vk
        .pending_destroy
        .lock()
        .expect("pending destroy")
        .drain(..)
        .collect()
}

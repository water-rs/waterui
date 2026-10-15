//! Composition attribution (issue #2547): what each epoch keeps on chip
//! and why every other logical image is materialized.
//!
//! When a [`Sink`] is installed through `GpuConfig::composition_diag`,
//! each encoded surface records one [`FrameReport`]: the frame's tile
//! epochs with the passes they cover, every materialized logical image
//! with the [`Materialization`] reason the planner decided, the
//! attachment storage the frame held split into memoryless and backed,
//! the logical intermediate traffic, and the tile pipelines created on
//! first use.
//!
//! Without a sink nothing is collected: the recording path is one
//! `Option` test per surface, and no report structure is built. The
//! on-chip decisions themselves are always made — the planner names a
//! reason for every image whether or not anyone is listening — so a
//! disabled sink costs neither allocation nor analysis.
//!
//! The byte counts are *logical*: the bytes the frame's images and reads
//! describe, not the traffic the memory system saw. Physical bandwidth
//! belongs to the device's own counters and is never inferred here.
//!
//! The planner that produces these decisions is platform-neutral
//! (issue #2549), but it runs only where a backend supplies a tile
//! executor and its limits (Apple, issue #2191 tracks Vulkan).
//! Elsewhere the frame is never planned, so no report is recorded and
//! every image's reason would be [`Materialization::NoExecutor`].

use std::ops::Range;
use std::sync::{Arc, Mutex};

/// Why a logical image is materialized instead of kept in an epoch's
/// attachment slot (issue #2547).
///
/// The planner decides one of these for every logical image of a frame:
/// the attempt that opens an image names the reason the interval rooted
/// at it did not fuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Materialization {
    /// The image is not an intermediate: a surface part, a plane, a
    /// backdrop group's capture, or a retained projected image.
    Persistent,
    /// Producer and last consumer are the same pass — there is no
    /// producer→consumer interval to keep on chip.
    NoInterval,
    /// The interval's last consumer does not write an image an epoch can
    /// hold as slot zero: it writes the intermediate itself, or a target
    /// class the tile executor cannot store to.
    OutputNotEligible,
    /// A consumer reads more than the corresponding pixel — a blur,
    /// refraction, arbitrary UV, or a mip chain — which attachment fetch
    /// cannot serve.
    NeighborhoodRead,
    /// A frozen backdrop snapshot that is not provably equal to the live
    /// destination, so the snapshot stays a separate image.
    FrozenSnapshot,
    /// A producer or consumer outside the interval: a capture, a later
    /// frame, a retained image, or a host.
    External,
    /// The image's pixel format differs from the epoch's attachment
    /// format.
    Format,
    /// The pass composites in a different blend space than the image it
    /// writes, so the epoch would move a conversion point.
    BlendSpace,
    /// The image covers pixels outside the epoch's attachment region.
    Region,
    /// Every attachment slot within the device's limit is already live.
    SlotLimit,
    /// An operation the on-chip executor does not implement: a filter, a
    /// shadow, a mip build, a custom effect, a content shader, or a blend
    /// other than source-over and replace.
    UnsupportedOp,
    /// The backend's tile executor orders accesses to a slot only
    /// between passes, so no draw may read what an earlier draw of the
    /// same pass wrote and no epoch can form.
    Ordering,
    /// This target has no tile executor at all, so the frame is never
    /// planned and every logical image is materialized (issue #2549).
    NoExecutor,
    /// The target has a tile executor, but *this device* was not opened
    /// with the capabilities it needs, so the engine installed none and
    /// the frame is never planned.
    ///
    /// On Vulkan the executor needs a rendering scope's colour
    /// attachments to be readable per pixel — `dynamicRendering`,
    /// `VK_KHR_dynamic_rendering_local_read` and rasterization-order
    /// attachment access together. A device missing local read offers
    /// no attachment read at all; one missing ordered access offers it
    /// only between draws, which cannot carry the engine's
    /// same-pass read-after-write (#2191). Both land here, because the
    /// distinction is the device's, not the plan's.
    DeviceAccess,
}

impl Materialization {
    /// The reason's stable report name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::NoInterval => "no-interval",
            Self::OutputNotEligible => "output-not-eligible",
            Self::NeighborhoodRead => "neighborhood-read",
            Self::FrozenSnapshot => "frozen-snapshot",
            Self::External => "external",
            Self::Format => "format",
            Self::BlendSpace => "blend-space",
            Self::Region => "region",
            Self::SlotLimit => "slot-limit",
            Self::UnsupportedOp => "unsupported-op",
            Self::Ordering => "ordering",
            Self::NoExecutor => "no-executor",
            Self::DeviceAccess => "device-access",
        }
    }
}

/// A shared sink the renderer records into and the caller drains.
///
/// `Sink::new` hands a handle to `GpuConfig::composition_diag`; after the
/// run, [`Sink::take`] reads the reports. The handle is `Send + Sync` and
/// may be cloned; one engine writes it.
#[derive(Clone, Default)]
pub struct Sink {
    inner: Arc<Mutex<Vec<FrameReport>>>,
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.inner.lock().map_or(0, |reports| reports.len());
        f.debug_struct("composition::Sink")
            .field("reports", &count)
            .finish()
    }
}

impl Sink {
    /// An empty sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drains the recorded reports, oldest first.
    ///
    /// # Panics
    /// When the sink's lock is poisoned.
    #[must_use]
    pub fn take(&self) -> Vec<FrameReport> {
        std::mem::take(&mut *self.inner.lock().expect("composition diag poisoned"))
    }

    /// How many reports are recorded so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |reports| reports.len())
    }

    /// Whether anything is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends one surface's report.
    ///
    /// # Panics
    /// When the sink's lock is poisoned.
    pub fn record(&self, report: FrameReport) {
        self.inner
            .lock()
            .expect("composition diag poisoned")
            .push(report);
    }
}

/// One encoded surface of one frame.
#[derive(Clone, Debug)]
pub struct FrameReport {
    /// The engine frame id this surface was encoded for.
    pub frame: u64,
    /// The surface's raw id.
    pub surface: u64,
    /// Passes the frame's canonical plan holds.
    pub passes: usize,
    /// The tile epochs, in pass order.
    pub epochs: Vec<Epoch>,
    /// Every logical image that reaches device memory, with its reason.
    pub materialized: Vec<MaterializedImage>,
    /// Logical images the epochs keep in attachment slots.
    pub on_chip: Vec<OnChipImage>,
    /// The attachment storage the frame held.
    pub attachments: AttachmentBytes,
    /// The frame's logical intermediate traffic.
    pub traffic: Traffic,
    /// Tile composition pipelines created on first use during this
    /// surface's encode.
    pub pipelines: Vec<PipelineCreated>,
}

/// One tile epoch: the pass interval it fuses and what it holds.
#[derive(Clone, Debug)]
pub struct Epoch {
    /// The canonical passes the epoch covers, as `start..end`.
    pub passes: Range<usize>,
    /// Attachment slots the epoch binds, slot zero (its stored output)
    /// included.
    pub slots: u8,
    /// The epoch's attachment region in device pixels, `[x, y, w, h]`.
    pub region: [u32; 4],
    /// Logical images the epoch keeps in slots other than zero.
    pub on_chip_images: usize,
}

/// A logical image class, as the canonical plan names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    /// A surface part: the frame's own output.
    Part,
    /// An isolation scratch at some depth.
    Scratch,
    /// A blend backdrop, a group capture included.
    Backdrop,
    /// A retained projective image.
    Projected,
    /// A system-compositor plane.
    Plane,
}

impl ImageKind {
    /// The kind's stable report name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Part => "part",
            Self::Scratch => "scratch",
            Self::Backdrop => "backdrop",
            Self::Projected => "projected",
            Self::Plane => "plane",
        }
    }

    /// Whether an image of this class is an intermediate: everything but
    /// the surface's own parts and its compositor planes.
    #[must_use]
    pub const fn intermediate(self) -> bool {
        matches!(self, Self::Scratch | Self::Backdrop | Self::Projected)
    }
}

/// A logical image written to device memory, and why.
#[derive(Clone, Copy, Debug)]
pub struct MaterializedImage {
    /// Which class of image it is.
    pub kind: ImageKind,
    /// The image's generation: every clear starts a new one.
    pub generation: usize,
    /// Its region in device pixels, `[x, y, w, h]`.
    pub region: [u32; 4],
    /// Why the planner materialized it.
    pub reason: Materialization,
    /// The region's bytes in the image's own format — a logical size,
    /// not the texture allocation that holds it.
    pub logical_bytes: u64,
}

/// A logical image an epoch keeps in an attachment slot.
#[derive(Clone, Copy, Debug)]
pub struct OnChipImage {
    /// Which class of image it is.
    pub kind: ImageKind,
    /// The image's generation.
    pub generation: usize,
    /// Its region in device pixels, `[x, y, w, h]`.
    pub region: [u32; 4],
    /// The bytes the image would have cost had it been materialized —
    /// what staying on chip saves, logically.
    pub logical_bytes: u64,
}

/// The attachment storage one surface's frame held.
///
/// Every number is a peak over the frame: the renderer allocates a
/// surface's attachments for the whole frame, so each is the storage
/// bound while any pass of the frame is encoded.
///
/// The tile slots are reported twice, because the two answers are not
/// the same question. *Reserved* is what the epochs' slots would have
/// cost in device memory — the saving claimed by keeping them on chip.
/// *Committed* is what the device says it actually spent on them, and
/// only a platform that can be asked reports it.
#[derive(Clone, Copy, Debug, Default)]
pub struct AttachmentBytes {
    /// Bytes the epochs' tile slots reserve. Their contents are not
    /// meant to leave tile memory, so this is storage the frame asked
    /// for but does not expect to take from device memory.
    pub peak_reserved_bytes: u64,
    /// Bytes the device reports committed for those slots.
    ///
    /// `Some(0)` is the good case: every slot stayed on chip. A larger
    /// value is the driver admitting it had to back some of them, and
    /// that is real device memory the reserved figure would have hidden.
    /// `None` where the platform has no such query — Metal's
    /// `MTLStorageMode::Memoryless` has no commitment to read, so Apple
    /// reports only the reserved figure.
    pub peak_committed_bytes: Option<u64>,
    /// Bytes of device-memory attachments the frame bound: the surface
    /// parts and the materialized isolation scratches.
    pub peak_backed_bytes: u64,
}

/// The frame's logical intermediate traffic.
///
/// Intermediates are every image but the surface's own parts. The counts
/// are what the plan describes — region bytes per write, read and copy —
/// and never a measurement of memory-system traffic.
#[derive(Clone, Copy, Debug, Default)]
pub struct Traffic {
    /// Logical bytes passes wrote into intermediates.
    pub logical_bytes_written: u64,
    /// Logical bytes draws sampled from intermediates.
    pub logical_bytes_read: u64,
    /// Logical bytes copied between images (backdrop snapshots and
    /// group captures).
    pub logical_bytes_copied: u64,
    /// Read-after-write boundaries between passes: a materialized
    /// intermediate sampled after an earlier pass wrote it. Each is a
    /// point the backend must order the two accesses at — on Metal the
    /// pass boundary itself, under a by-region barrier elsewhere.
    pub barriers: u32,
}

/// The fragment stage a tile pipeline composites with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// The solid-paint stage.
    Simple,
    /// The silhouette-shadow stage.
    Shadow,
    /// The full paint stage, including gradients and
    /// corresponding-pixel source reads.
    Source,
    /// The slot clear that reopens a logical image inside an epoch.
    Clear,
}

impl Variant {
    /// The variant's stable report name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Shadow => "shadow",
            Self::Source => "source",
            Self::Clear => "clear",
        }
    }
}

/// A tile composition pipeline created on first use.
///
/// The tile pipeline set is keyed by its attachment layout, so the first
/// frame that needs a key pays for its creation — the same first-use cost
/// the core pipeline set reports through the creation ledger (#170).
#[derive(Clone, Copy, Debug)]
pub struct PipelineCreated {
    /// The canonical pass that first needed it.
    pub pass: usize,
    /// The attachment slot it writes.
    pub target: u8,
    /// The slot it reads, equal to `target` when it reads none.
    pub source: u8,
    /// The fragment stage.
    pub variant: Variant,
    /// Whether it replaces instead of blending.
    pub replace: bool,
    /// Attachment slots bound when it was created.
    pub slots: u8,
}

/// The report of one encoded surface, built from the decided plan.
///
/// Called only with a sink installed; it allocates and walks the frame.
pub(crate) fn frame_report(
    frame: u64,
    surface: u64,
    lowered: &super::lower::Frame,
    plan: &super::composite::plan::ExecutionPlan,
    attachments: AttachmentBytes,
    pipelines: Vec<PipelineCreated>,
) -> FrameReport {
    use super::composite::plan::Storage;

    let epochs = plan
        .epochs()
        .map(|epoch| Epoch {
            passes: epoch.passes.clone(),
            slots: epoch.slots,
            region: epoch.region,
            on_chip_images: epoch
                .ops
                .iter()
                .filter(|op| plan.targets[op.pass] != epoch.output)
                .count(),
        })
        .collect();
    let mut materialized = Vec::new();
    let mut on_chip = Vec::new();
    for image in &plan.images {
        let kind = kind(image.target);
        let region = image.region;
        let logical_bytes = region_bytes(region, image.format);
        match image.storage {
            Storage::Materialized(reason) => materialized.push(MaterializedImage {
                kind,
                generation: image.generation,
                region,
                reason,
                logical_bytes,
            }),
            Storage::OnChip => on_chip.push(OnChipImage {
                kind,
                generation: image.generation,
                region,
                logical_bytes,
            }),
        }
    }
    FrameReport {
        frame,
        surface,
        passes: lowered.passes.len(),
        epochs,
        materialized,
        on_chip,
        attachments,
        traffic: traffic(lowered, plan),
        pipelines,
    }
}

/// The frame's logical intermediate traffic.
fn traffic(lowered: &super::lower::Frame, plan: &super::composite::plan::ExecutionPlan) -> Traffic {
    use super::composite::plan::Storage;

    let mut traffic = Traffic::default();
    for (index, pass) in lowered.passes.iter().enumerate() {
        let target = &plan.images[plan.targets[index].0];
        let bytes = region_bytes(pass.region, target.format);
        if kind(target.target).intermediate() {
            traffic.logical_bytes_written += bytes;
        }
        // A backdrop snapshot copies the destination's region; a group
        // capture copies its source's region into the capture image.
        if pass.backdrop_copy.is_some() {
            traffic.logical_bytes_copied += bytes;
        }
        if pass.capture.is_some() {
            traffic.logical_bytes_copied += bytes;
        }
        for read in plan.reads[index].iter().flatten() {
            let source = &plan.images[read.image.0];
            if !kind(source.target).intermediate() {
                continue;
            }
            traffic.logical_bytes_read += region_bytes(source.region, source.format);
            // A read of a materialized image produced by an earlier pass
            // is a read-after-write the backend has to order.
            if matches!(source.storage, Storage::Materialized(_))
                && source.producer.is_some_and(|producer| producer < index)
            {
                traffic.barriers += 1;
            }
        }
    }
    traffic
}

/// The bytes a region covers in `format` — a logical size, never an
/// allocation.
fn region_bytes(region: [u32; 4], format: wgpu::TextureFormat) -> u64 {
    u64::from(region[2]) * u64::from(region[3]) * super::texel_bytes(format)
}

/// The report class of a canonical target.
const fn kind(target: super::lower::Target) -> ImageKind {
    use super::lower::Target;

    match target {
        Target::Part(_) => ImageKind::Part,
        Target::Scratch(_) => ImageKind::Scratch,
        Target::Backdrop { .. } => ImageKind::Backdrop,
        Target::Projected(_) => ImageKind::Projected,
        Target::Plane(_) => ImageKind::Plane,
    }
}

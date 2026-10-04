//! Allocation-event diagnostic (issue #169).
//!
//! When a [`Sink`] is installed through `GpuConfig::alloc_diag`, the
//! renderer records an [`AllocEvent`] around every GPU allocation,
//! growth, upload and submission boundary. Each event carries its
//! sequence number, the frame and surface it belongs to, the resource's
//! label and class, the old and new requested capacity where the event
//! changes one, the submission ordinal, and the wgpu allocator's state
//! (allocated and reserved bytes, allocation and block counts, the
//! allocation-level delta since the previous event, and the engine's
//! own accounting of live and retired-but-unfreed bytes).
//!
//! The trace exists to find the first interval in which reserved
//! storage grows a second memory block, and to say which labels and how
//! many bytes are implicated. It is a diagnostic: keep it out of timed
//! and energy runs — the per-event allocator snapshot is deliberately
//! expensive.
//!
//! Recording is thread-local: the renderer installs its sink for the
//! duration of each entry point that performs GPU work, so helpers
//! deeper in the call tree emit events without a signature change.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

/// The wgpu-internal staging buffer's allocator name.
const STAGING_NAME: &str = "(wgpu internal) Staging";

/// A resource's functional class in the trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A frame-wide storage/uniform buffer (instances, stops, globals).
    Buffer,
    /// A host-readable buffer (readback, timestamp staging).
    MapBuffer,
    /// Timestamp query resolve buffer or query set.
    Query,
    /// The R8 coverage atlas.
    Atlas,
    /// A dedicated clip-mask `R8Unorm` texture.
    MaskTexture,
    /// A render target (surface target, isolation scratch, blend
    /// backdrop, backdrop capture, dummy view source).
    Target,
    /// An uploaded image texture.
    Image,
    /// Anything not classified above.
    Other,
}

impl Class {
    /// The class a resource label denotes.
    #[must_use]
    pub fn for_label(label: &str) -> Self {
        match label {
            "instances" | "stops" | "globals" | "present uniform" | "engine data" => Self::Buffer,
            "readback" | "timestamp staging" => Self::MapBuffer,
            "timestamp resolve" | "frame timestamps" => Self::Query,
            "glyph atlas" => Self::Atlas,
            "clip mask" => Self::MaskTexture,
            "image" => Self::Image,
            "surface target" | "isolation scratch" | "blend backdrop" | "backdrop capture"
            | "dummy source" | "source texture" | "projective image" => Self::Target,
            _ => Self::Other,
        }
    }
}

/// One allocation in an [`AllocEvent`]'s delta: name, size and the block
/// it lives in.
#[derive(Clone, Debug)]
pub struct AllocDelta {
    /// The allocator's name for the allocation (the resource's label).
    pub name: String,
    /// The allocation's size in bytes — the actual requirement the
    /// driver bound, alignment included.
    pub size: u64,
    /// The allocation's offset within its block.
    pub offset: u64,
    /// Index of the memory block holding it (per report order).
    pub block: u64,
}

/// The wgpu allocator's state at one event.
#[derive(Clone, Debug, Default)]
pub struct AllocSnap {
    /// `AllocatorReport::total_allocated_bytes`.
    pub allocated: u64,
    /// `AllocatorReport::total_reserved_bytes`.
    pub reserved: u64,
    /// Live allocation count.
    pub allocations: u64,
    /// Memory block count.
    pub blocks: u64,
    /// Per-block reserved sizes, in report order.
    pub block_sizes: Vec<u64>,
    /// Live staging allocations (`(wgpu internal) Staging`) as
    /// `(count, bytes)` — queue staging is the allocation class whose
    /// accumulation the issue tracks.
    pub staging: (u64, u64),
    /// Per-label live `(name, count, bytes)`, attached on detailed
    /// events: block-count changes and frame boundaries.
    pub by_name: Option<Vec<(String, u64, u64)>>,
}

/// What happened at an event.
#[derive(Clone, Debug)]
pub enum EventKind {
    /// `render` began for the engine frame.
    FrameBegin,
    /// `render` returned; `drew` records whether it drew.
    FrameEnd {
        /// Whether the frame produced a submission.
        drew: bool,
    },
    /// A named marker in the trace (e.g. "teardown").
    Phase {
        /// The marker's name.
        name: &'static str,
    },
    /// A buffer or texture was created.
    Create {
        /// The resource's wgpu label.
        label: &'static str,
        /// Its class.
        class: Class,
        /// Requested size in bytes.
        bytes: u64,
    },
    /// A buffer or texture was recreated larger, replacing the previous
    /// one. `old` is the replaced resource's byte size.
    Grow {
        /// The resource's wgpu label.
        label: &'static str,
        /// Its class.
        class: Class,
        /// Replaced size in bytes.
        old: u64,
        /// New requested size in bytes.
        new: u64,
        /// Bytes of the predecessor copied over — the copy submission
        /// keeps the old buffer live until it completes.
        preserve: u64,
    },
    /// The renderer dropped its last handle on a resource — a bind
    /// group, view or buffer/texture slot replacement. The allocation
    /// itself frees when every submission referencing it completes.
    Retire {
        /// The resource's wgpu label.
        label: &'static str,
        /// Its class.
        class: Class,
        /// Its byte size.
        bytes: u64,
        /// The last submission ordinal that used it, when any.
        last_used: Option<u64>,
        /// What retired it.
        reason: &'static str,
    },
    /// `Queue::write_buffer`/`write_texture`: `bytes` uploaded to the
    /// resource `label`.
    Upload {
        /// The destination resource's wgpu label.
        label: &'static str,
        /// Its class.
        class: Class,
        /// Useful bytes written.
        bytes: u64,
        /// `(x, y, w, h)` for texture writes.
        rect: Option<(u32, u32, u32, u32)>,
    },
    /// `Queue::submit` enqueued work; `index` is the renderer's own
    /// submission ordinal (wgpu's `SubmissionIndex` is opaque).
    Submit {
        /// Submission ordinal.
        index: u64,
        /// The encoder's label.
        encoder: &'static str,
    },
    /// A buffer map was requested.
    Map {
        /// The buffer's wgpu label.
        label: &'static str,
        /// The mapped size.
        bytes: u64,
    },
    /// `Device::poll`/`wait` observed; `wait_finished` reports whether a
    /// waited submission completed.
    Poll {
        /// Whether the awaited submission reported finished.
        wait_finished: bool,
    },
    /// A bind-group set was dropped without its resources.
    BindGroups {
        /// How many bind groups were dropped.
        dropped: u64,
        /// Why ("buffer growth", "atlas generation", "stamp change",
        /// "trim", "resize", "destroy").
        reason: &'static str,
    },
    /// An atlas cell was placed without an upload (zero-sized cells).
    AtlasCell {
        /// Cell `(x, y, w, h)`.
        rect: (u32, u32, u32, u32),
    },
}

/// One recorded boundary.
#[derive(Clone, Debug)]
pub struct AllocEvent {
    /// Monotonic index.
    pub seq: u64,
    /// Engine frame being rendered, inside `render`.
    pub frame: Option<u64>,
    /// The surface the work belongs to, when known.
    pub surface: Option<u64>,
    /// The section of the pipeline the event is in ("init", "lower",
    /// "uploads", "encode", "timestamps", "present", "readback",
    /// "teardown").
    pub phase: &'static str,
    /// What happened.
    pub kind: EventKind,
    /// Allocator state at this point.
    pub alloc: AllocSnap,
    /// Allocations that appeared since the previous event.
    pub new: Vec<AllocDelta>,
    /// Allocations that disappeared since the previous event.
    pub freed: Vec<AllocDelta>,
    /// Engine-accounted live bytes: resources the renderer holds.
    pub live_bytes: u64,
    /// Bytes retired after the newest completed submission — the
    /// estimate of dead-but-held memory.
    pub retired_in_flight: u64,
    /// The newest submission ordinal seen.
    pub last_submission: u64,
    /// The newest submission ordinal confirmed complete.
    pub completed_submission: u64,
}

/// `(name, size, offset, block)` — one live allocation's identity in a
/// report.
type AllocKey = (String, u64, u64, u64);

struct Inner {
    events: Vec<AllocEvent>,
    seq: u64,
    frame: Option<u64>,
    surface: Option<u64>,
    phase: &'static str,
    last_submission: u64,
    completed_submission: u64,
    /// Submissions whose completion callback has not run, in order.
    pending_submits: VecDeque<u64>,
    /// Every live allocation's key at the previous event.
    last_allocs: HashSet<AllocKey>,
    /// Engine-held bytes.
    live_bytes: u64,
    /// Retired `(used_submission, bytes)` not yet provably freed.
    retired: VecDeque<(u64, u64)>,
    /// Previous event's block count, for detail flagging.
    last_blocks: u64,
}

fn block_of(blocks: &[wgpu::wgt::MemoryBlockReport], i: usize) -> u64 {
    blocks
        .iter()
        .position(|b| b.allocations.contains(&i))
        .unwrap_or(0) as u64
}

impl Inner {
    fn event(&mut self, device: &wgpu::Device, kind: EventKind) {
        let Some(report) = device.generate_allocator_report() else {
            self.push(kind, AllocSnap::default(), Vec::new(), Vec::new());
            return;
        };
        let mut cur = HashSet::with_capacity(report.allocations.len());
        let mut staging = (0u64, 0u64);
        for (i, a) in report.allocations.iter().enumerate() {
            cur.insert((
                a.name.clone(),
                a.size,
                a.offset,
                block_of(&report.blocks, i),
            ));
            if a.name == STAGING_NAME {
                staging.0 += 1;
                staging.1 += a.size;
            }
        }
        let new: Vec<AllocDelta> = report
            .allocations
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                !self.last_allocs.contains(&(
                    a.name.clone(),
                    a.size,
                    a.offset,
                    block_of(&report.blocks, *i),
                ))
            })
            .map(|(i, a)| AllocDelta {
                name: a.name.clone(),
                size: a.size,
                offset: a.offset,
                block: block_of(&report.blocks, i),
            })
            .collect();
        let freed: Vec<AllocDelta> = self
            .last_allocs
            .iter()
            .filter(|k| !cur.contains(*k))
            .map(|(name, size, offset, block)| AllocDelta {
                name: name.clone(),
                size: *size,
                offset: *offset,
                block: *block,
            })
            .collect();
        self.last_allocs = cur;
        let blocks = report.blocks.len() as u64;
        // Per-label live totals are attached at block transitions and
        // frame boundaries — the points the analysis needs implicated
        // labels for — not on every event.
        let detailed = blocks != self.last_blocks
            || matches!(kind, EventKind::FrameBegin | EventKind::FrameEnd { .. });
        self.last_blocks = blocks;
        let by_name = detailed.then(|| {
            let mut names: HashMap<&str, (u64, u64)> = HashMap::new();
            for a in &report.allocations {
                let e = names.entry(a.name.as_str()).or_default();
                e.0 += 1;
                e.1 += a.size;
            }
            let mut v: Vec<(String, u64, u64)> = names
                .into_iter()
                .map(|(k, (c, b))| (k.to_string(), c, b))
                .collect();
            v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
            v
        });
        let snap = AllocSnap {
            allocated: report.total_allocated_bytes,
            reserved: report.total_reserved_bytes,
            allocations: report.allocations.len() as u64,
            blocks,
            block_sizes: report.blocks.iter().map(|b| b.size).collect(),
            staging,
            by_name,
        };
        self.push(kind, snap, new, freed);
    }

    fn push(
        &mut self,
        kind: EventKind,
        alloc: AllocSnap,
        new: Vec<AllocDelta>,
        freed: Vec<AllocDelta>,
    ) {
        while let Some(&(used, _)) = self.retired.front() {
            if used <= self.completed_submission {
                self.retired.pop_front();
            } else {
                break;
            }
        }
        let retired_in_flight = self.retired.iter().map(|&(_, b)| b).sum();
        self.events.push(AllocEvent {
            seq: self.seq,
            frame: self.frame,
            surface: self.surface,
            phase: self.phase,
            kind,
            alloc,
            new,
            freed,
            live_bytes: self.live_bytes,
            retired_in_flight,
            last_submission: self.last_submission,
            completed_submission: self.completed_submission,
        });
        self.seq += 1;
    }

    fn retire(&mut self, device: &wgpu::Device, ev: RetireArgs) {
        let RetireArgs {
            label,
            class,
            bytes,
            used_in_latest_submit,
            reason,
        } = ev;
        self.live_bytes = self.live_bytes.saturating_sub(bytes);
        let last_used = used_in_latest_submit.then_some(self.last_submission);
        if bytes > 0 {
            self.retired.push_back((self.last_submission, bytes));
        }
        self.event(
            device,
            EventKind::Retire {
                label,
                class,
                bytes,
                last_used,
                reason,
            },
        );
    }
}

/// Arguments to [`Inner::retire`], grouped to keep call sites short.
#[derive(Clone, Copy)]
pub(crate) struct RetireArgs {
    pub(crate) label: &'static str,
    pub(crate) class: Class,
    pub(crate) bytes: u64,
    pub(crate) used_in_latest_submit: bool,
    pub(crate) reason: &'static str,
}

/// A shared sink the renderer records into and the caller drains.
///
/// `Sink::new` hands a handle to `GpuConfig::alloc_diag`; after the run,
/// [`Sink::take`] reads the trace. The handle is `Send + Sync` and may
/// be cloned; one engine writes it.
#[derive(Clone)]
pub struct Sink {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.inner.lock().map_or(0, |i| i.events.len());
        f.debug_struct("diag::Sink")
            .field("events", &count)
            .finish()
    }
}

impl Default for Sink {
    fn default() -> Self {
        Self::new()
    }
}

impl Sink {
    /// An empty trace.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                events: Vec::new(),
                seq: 0,
                frame: None,
                surface: None,
                phase: "init",
                last_submission: 0,
                completed_submission: 0,
                pending_submits: VecDeque::new(),
                last_allocs: HashSet::new(),
                live_bytes: 0,
                retired: VecDeque::new(),
                last_blocks: 0,
            })),
        }
    }

    /// Drains the recorded events.
    ///
    /// # Panics
    /// When the sink's lock is poisoned.
    #[must_use]
    pub fn take(&self) -> Vec<AllocEvent> {
        std::mem::take(&mut self.inner.lock().expect("diag poisoned").events)
    }

    /// How many events are recorded so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |i| i.events.len())
    }

    /// Whether the trace is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A final snapshot event after teardown, so the trace ends with
    /// the post-drop allocator state.
    ///
    /// # Panics
    /// When the sink's lock is poisoned.
    pub fn teardown(&self, device: &wgpu::Device) {
        self.inner
            .lock()
            .expect("diag poisoned")
            .event(device, EventKind::Phase { name: "teardown" });
    }
}

thread_local! {
    #[allow(
        clippy::missing_const_for_thread_local,
        reason = "the initializer is already const; false positive on clippy 1.98"
    )]
    static ACTIVE: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

/// Installs `sink` until the guard drops. The renderer takes a guard at
/// every entry point that performs GPU work.
pub(crate) struct Guard(Option<Sink>);

impl Guard {
    /// Installs `sink`, restoring the previous one on drop.
    pub(crate) fn scope(sink: Option<&Sink>) -> Self {
        ACTIVE.with(|slot| Self(slot.replace(sink.cloned())))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|slot| {
            let _ = slot.replace(self.0.take());
        });
    }
}

/// Records `kind` with an allocator snapshot when a sink is installed.
pub(crate) fn event(device: &wgpu::Device, kind: EventKind) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            sink.inner
                .lock()
                .expect("diag poisoned")
                .event(device, kind);
        }
    });
}

/// Sets the surface tag subsequent events carry.
pub(crate) fn set_surface(surface: Option<u64>) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            sink.inner.lock().expect("diag poisoned").surface = surface;
        }
    });
}

/// Sets the phase tag subsequent events carry.
pub(crate) fn set_phase(phase: &'static str) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            sink.inner.lock().expect("diag poisoned").phase = phase;
        }
    });
}

/// A create boundary.
pub(crate) fn create(device: &wgpu::Device, label: &'static str, bytes: u64) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            let mut inner = sink.inner.lock().expect("diag poisoned");
            inner.live_bytes += bytes;
            inner.event(
                device,
                EventKind::Create {
                    label,
                    class: Class::for_label(label),
                    bytes,
                },
            );
        }
    });
}

/// A grow boundary: `old` bytes retire, `new` bytes are created.
pub(crate) fn grow(
    device: &wgpu::Device,
    label: &'static str,
    class: Class,
    old: u64,
    new: u64,
    preserve: u64,
    used_in_latest_submit: bool,
) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            let mut inner = sink.inner.lock().expect("diag poisoned");
            if old > 0 {
                inner.retire(
                    device,
                    RetireArgs {
                        label,
                        class,
                        bytes: old,
                        used_in_latest_submit,
                        reason: "grow",
                    },
                );
            }
            inner.live_bytes += new;
            inner.event(
                device,
                EventKind::Grow {
                    label,
                    class,
                    old,
                    new,
                    preserve,
                },
            );
        }
    });
}

/// A retire boundary: the engine dropped a resource it held.
pub(crate) fn retire(device: &wgpu::Device, args: RetireArgs) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            sink.inner
                .lock()
                .expect("diag poisoned")
                .retire(device, args);
        }
    });
}

/// An upload boundary (`write_buffer`/`write_texture`).
pub(crate) fn upload(
    device: &wgpu::Device,
    label: &'static str,
    bytes: u64,
    rect: Option<(u32, u32, u32, u32)>,
) {
    event(
        device,
        EventKind::Upload {
            label,
            class: Class::for_label(label),
            bytes,
            rect,
        },
    );
}

/// A submission boundary: bumps the renderer's submission ordinal and
/// registers a completion callback so later events can distinguish
/// in-flight retired bytes.
pub(crate) fn submit(device: &wgpu::Device, queue: &wgpu::Queue, encoder: &'static str) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            {
                let mut inner = sink.inner.lock().expect("diag poisoned");
                inner.last_submission += 1;
                let index = inner.last_submission;
                inner.pending_submits.push_back(index);
                inner.event(device, EventKind::Submit { index, encoder });
            }
            // Registration follows the event so the callback, which locks
            // the sink, cannot run inside it. Callbacks fire in submission
            // order; each one retires the oldest pending ordinal.
            let inner = Arc::clone(&sink.inner);
            queue.on_submitted_work_done(move || {
                if let Ok(mut inner) = inner.lock()
                    && let Some(done) = inner.pending_submits.pop_front()
                {
                    inner.completed_submission = inner.completed_submission.max(done);
                }
            });
        }
    });
}

/// A map boundary (`map_async` request).
pub(crate) fn map(device: &wgpu::Device, label: &'static str, bytes: u64) {
    event(device, EventKind::Map { label, bytes });
}

/// A poll/wait boundary.
pub(crate) fn poll(device: &wgpu::Device, wait_finished: bool) {
    event(device, EventKind::Poll { wait_finished });
}

/// A bind-group drop boundary.
pub(crate) fn bind_groups_dropped(device: &wgpu::Device, dropped: u64, reason: &'static str) {
    event(device, EventKind::BindGroups { dropped, reason });
}

/// A frame boundary. `begin` tags subsequent events with `frame`;
/// `end` clears the tag.
pub(crate) fn frame_boundary(device: &wgpu::Device, frame: u64, begin: bool, drew: bool) {
    ACTIVE.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            let mut inner = sink.inner.lock().expect("diag poisoned");
            inner.frame = Some(frame);
            inner.event(
                device,
                if begin {
                    EventKind::FrameBegin
                } else {
                    EventKind::FrameEnd { drew }
                },
            );
            if !begin {
                inner.frame = None;
            }
        }
    });
}

/// An atlas cell placement without an upload (zero-sized cells).
pub(crate) fn atlas_cell(device: &wgpu::Device, rect: (u32, u32, u32, u32)) {
    event(device, EventKind::AtlasCell { rect });
}

fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn opt_u64(v: Option<u64>, out: &mut String) {
    match v {
        Some(v) => out.push_str(&v.to_string()),
        None => out.push_str("null"),
    }
}

fn class_json(class: Class, out: &mut String) {
    out.push_str(match class {
        Class::Buffer => "\"buffer\"",
        Class::MapBuffer => "\"map_buffer\"",
        Class::Query => "\"query\"",
        Class::Atlas => "\"atlas\"",
        Class::MaskTexture => "\"mask_texture\"",
        Class::Target => "\"target\"",
        Class::Image => "\"image\"",
        Class::Other => "\"other\"",
    });
}

fn delta_json(d: &AllocDelta, out: &mut String) {
    out.push_str("{\"name\":");
    json_escape(&d.name, out);
    out.push_str(",\"size\":");
    out.push_str(&d.size.to_string());
    out.push_str(",\"offset\":");
    out.push_str(&d.offset.to_string());
    out.push_str(",\"block\":");
    out.push_str(&d.block.to_string());
    out.push('}');
}

fn snap_json(s: &AllocSnap, out: &mut String) {
    out.push_str("{\"allocated\":");
    out.push_str(&s.allocated.to_string());
    out.push_str(",\"reserved\":");
    out.push_str(&s.reserved.to_string());
    out.push_str(",\"allocations\":");
    out.push_str(&s.allocations.to_string());
    out.push_str(",\"blocks\":");
    out.push_str(&s.blocks.to_string());
    out.push_str(",\"block_sizes\":[");
    for (i, b) in s.block_sizes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&b.to_string());
    }
    out.push_str("],\"staging\":{\"count\":");
    out.push_str(&s.staging.0.to_string());
    out.push_str(",\"bytes\":");
    out.push_str(&s.staging.1.to_string());
    out.push('}');
    if let Some(by_name) = &s.by_name {
        out.push_str(",\"by_name\":[");
        for (i, (name, count, bytes)) in by_name.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"name\":");
            json_escape(name, out);
            out.push_str(",\"count\":");
            out.push_str(&count.to_string());
            out.push_str(",\"bytes\":");
            out.push_str(&bytes.to_string());
            out.push('}');
        }
        out.push(']');
    }
    out.push('}');
}

#[expect(
    clippy::too_many_lines,
    reason = "a flat serializer; every arm writes one variant's object"
)]
fn kind_json(kind: &EventKind, out: &mut String) {
    out.push_str(match kind {
        EventKind::FrameBegin => "{\"frame_begin\":{}}",
        EventKind::FrameEnd { drew } => {
            return {
                out.push_str("{\"frame_end\":{\"drew\":");
                out.push_str(if *drew { "true" } else { "false" });
                out.push_str("}}");
            };
        }
        EventKind::Phase { name } => {
            return {
                out.push_str("{\"phase\":{\"name\":");
                json_escape(name, out);
                out.push_str("}}");
            };
        }
        EventKind::Create {
            label,
            class,
            bytes,
        } => {
            return resource_json("create", label, *class, out, |out| {
                out.push_str(",\"bytes\":");
                out.push_str(&bytes.to_string());
            });
        }
        EventKind::Grow {
            label,
            class,
            old,
            new,
            preserve,
        } => {
            return resource_json("grow", label, *class, out, |out| {
                out.push_str(",\"old\":");
                out.push_str(&old.to_string());
                out.push_str(",\"new\":");
                out.push_str(&new.to_string());
                out.push_str(",\"preserve\":");
                out.push_str(&preserve.to_string());
            });
        }
        EventKind::Retire {
            label,
            class,
            bytes,
            last_used,
            reason,
        } => {
            return resource_json("retire", label, *class, out, |out| {
                out.push_str(",\"bytes\":");
                out.push_str(&bytes.to_string());
                out.push_str(",\"last_used\":");
                opt_u64(*last_used, out);
                out.push_str(",\"reason\":");
                json_escape(reason, out);
            });
        }
        EventKind::Upload {
            label,
            class,
            bytes,
            rect,
        } => {
            return resource_json("upload", label, *class, out, |out| {
                out.push_str(",\"bytes\":");
                out.push_str(&bytes.to_string());
                if let Some((x, y, w, h)) = rect {
                    out.push_str(",\"rect\":[");
                    let _ = write!(out, "{x},{y},{w},{h}]");
                }
            });
        }
        EventKind::Submit { index, encoder } => {
            return {
                out.push_str("{\"submit\":{\"index\":");
                out.push_str(&index.to_string());
                out.push_str(",\"encoder\":");
                json_escape(encoder, out);
                out.push_str("}}");
            };
        }
        EventKind::Map { label, bytes } => {
            return {
                out.push_str("{\"map\":{\"label\":");
                json_escape(label, out);
                out.push_str(",\"bytes\":");
                out.push_str(&bytes.to_string());
                out.push_str("}}");
            };
        }
        EventKind::Poll { wait_finished } => {
            return {
                out.push_str("{\"poll\":{\"wait_finished\":");
                out.push_str(if *wait_finished { "true" } else { "false" });
                out.push_str("}}");
            };
        }
        EventKind::BindGroups { dropped, reason } => {
            return {
                out.push_str("{\"bind_groups\":{\"dropped\":");
                out.push_str(&dropped.to_string());
                out.push_str(",\"reason\":");
                json_escape(reason, out);
                out.push_str("}}");
            };
        }
        EventKind::AtlasCell { rect } => {
            return {
                let (x, y, w, h) = rect;
                out.push_str("{\"atlas_cell\":{\"rect\":[");
                let _ = write!(out, "{x},{y},{w},{h}]}}");
                out.push('}');
            };
        }
    });
}

fn resource_json(
    tag: &str,
    label: &str,
    class: Class,
    out: &mut String,
    rest: impl Fn(&mut String),
) {
    out.push('{');
    json_escape(tag, out);
    out.push_str(":{\"label\":");
    json_escape(label, out);
    out.push_str(",\"class\":");
    class_json(class, out);
    rest(out);
    out.push_str("}}");
}

fn event_json(ev: &AllocEvent, out: &mut String) {
    out.push_str("{\"seq\":");
    out.push_str(&ev.seq.to_string());
    out.push_str(",\"frame\":");
    opt_u64(ev.frame, out);
    out.push_str(",\"surface\":");
    opt_u64(ev.surface, out);
    out.push_str(",\"phase\":");
    json_escape(ev.phase, out);
    out.push_str(",\"kind\":");
    kind_json(&ev.kind, out);
    out.push_str(",\"alloc\":");
    snap_json(&ev.alloc, out);
    out.push_str(",\"new\":[");
    for (i, d) in ev.new.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        delta_json(d, out);
    }
    out.push_str("],\"freed\":[");
    for (i, d) in ev.freed.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        delta_json(d, out);
    }
    out.push_str("],\"live_bytes\":");
    out.push_str(&ev.live_bytes.to_string());
    out.push_str(",\"retired_in_flight\":");
    out.push_str(&ev.retired_in_flight.to_string());
    out.push_str(",\"last_submission\":");
    out.push_str(&ev.last_submission.to_string());
    out.push_str(",\"completed_submission\":");
    out.push_str(&ev.completed_submission.to_string());
    out.push('}');
}

impl Sink {
    /// Writes the recorded events as JSON Lines (one event per line) to
    /// `path`, returning how many were written.
    ///
    /// # Errors
    /// [`std::io::Error`] when the file cannot be created or written.
    pub fn write_json(&self, path: &std::path::Path) -> std::io::Result<usize> {
        write_events(&self.take(), path)
    }
}

/// Writes `events` as one JSON object per line (#169 A5 splits the
/// take/serialize pair so callers can scan the same events).
///
/// # Errors
/// Any write failure on `path`.
pub fn write_events(events: &[AllocEvent], path: &std::path::Path) -> std::io::Result<usize> {
    use std::io::Write as _;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut line = String::with_capacity(1024);
    for ev in events {
        line.clear();
        event_json(ev, &mut line);
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
    }
    file.flush()?;
    Ok(events.len())
}

//! The platform-view embedding contract: a [`PlatformView`] leaf marks where a
//! real platform widget mounts inside the retained tree, and the host that
//! embeds native children mirrors the published placements.
//!
//! `Native<PlatformView>` draws nothing itself — every flush its frame (in
//! window-space logical units), the effective clip, and its paint-order index
//! are recorded onto the [`PlatformViewSink`] the runner installed into the
//! window environment. A host (the Android runner today) turns each placement
//! into a mounted platform child: z-order between native children comes from
//! `order`, the clip keeps a scrolled-off child inside its scroll viewport,
//! and the mounted child owns the input landing in its rect — the host's own
//! content never receives it.
//!
//! A `PlatformView` leaf reaching a runner that installed no sink panics at
//! node build, naming the missing piece — there is no stand-in rendering for
//! a native child.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use waterui_core::NativeView;
use waterui_core::layout::StretchAxis;

/// A request to mount a real platform widget at this leaf's frame.
///
/// `kind` is the registry key the host resolves a factory for — on Android a
/// `PlatformViewRegistry.registerFactory(kind) { context -> View }`
/// registration. Kinds are application vocabulary (`"map"`, `"video"`,
/// `"webview"`, …); this type carries no platform widget of its own.
///
/// The leaf stretches on both axes: mount it with a frame/weight modifier to
/// bound it.
#[derive(Debug)]
pub struct PlatformView {
    kind: Box<str>,
}

impl PlatformView {
    /// A platform-view leaf of the named kind — wrap it in
    /// `waterui_core::components::native::Native::new` to place it in a view
    /// tree.
    pub fn new(kind: impl Into<Box<str>>) -> Self {
        Self { kind: kind.into() }
    }

    /// The registry key the host resolves this leaf's factory through.
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }
}

impl NativeView for PlatformView {
    /// A mounted platform child fills its proposal, like a `GpuSurface`.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Stable ids for placements — allocated once per retained node, kept across
/// relayouts so a resize re-places the same mounted view instead of remounting
/// it. A structural replacement (a `Dynamic` rebuild) mints a fresh id.
pub fn next_platform_view_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Where one embedded platform view sits, in the window's logical coordinate
/// space.
///
/// The host converts to its own units (the Android host scales to
/// physical px in Kotlin with the same density the input edge divides by).
// Serialization exists only on hosts that push placements off-platform as
// JSON — Android JNI today; serde is a target-scoped dependency.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    any(target_os = "android", target_arch = "wasm32"),
    derive(serde::Serialize)
)]
pub struct PlatformViewPlacement {
    /// The retained node's stable placement id.
    pub id: u64,
    /// The factory key from [`PlatformView::new`].
    pub kind: Box<str>,
    /// The leaf's laid-out x origin under this frame's transforms.
    pub x: f32,
    /// The leaf's laid-out y origin under this frame's transforms.
    pub y: f32,
    /// The leaf's laid-out width under this frame's transforms.
    pub width: f32,
    /// The leaf's laid-out height under this frame's transforms.
    pub height: f32,
    /// The effective clip the frame's paint layers impose (scroll viewports,
    /// `.clip()`), as `[x, y, width, height]` — present only when it actually
    /// cuts the frame.
    #[cfg_attr(
        any(target_os = "android", target_arch = "wasm32"),
        serde(skip_serializing_if = "Option::is_none")
    )]
    pub clip: Option<[f32; 4]>,
    /// Paint order among this frame's interactive content: a larger `order`
    /// is later — the host stacks embedded children by it.
    pub order: u32,
    /// Whether any of the frame remains visible inside its clip.
    pub visible: bool,
}

/// The placements one retained tree produced — the single source of truth a
/// host's platform-view registry mirrors.
///
/// A flush *records* the current frame's set; [`Self::publish`] promotes it as
/// the authoritative set and marks the table dirty when it differs. Hosts
/// publish once after a frame's encode completed and only then — recording
/// without publishing keeps the previous set, which is what an idle frame
/// (no flush ran) must report.
#[derive(Debug, Default)]
pub struct PlatformViewTable {
    /// Placements recorded since the last [`Self::publish`].
    current: Vec<PlatformViewPlacement>,
    /// The authoritative set — what the host is showing.
    published: Vec<PlatformViewPlacement>,
    dirty: bool,
}

impl PlatformViewTable {
    /// Records a leaf's frame during the current flush.
    pub fn record(&mut self, placement: PlatformViewPlacement) {
        self.current.push(placement);
    }

    /// Ends a flush: promote the recorded set. Call exactly once per encoded
    /// frame — a frame that encoded no platform-view leaf drops its entry.
    pub fn publish(&mut self) {
        if self.current != self.published {
            self.dirty = true;
            std::mem::swap(&mut self.current, &mut self.published);
        }
        self.current.clear();
    }

    /// The set the host should currently mount.
    pub fn placements(&self) -> &[PlatformViewPlacement] {
        &self.published
    }

    /// Whether the published set changed since the last `take_dirty`.
    pub const fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }
}

/// The environment value installing a [`PlatformViewTable`] into a window:
/// a host inserts one at session create and [`PlatformView`] leaves record
/// their frames into it.
///
/// A leaf built under an environment without a sink panics at build —
/// embedding platform views is a host capability, not a renderer default.
#[derive(Clone, Debug)]
pub struct PlatformViewSink {
    pub(crate) table: Rc<RefCell<PlatformViewTable>>,
}

impl PlatformViewSink {
    /// A sink backed by a fresh empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table: Rc::new(RefCell::new(PlatformViewTable::default())),
        }
    }

    /// The table the host mirrors — read it after a frame's `publish`.
    #[must_use]
    pub const fn table(&self) -> &Rc<RefCell<PlatformViewTable>> {
        &self.table
    }
}

impl Default for PlatformViewSink {
    fn default() -> Self {
        Self::new()
    }
}

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

#[cfg(target_os = "android")]
use std::cell::Cell;
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

/// What a placement asks the host to mount.
///
/// A placement either names a *factory* — the `kind` an application
/// registered a `Context -> View` builder for — or an *instance* — the
/// registration id of a native view that already exists and the host mounts
/// where the leaf lays out. The instance form is how a backend-driven
/// platform widget (the Android system `WebView`) reaches the overlay: the
/// native side created the view itself, so there is no factory for the host
/// to call.
// Serialization exists only on hosts that push placements off-platform as
// JSON — Android JNI today; serde is a target-scoped dependency — and in
// tests, which exercise the wire shape on the host. Untagged so a placement
// serializes flat: `{"kind":"map",…}` or `{"instance":7,…}`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    any(target_os = "android", target_arch = "wasm32", test),
    derive(serde::Serialize),
    serde(untagged)
)]
pub enum PlatformViewSource {
    /// Resolve the named factory and mount the view it builds.
    Factory {
        /// The registry key from [`PlatformView::new`].
        kind: Box<str>,
    },
    /// Mount the native view the host registered under this id.
    Instance {
        /// The host-side registration id of the already-constructed view.
        instance: u64,
    },
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
    any(test, target_os = "android", target_arch = "wasm32"),
    derive(serde::Serialize)
)]
pub struct PlatformViewPlacement {
    /// The retained node's stable placement id.
    pub id: u64,
    /// What the host mounts for this leaf — a factory `kind` or a registered
    /// `instance`. Flattened so the JSON reads as before for factories.
    #[cfg_attr(
        any(test, target_os = "android", target_arch = "wasm32"),
        serde(flatten)
    )]
    pub source: PlatformViewSource,
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
        any(test, target_os = "android", target_arch = "wasm32"),
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
    /// Writes the frame's complete set wholesale — the retained path's
    /// frame-end record is idempotent: however many frame-end sites run
    /// in one presented frame (build plus refresh-after-build, say), each
    /// leaves `current` holding exactly the staged placements.
    pub fn record_frame(&mut self, placements: Vec<PlatformViewPlacement>) {
        self.current = placements;
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

/// How many mounted platform-view children currently hold UI focus — the
/// system `WebView` editing inside the page. While set a mounted child
/// owns the IME: the runner clears the `WaterUI` text-input focus (a Hydrolysis
/// field must not claim focus it does not hold) and drives no show/hide of
/// its own — the child manages the keyboard itself.
///
/// One boolean, not a count: the Kotlin `PlatformViewRegistry`'s single
/// focus listener computes "focus sits inside the container" and reports it
/// here — a removed child's focus move reports its own loss, so nothing can
/// leak the way a per-view count could. Only Android realizes mounted
/// children today, so the type builds there.
#[cfg(target_os = "android")]
#[derive(Clone, Debug, Default)]
pub struct PlatformViewFocus {
    holding: Rc<Cell<bool>>,
    changed: Rc<Cell<bool>>,
}

#[cfg(target_os = "android")]
impl PlatformViewFocus {
    /// The host's "focus is inside a platform-view container" report —
    /// idempotent. `true` is returned when the boolean flipped, so the
    /// caller can request the frame that consumes the edge.
    pub fn set(&self, focused: bool) -> bool {
        if self.holding.replace(focused) != focused {
            self.changed.set(true);
            return true;
        }
        false
    }

    /// The edge flag the frame pump consumes, exactly once.
    pub fn take_changed(&self) -> bool {
        self.changed.replace(false)
    }

    /// Whether a platform-view child currently holds UI focus.
    pub fn is_holding(&self) -> bool {
        self.holding.get()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The placement source's wire shape: a factory placement keeps its
    /// `{"kind": ...}` JSON, an instance placement carries `{"instance": id}`
    /// — flattened so the two read as one schema to the Kotlin registry.
    #[test]
    fn placement_source_wire_shape() {
        let factory =
            serde_json::to_value(PlatformViewSource::Factory { kind: "map".into() }).unwrap();
        assert_eq!(factory, serde_json::json!({ "kind": "map" }));

        let instance = serde_json::to_value(PlatformViewSource::Instance { instance: 7 }).unwrap();
        assert_eq!(instance, serde_json::json!({ "instance": 7 }));
    }

    /// A whole placement on the wire — the object the Kotlin registry's
    /// `createSlot` parses. Factory and instance leaves must read as the
    /// same shape except for the flattened source field.
    #[test]
    fn placement_wire_shape() {
        let base = serde_json::json!({
            "x": 1.0,
            "y": 2.0,
            "width": 3.0,
            "height": 4.0,
            "order": 5,
            "visible": true,
        });
        let placement = |source| PlatformViewPlacement {
            id: 9,
            source,
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            clip: None,
            order: 5,
            visible: true,
        };
        let mut factory = base.clone();
        factory["id"] = serde_json::json!(9);
        factory["kind"] = serde_json::json!("map");
        assert_eq!(
            serde_json::to_value(placement(PlatformViewSource::Factory {
                kind: "map".into()
            }))
            .unwrap(),
            factory
        );
        let mut instance = base;
        instance["id"] = serde_json::json!(9);
        instance["instance"] = serde_json::json!(7);
        assert_eq!(
            serde_json::to_value(placement(PlatformViewSource::Instance { instance: 7 })).unwrap(),
            instance
        );
    }
}

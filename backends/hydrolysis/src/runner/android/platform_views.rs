//! The platform-view placement table the Kotlin `PlatformViewRegistry`
//! mirrors.
//!
//! A native view embedded in the host view's band layout is placed by an
//! entry here — the table is the single source of truth for where the
//! backing view sits relative to the GPU band, in host-view physical px.
//! The deep native-embedding pipeline that produces placements is a later
//! step of the plan; the contract this module fixes is the transport:
//! placements arrive as one JSON frame set per change, addressed by view
//! id, so a provider never tracks per-view deltas itself.

use super::jni::JniError;

/// One embedded native view's placement, in host-view physical px.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub(crate) struct PlatformViewPlacement {
    /// The provider-assigned view id (stable across relayouts).
    pub(crate) id: u64,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// Whether the view currently draws; an occluded band parks its view
    /// detached rather than placed at zero.
    pub(crate) visible: bool,
}

/// Placements keyed by provider-issued id. Whole-set publishes only: the
/// registry replaces its layout each publish and never merges deltas.
#[derive(Default)]
pub(crate) struct PlatformViewTable {
    placements: Vec<PlatformViewPlacement>,
    dirty: bool,
}

impl PlatformViewTable {
    /// Records or replaces the placement for `id`.
    #[expect(
        dead_code,
        reason = "producers land with the platform-view embedding step of the plan"
    )]
    pub(crate) fn place(&mut self, placement: PlatformViewPlacement) {
        if let Some(existing) = self.placements.iter_mut().find(|p| p.id == placement.id) {
            if *existing != placement {
                *existing = placement;
                self.dirty = true;
            }
            return;
        }
        self.placements.push(placement);
        self.dirty = true;
    }

    /// Removes the placement for `id` — the provider takes its view away.
    #[expect(
        dead_code,
        reason = "producers land with the platform-view embedding step of the plan"
    )]
    pub(crate) fn remove(&mut self, id: u64) {
        if self.placements.iter().any(|p| p.id == id) {
            self.placements.retain(|p| p.id != id);
            self.dirty = true;
        }
    }

    /// Serializes pending placements as a JSON frame set for the registry.
    pub(crate) fn take_json(&mut self) -> Result<Option<String>, JniError> {
        if !self.dirty {
            return Ok(None);
        }
        self.dirty = false;
        serde_json::to_string(&self.placements)
            .map(Some)
            .map_err(|error| {
                JniError(format!(
                    "hydrolysis android: platform-view placements failed to serialize: {error}"
                ))
            })
    }
}

/// Publishes a changed placement set — currently a table-drain; the
/// native-embedding producers land in a later step of the plan and write
/// through `place`/`remove`.
pub(crate) fn publish_if_pending(_session: &mut super::host::AndroidSession) {}

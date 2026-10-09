//! Platform text layout ids, shared by the encoder and every layout.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use waterui_graphics::draw::kurbo::Rect;

use crate::hwui::HwuiError;
use crate::hwui::buffer::{CommandBuffer, Field};
use crate::hwui::protocol::Op;
use crate::hwui::resources::Table;

/// The dense ids of platform text layouts.
///
/// Layouts are shaped and dropped on whichever thread the text engine runs,
/// so unlike the encoder's `Registry` the table is
/// shared. Its releases follow the same discipline: written into the first
/// frame no installed content draws the layout in, recycled when the frame
/// after it drains.
#[derive(Clone, Debug, Default)]
pub struct TextLayoutIds(Arc<Mutex<State>>);

#[derive(Debug)]
struct State {
    table: Table,
    /// Each live layout's node bounds, by id.
    bounds: Vec<Option<Rect>>,
    pending: Vec<u32>,
    retired: Vec<u32>,
}

impl State {
    /// The wire id of `raw` while content may draw it: live, or released
    /// with its release not yet written.
    fn drawable(&self, raw: u64) -> Option<u32> {
        self.table.is_live(raw).or_else(|| {
            u32::try_from(raw)
                .ok()
                .filter(|id| self.pending.contains(id))
        })
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            table: Table::new("text layout"),
            bounds: Vec::new(),
            pending: Vec::new(),
            retired: Vec::new(),
        }
    }
}

impl TextLayoutIds {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // Every critical section leaves the state consistent, so a panic
    // elsewhere that poisoned the lock left nothing half-written.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A fresh id for a layout about to be shaped.
    ///
    /// # Errors
    ///
    /// [`HwuiError::IdsExhausted`] when every id is live.
    pub fn acquire(&self) -> Result<u32, HwuiError> {
        self.lock().table.acquire()
    }

    /// Queues the release of layout `id` into the next frame.
    pub fn release(&self, id: u32) {
        let mut state = self.lock();
        if state.table.retire(id) {
            state.pending.push(id);
        }
    }

    /// Records the bounds layout `id`'s node draws in, once shaped.
    pub(in crate::hwui) fn register(&self, id: u32, bounds: Rect) {
        let mut state = self.lock();
        let slot = id as usize;
        if state.bounds.len() <= slot {
            state.bounds.resize(slot + 1, None);
        }
        state.bounds[slot] = Some(bounds);
    }

    /// The node bounds of layout `raw`, while content may draw it.
    #[must_use]
    pub fn bounds(&self, raw: u64) -> Option<Rect> {
        let state = self.lock();
        let bounds = state
            .drawable(raw)
            .and_then(|id| state.bounds.get(id as usize).copied().flatten());
        drop(state);
        bounds
    }

    /// Returns `id`, whose platform layout was never registered, straight
    /// to the pool: there is nothing for a release to drop.
    pub(in crate::hwui) fn forget(&self, id: u32) {
        let mut state = self.lock();
        if state.table.retire(id) {
            state.table.recycle(id);
        }
    }

    /// The wire id of live layout `raw`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Unregistered`] naming `layer` when no live layout has
    /// that id.
    pub fn resolve(&self, raw: u64, layer: u64) -> Result<u32, HwuiError> {
        let drawable = self.lock().drawable(raw);
        drawable.ok_or(HwuiError::Unregistered {
            layer,
            kind: "text layout",
            id: raw,
        })
    }

    /// Recycles the ids the previous frame released and writes into
    /// `buffer` the queued releases of the layouts no installed content
    /// draws; `drawn` says whether some does. The others stay queued.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Encoding`] from the buffer.
    pub(in crate::hwui) fn drain_releases(
        &self,
        buffer: &mut CommandBuffer,
        drawn: impl Fn(u64) -> bool,
    ) -> Result<(), HwuiError> {
        let mut state = self.lock();
        let State {
            table,
            bounds,
            pending,
            retired,
        } = &mut *state;
        for id in retired.drain(..) {
            table.recycle(id);
            if let Some(slot) = bounds.get_mut(id as usize) {
                *slot = None;
            }
        }
        let mut index = 0;
        while index < pending.len() {
            let id = pending[index];
            if drawn(u64::from(id)) {
                index += 1;
            } else {
                buffer.op(Op::ReleaseTextLayout, &[Field::U("id", id)])?;
                retired.push(pending.remove(index));
            }
        }
        drop(state);
        Ok(())
    }
}

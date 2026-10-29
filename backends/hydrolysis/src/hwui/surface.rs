//! The queue→encoder surface: one retained `Shared` tree's change sets
//! commit into the encoder, whose `frame` hands the session a replayable
//! command buffer. Hiding a surface does not tear the tree down — pending
//! commits stash inline and one wake on restore draws them.

use std::cell::RefCell;
use std::rc::Rc;

use cherenkov_record::{ChangeSet, Instant, Queue, Shared, SurfaceId};

use super::{Encoder, Frame, HostEntry, HwuiError, HwuiQueue, HwuiTarget};

/// One retained HWUI surface: the `Shared` change source and the encoder
/// its commits feed.
#[derive(Debug)]
pub struct HwuiSurface {
    shared: Rc<RefCell<Shared<HwuiTarget>>>,
    queue: HwuiQueue,
    encoder: Encoder,
}

impl HwuiSurface {
    /// A surface `id` owns, queuing redraws through `wake` (typically the
    /// host's `Choreographer` request).
    ///
    /// # Errors
    ///
    /// A description when the empty tree's root node cannot be encoded.
    pub fn new(
        id: SurfaceId,
        api_level: u32,
        wake: impl Fn() + 'static,
    ) -> Result<Self, HwuiError> {
        let encoder = Encoder::new(api_level)?;
        let queue = HwuiQueue::new(wake);
        let shared = Rc::new(RefCell::new(Shared::new(id, queue.clone())));
        Ok(Self {
            shared,
            queue,
            encoder,
        })
    }

    /// The retained tree, for `Shared::root`/`Shared::layer`/`run_transaction`.
    #[must_use]
    pub const fn shared(&self) -> &Rc<RefCell<Shared<HwuiTarget>>> {
        &self.shared
    }

    /// The encoder, for registry and font-id construction.
    #[must_use]
    pub const fn encoder(&self) -> &Encoder {
        &self.encoder
    }

    /// The encoder's op log, for tests asserting what a frame emitted.
    #[cfg(test)]
    pub(in crate::hwui) fn take_log(&mut self) -> Vec<String> {
        self.encoder.take_log()
    }

    /// The top-level draw order the next frame writes.
    pub fn set_host_order(&mut self, order: &[HostEntry]) {
        self.encoder.set_host_order(order);
    }

    /// The window clear the frame writes.
    #[must_use]
    pub fn clear_color(&self) -> Option<u64> {
        self.encoder.clear_color()
    }

    /// The platform views bound into layers' own `RenderNode`s this frame.
    pub fn set_platform_views(&mut self, bindings: &[crate::hwui::PlatformViewBinding]) {
        self.encoder.set_platform_views(bindings);
    }

    /// Whether queued scene changes wait for a frame — the host latches
    /// them into the frame transaction's continuation so an idle session
    /// renders them rather than parking with the queue armed.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.queue.has_pending()
    }

    /// Hides or shows the surface.
    ///
    /// The retained tree and its resources survive a view detach: while
    /// hidden, pending commits stash inline rather than waking the host —
    /// an offscreen surface must not request frames — and `flush` makes any
    /// already-pending changes visible so nothing replays stale content.
    /// The first `frame` after restore commits them; one wake then asks for
    /// it. The host gates actual visibility before calling this.
    pub fn set_hidden(&self, hidden: bool) {
        if self.queue.drains_inline() == hidden {
            return;
        }
        self.queue.set_inline(hidden);
        if hidden {
            self.shared.borrow_mut().flush();
        } else {
            self.queue.wake();
        }
    }

    /// Commits every queued change set and encodes the frame.
    ///
    /// Inline-applied commits (hidden period) land first, in order, then
    /// the current pending set; the encoded frame's `words` borrow the
    /// encoder's buffer, so the host replays it before the next `frame`.
    ///
    /// # Errors
    ///
    /// A failed commit is fatal — the frame is not encoded and there is no
    /// retry or rollback.
    pub fn frame(&mut self, time: Instant, scale: f64) -> Result<Frame<'_>, HwuiError> {
        for changes in self.queue.take_applied() {
            self.commit(changes)?;
        }
        let changes = self.shared.borrow_mut().take_changes(time);
        // Clear before `commit`: any queue demand the commit itself raises —
        // a dropped layer's changes, say — must ride the next frame, not
        // this one's consumed flag.
        let _ = self.queue.take_pending();
        if let Some(changes) = changes {
            self.commit(changes)?;
        }
        self.encoder.frame(time, scale)
    }

    fn commit(&mut self, changes: ChangeSet<HwuiTarget>) -> Result<(), HwuiError> {
        let (ops, mut recycled) = self.encoder.commit(changes)?.into_parts();
        self.shared.borrow_mut().recycle(ops, &mut recycled);
        Ok(())
    }
}

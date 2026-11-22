//! Nonblocking frame submission on the native engine: the frontend state
//! an [`Engine`](super::Engine) shares with the [`PendingFrame`]s it hands
//! out, and the one frame slot that orders them.
//!
//! One `Render` is in flight at a time — the slot is the protocol's frame
//! backpressure. Its reply channel is persistent: a completed frame leaves
//! it free and hands back the drained commit buffers for the next one. A
//! frame dropped before its reply leaves the reply owed; whoever claims the
//! slot next receives that reply on the UI thread, recycles its buffers and
//! drops its presentation commit unapplied. A frame submitted while the
//! slot is taken is queued: it drains its surfaces and posts its `Render`
//! once the slot is free, without ever making its caller wait.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use cherenkov_record::{ChangeSet, SurfaceId};
use waterui_contract::contract_violation;

use crate::backend::{Backend, Renderer};
use crate::error::RenderError;
use crate::frame::{FrameStats, FrameTime, Next};
use crate::message::{Message, RenderReply};
use crate::transport::{Reply, ReplyRx};

/// The engine's UI-thread frame state, shared by the engine and every
/// pending frame: the surfaces a frame drains, the last frame's stats, and
/// the frame slot.
pub(super) struct Frontend<B: Backend> {
    /// The transaction channel a queued frame posts its `Render` on.
    tx: crossbeam_channel::Sender<Message<B>>,
    /// The live surfaces' shared queues, drained into one `Render` per
    /// frame.
    pub(super) surfaces: RefCell<super::Surfaces<B>>,
    /// Statistics of the last applied frame.
    pub(super) stats: RefCell<FrameStats>,
    slot: RefCell<Slot<B>>,
}

/// The unique frame slot: the persistent reply channel, who holds it, and
/// the commit buffers the last frame returned.
struct Slot<B: Backend> {
    reply: Reply<RenderReply<B>>,
    rx: ReplyRx<RenderReply<B>>,
    state: SlotState,
    commits: Vec<(SurfaceId, ChangeSet<B>)>,
    /// Queued frames waiting for the slot. Every change of hands wakes
    /// them all; each re-polls and the first to find the slot claims it.
    queued: Vec<Waker>,
}

impl<B: Backend> Slot<B> {
    /// Hands the slot on: `state` is what the holder leaves behind, and
    /// every queued frame re-polls for it.
    fn release(&mut self, state: SlotState) {
        self.state = state;
        wake_queued(&mut self.queued);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    /// No `Render` is in flight.
    Free,
    /// A live pending frame posted the in-flight `Render`.
    Live,
    /// The in-flight `Render`'s frame was dropped: its reply is owed to
    /// the channel, and the next claimant receives and discards it.
    Owed,
}

impl<B: Backend> Frontend<B> {
    pub(super) fn new(tx: crossbeam_channel::Sender<Message<B>>) -> Self {
        let (reply, rx) = crate::transport::reply();
        Self {
            tx,
            surfaces: RefCell::new(Vec::new()),
            stats: RefCell::new(FrameStats::default()),
            slot: RefCell::new(Slot {
                reply,
                rx,
                state: SlotState::Free,
                commits: Vec::new(),
                queued: Vec::new(),
            }),
        }
    }

    /// Submits the frame at `time`: when the slot is free, drains every
    /// visible surface and posts the `Render` at once; otherwise the frame
    /// is queued and posts it once the slot frees. Never waits.
    pub(super) fn submit(self: &Rc<Self>, time: FrameTime) -> Result<PendingFrame<B>, RenderError> {
        if super::all_hidden(&self.surfaces.borrow()) {
            return Err(RenderError::Hidden);
        }
        let mut frame = PendingFrame {
            frontend: Rc::clone(self),
            stage: Stage::Queued(time),
        };
        if self.try_claim() {
            self.post(time).inspect_err(|_| frame.stage = Stage::Done)?;
            frame.stage = Stage::Sent;
        }
        Ok(frame)
    }

    /// Claims a free slot, or one whose owed reply already arrived.
    fn try_claim(&self) -> bool {
        let mut slot = self.slot.borrow_mut();
        match slot.state {
            SlotState::Free => {}
            SlotState::Owed => {
                let Some(stale) = slot.rx.try_recv() else {
                    return false;
                };
                drop(slot);
                self.discard(stale);
                slot = self.slot.borrow_mut();
            }
            SlotState::Live => return false,
        }
        slot.state = SlotState::Live;
        true
    }

    /// Claims the slot for a queued frame, waiting — through the reply
    /// channel's own wake for an owed reply, through the queue for a live
    /// frame — without blocking.
    fn poll_claim(&self, cx: &mut Context<'_>) -> Poll<()> {
        let mut slot = self.slot.borrow_mut();
        match slot.state {
            SlotState::Free => {}
            SlotState::Owed => match slot.rx.poll_recv(cx) {
                Poll::Ready(Ok(stale)) => {
                    drop(slot);
                    self.discard(stale);
                    slot = self.slot.borrow_mut();
                }
                // The render thread is gone: the claim posts nothing, and
                // the post that follows reports it.
                Poll::Ready(Err(_)) => {}
                Poll::Pending => {
                    queue(&mut slot.queued, cx.waker());
                    return Poll::Pending;
                }
            },
            SlotState::Live => {
                queue(&mut slot.queued, cx.waker());
                return Poll::Pending;
            }
        }
        slot.release(SlotState::Live);
        Poll::Ready(())
    }

    /// Posts the claimed slot's `Render` for `time`: drains every visible
    /// surface's queued changes, re-arming each one's wake before the
    /// render is sent — completions may arrive while it is in flight. A
    /// failed post frees the slot again.
    fn post(&self, time: FrameTime) -> Result<(), RenderError> {
        let mut slot = self.slot.borrow_mut();
        if super::all_hidden(&self.surfaces.borrow()) {
            slot.release(SlotState::Free);
            return Err(RenderError::Hidden);
        }
        let mut commits = std::mem::take(&mut slot.commits);
        commits.clear();
        super::drain_visible(&mut self.surfaces.borrow_mut(), time, &mut commits);
        if let Err(error) = self.tx.send(Message::Render {
            time,
            commits,
            reply: slot.reply.request(),
        }) {
            if let Message::Render { commits, .. } = error.0 {
                slot.commits = commits;
            }
            slot.release(SlotState::Free);
            return Err(RenderError::Thread);
        }
        Ok(())
    }

    /// Polls the live frame's reply.
    fn poll_reply(&self, cx: &mut Context<'_>) -> Poll<Result<RenderReply<B>, RenderError>> {
        let mut slot = self.slot.borrow_mut();
        let reply = std::task::ready!(slot.rx.poll_recv(cx));
        slot.release(SlotState::Free);
        Poll::Ready(reply.map_err(|_| RenderError::Thread))
    }

    /// A live frame was dropped before its reply: the reply stays owed.
    fn abandon(&self) {
        self.slot.borrow_mut().release(SlotState::Owed);
    }

    /// A queued frame was dropped: the frames still queued re-poll, since
    /// the dropped one may have been the poller the reply channel wakes.
    fn leave_queue(&self) {
        wake_queued(&mut self.slot.borrow_mut().queued);
    }

    /// An abandoned frame's reply: its buffers recycle into the slot and
    /// its presentation commit drops unapplied, on the UI thread.
    fn discard(&self, mut stale: RenderReply<B>) {
        self.recycle_commits(&mut stale.commits);
        stale.commits.clear();
        self.slot.borrow_mut().commits = stale.commits;
    }

    /// The live frame's reply: recycles the buffers, applies the frame's
    /// presentation commit on the caller's thread, publishes each
    /// surface's `next_frame` and the stats — exactly once.
    fn finish(&self, mut reply: RenderReply<B>) -> Result<Next, RenderError> {
        self.recycle_commits(&mut reply.commits);
        reply.commits.clear();
        self.slot.borrow_mut().commits = reply.commits;
        let frame = reply.result?;
        // The frame's main-thread work applies here, on the polling
        // thread, before the frame resolves: a window surface's drawable
        // presents land inside this call, ordered before the next frame's
        // acquire — a frame that left surfaces out included. A
        // `RenderError` above drops the commit with its acquired
        // drawables, which a later frame re-acquires.
        <B::Renderer as Renderer>::apply_frame_commit(frame.commit);
        super::publish_next(&self.surfaces.borrow(), &frame.surface_next);
        *self.stats.borrow_mut() = frame.stats;
        super::rendered(frame.next, frame.rejected)
    }

    fn recycle_commits(&self, commits: &mut [(SurfaceId, ChangeSet<B>)]) {
        let surfaces = self.surfaces.borrow();
        for (id, changes) in commits {
            let Some(shared) = surfaces
                .iter()
                .filter_map(|entry| entry.shared.upgrade())
                .find(|shared| shared.borrow().id == *id)
            else {
                continue;
            };
            shared
                .borrow_mut()
                .recycle(std::mem::take(&mut changes.ops), &mut changes.recycled);
        }
    }
}

/// Registers `waker` with the queue once.
fn queue(queued: &mut Vec<Waker>, waker: &Waker) {
    if !queued.iter().any(|queued| queued.will_wake(waker)) {
        queued.push(waker.clone());
    }
}

fn wake_queued(queued: &mut Vec<Waker>) {
    for waker in queued.drain(..) {
        waker.wake();
    }
}

/// A frame [`Engine::submit`](super::Engine::submit) handed to the render
/// thread, resolved on the thread that polls it once the render thread
/// replies.
///
/// The reply channel's own wake resolves it: the waker of the last poll is
/// woken when the reply lands, with no timer and no readback. Resolving
/// applies the frame's presentation commit on the polling thread, then
/// publishes each surface's [`Surface::next_frame`](crate::Surface::next_frame)
/// and the frame's [`Engine::stats`](super::Engine::stats); its output is
/// what [`Engine::render`](super::Engine::render) returns.
///
/// The frame owns its share of the engine's frame state, so a host keeps
/// it beside its surface without a borrow. Dropping it before it resolves
/// discards the frame's presentation without waiting for the render
/// thread: the next submission receives the reply, recycles its buffers
/// and drops its commit unapplied.
///
/// Frames are sent to the render thread one at a time: a frame submitted
/// while another is in flight drains its surfaces and posts its render once
/// that one resolves or is dropped. A host awaits its frames in order; a
/// frame submitted behind a live one that is never polled again waits for
/// it.
#[must_use = "dropping a pending frame discards its presentation"]
pub struct PendingFrame<B: Backend> {
    frontend: Rc<Frontend<B>>,
    stage: Stage,
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    /// Waiting for the slot; posts the `Render` for the time it was
    /// submitted at once it claims it.
    Queued(FrameTime),
    /// The `Render` is in flight.
    Sent,
    /// Resolved, or failed to post.
    Done,
}

impl<B: Backend> std::fmt::Debug for PendingFrame<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingFrame")
            .field("stage", &self.stage)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> Future for PendingFrame<B> {
    type Output = Result<Next, RenderError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        if let Stage::Queued(time) = this.stage {
            std::task::ready!(this.frontend.poll_claim(cx));
            if let Err(error) = this.frontend.post(time) {
                this.stage = Stage::Done;
                return Poll::Ready(Err(error));
            }
            this.stage = Stage::Sent;
        }
        match this.stage {
            Stage::Sent => {
                let reply = std::task::ready!(this.frontend.poll_reply(cx));
                this.stage = Stage::Done;
                Poll::Ready(reply.and_then(|reply| this.frontend.finish(reply)))
            }
            Stage::Done => {
                contract_violation!("a PendingFrame was polled after it resolved");
                Poll::Pending
            }
            Stage::Queued(_) => unreachable!("a queued frame posts or resolves above"),
        }
    }
}

impl<B: Backend> Drop for PendingFrame<B> {
    fn drop(&mut self) {
        match self.stage {
            Stage::Sent => self.frontend.abandon(),
            Stage::Queued(_) => self.frontend.leave_queue(),
            Stage::Done => {}
        }
    }
}

//! Native awaitable reply transport for the engine's request/reply
//! exchanges: an `async-channel` one-reply channel, and the request guard
//! that closes it when the render thread drops or unwinds a request it owes
//! a reply. Every native wait on the render thread — a pending frame,
//! engine and surface creation, the memory query — resolves through it;
//! the synchronous entry points wait on the same futures. wasm keeps its
//! serial executor and its `local` channel; nothing here exists there.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_lite::Stream;

/// A one-reply channel. `send` is `try_send`: a closed channel means the
/// caller dropped its receiver, and a full one means the single-reply
/// invariant was violated — never a reason to wait.
pub struct Reply<T>(async_channel::Sender<T>);

impl<T> Reply<T> {
    /// Posts the reply. `Closed` — the caller cancelled — is expected and
    /// ignored; `Full` is a protocol violation: a single-reply channel
    /// already holding its response means a reply was sent twice.
    pub fn send(&self, value: T) {
        match self.0.try_send(value) {
            Ok(()) | Err(async_channel::TrySendError::Closed(_)) => {}
            Err(async_channel::TrySendError::Full(_)) => {
                panic!("cherenkov: a single-reply channel already contains its response");
            }
        }
    }

    /// One request's hold on the reply channel: the worker wraps the
    /// `Reply` a `Message` carries in this guard, sends through it, and a
    /// success disarms — the channel stays open for the next frame's
    /// request. A dropped or unwinding request instead *closes* the
    /// channel: the frontend's pending `recv` resolves `Err` even though
    /// its own sender lives forever.
    pub fn request(&self) -> FrameRequest<T> {
        FrameRequest {
            reply: Some(self.clone()),
        }
    }

    /// Closes the channel from the sender side: once its queue drains,
    /// every pending or future `recv` resolves `Err` — the render
    /// thread's exit signal, which frontend senders cannot provide.
    pub fn close(&self) {
        self.0.close();
    }
}

impl<T> Clone for Reply<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> std::fmt::Debug for Reply<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reply").finish_non_exhaustive()
    }
}

/// The receiving half of a [`Reply`]. The receiver is pinned once, where it
/// is created: its stream half keeps the listener it registers in place.
pub struct ReplyRx<T>(Pin<Box<async_channel::Receiver<T>>>);

impl<T> ReplyRx<T> {
    /// Waits for the reply; resolves `Err` when every sender dropped — the
    /// render thread's exit closes the channel and wakes the waiter.
    pub async fn recv(&self) -> Result<T, async_channel::RecvError> {
        self.0.recv().await
    }

    /// Polls for the reply: the channel's own listener registers `cx`'s
    /// waker, so the reply's arrival wakes the poller directly. `Err` once
    /// the channel is closed and drained.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<T, async_channel::RecvError>> {
        self.0
            .as_mut()
            .poll_next(cx)
            .map(|reply| reply.ok_or(async_channel::RecvError))
    }

    /// Takes a reply that already arrived, without waiting.
    pub fn try_recv(&self) -> Option<T> {
        self.0.try_recv().ok()
    }
}

/// Opens a one-reply channel.
pub fn reply<T>() -> (Reply<T>, ReplyRx<T>) {
    let (tx, rx) = async_channel::bounded(1);
    (Reply(tx), ReplyRx(Box::pin(rx)))
}

/// One request's hold on a reply channel — see [`Reply::request`].
pub struct FrameRequest<T> {
    reply: Option<Reply<T>>,
}

impl<T> FrameRequest<T> {
    /// Posts the reply and disarms the guard — the channel stays open for
    /// the channel owner's next request.
    pub fn send(mut self, value: T) {
        self.reply.as_ref().expect("live frame request").send(value);
        self.reply.take();
    }
}

impl<T> core::fmt::Debug for FrameRequest<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrameRequest")
            .field("armed", &self.reply.is_some())
            .finish()
    }
}

impl<T> Drop for FrameRequest<T> {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            reply.close();
        }
    }
}

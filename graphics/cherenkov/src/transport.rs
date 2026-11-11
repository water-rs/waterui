//! Native awaitable reply transport for the engine's request/reply
//! exchanges: an `async-channel` one-reply channel per request, the unique
//! reusable frame slot that owns the render response machinery, and the
//! guards that make initialization and surface registration
//! cancellation-safe. wasm keeps its serial executor and its `local`
//! channel; nothing here exists there.

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

/// The receiving half of a [`Reply`].
pub struct ReplyRx<T>(async_channel::Receiver<T>);

impl<T> ReplyRx<T> {
    /// Waits for the reply; resolves `Err` when every sender dropped — the
    /// render thread's exit closes the channel and wakes the waiter.
    pub async fn recv(&self) -> Result<T, async_channel::RecvError> {
        self.0.recv().await
    }

    /// The temporary blocking variant the still-synchronous entry points
    /// use until the final interface lands and they are removed.
    pub fn recv_blocking(&self) -> Result<T, async_channel::RecvError> {
        // `Receiver::recv_blocking` is the channel's own thread-blocking
        // wait, used only by the temporary synchronous bridges.
        self.0.recv_blocking()
    }
}

/// Opens a one-reply channel.
pub fn reply<T>() -> (Reply<T>, ReplyRx<T>) {
    let (tx, rx) = async_channel::bounded(1);
    (Reply(tx), ReplyRx(rx))
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

/// The engine's unique reusable frame slot: the persistent response
/// channel, whether a reply is still owed on it, and the commit buffers
/// a completed frame returns for the next one. One frame is in flight at
/// a time — the slot is the protocol's backpressure, not a tunable quota.
pub struct FrameSlot<B: crate::backend::Backend> {
    /// The response channel, reused across frames.
    pub reply: Reply<crate::message::RenderReply<B>>,
    /// The response channel's receiver — the awaiting caller's half.
    pub reply_rx: async_channel::Receiver<crate::message::RenderReply<B>>,
    /// A reply is owed to this channel — the previous owner's request
    /// completed at the worker but its future was cancelled.
    pub pending: bool,
    /// The reusable commit buffers the last frame returned.
    pub commits: Vec<(cherenkov_record::SurfaceId, cherenkov_record::ChangeSet<B>)>,
}

impl<B: crate::backend::Backend> FrameSlot<B> {
    /// Creates the slot's persistent channel and buffers.
    pub fn new() -> Self {
        let (reply, rx) = reply();
        Self {
            reply,
            reply_rx: rx.0,
            pending: false,
            commits: Vec::new(),
        }
    }
}

impl<B: crate::backend::Backend> Default for FrameSlot<B> {
    fn default() -> Self {
        Self::new()
    }
}

/// A one-element slot pool. `acquire` waits for the slot without blocking
/// the caller's thread; a lease returns on drop — including a cancelled
/// future — and a slot whose reply was abandoned arrives with `pending`
/// still set so the next owner drains the stale response first.
pub struct SlotPool<T> {
    available: async_channel::Receiver<T>,
    returned: async_channel::Sender<T>,
}

impl<T: Default> Default for SlotPool<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> SlotPool<T> {
    /// Creates the pool holding exactly one slot.
    pub fn new(slot: T) -> Self {
        let (returned, available) = async_channel::bounded(1);
        let inserted = returned.try_send(slot);
        assert!(
            inserted.is_ok(),
            "cherenkov: a fresh slot pool rejected its sole slot"
        );
        Self {
            available,
            returned,
        }
    }

    /// Waits for the pool's slot; resolves `Err` when the pool itself is
    /// gone — the engine dropped and no reply will come.
    pub async fn acquire(&self) -> Result<SlotLease<'_, T>, async_channel::RecvError> {
        self.available.recv().await.map(|value| SlotLease {
            pool: self,
            value: Some(value),
        })
    }

    /// The temporary blocking variant the still-synchronous render entry
    /// point uses until the final interface lands and it is removed.
    pub fn acquire_blocking(&self) -> Result<SlotLease<'_, T>, async_channel::RecvError> {
        self.available.recv_blocking().map(|value| SlotLease {
            pool: self,
            value: Some(value),
        })
    }
}

/// The owned slot, returned to its pool on drop.
pub struct SlotLease<'a, T> {
    pool: &'a SlotPool<T>,
    value: Option<T>,
}

impl<T> SlotLease<'_, T> {
    /// The slot's contents.
    pub fn get(&self) -> &T {
        self.value.as_ref().unwrap_or_else(|| unreachable!())
    }
    /// The slot's contents, mutable.
    pub fn get_mut(&mut self) -> &mut T {
        self.value.as_mut().unwrap_or_else(|| unreachable!())
    }
}

impl<T> std::ops::Deref for SlotLease<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T> std::ops::DerefMut for SlotLease<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T> Drop for SlotLease<'_, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.pool
                .returned
                .try_send(value)
                .unwrap_or_else(|_| panic!("cherenkov: reply slot returned twice or pool closed"));
        }
    }
}

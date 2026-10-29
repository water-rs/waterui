//! Content a capture has to wait for.
//!
//! A view mounts synchronously, but some of what it shows arrives later: a
//! photo decodes on a background task, a `Suspense` swaps its content in when
//! a future resolves. A host that rasterizes the view — a preview, a
//! `ViewRenderer` capture, a test snapshot — must not do so while such
//! content is still in flight, or the capture shows an empty slot.
//!
//! The [`ContentLedger`] is how a view tells the capture what it still owes.
//! A capture host installs a fresh ledger in the environment of the subtree
//! it captures; a component that owes content after mount calls
//! [`ContentLedger::owe`] in its `body` and resolves the returned
//! [`OwedContent`] once the content is in place. The host then awaits
//! [`ContentLedger::wait`] between mount and capture.
//!
//! Outside a capture nothing installs a ledger, so [`ContentLedger::owe`]
//! returns `None` and a running app records nothing.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::fmt;
use core::future::{Future, poll_fn};
use core::pin::{Pin, pin};
use core::task::{Context, Poll, Waker};
use core::time::Duration;

use nami::SignalExt as _;

use crate::{Binding, Computed, Environment};

/// The record of content the views of one captured subtree still owe.
///
/// Cloning shares the record. Installed by a capture host into the
/// environment of the subtree it captures — see the [module
/// documentation](self).
#[derive(Clone)]
pub struct ContentLedger(Rc<LedgerState>);

struct LedgerState {
    /// The id the next [`OwedContent`] takes; ids order the entries by
    /// registration.
    next_id: Cell<u64>,
    /// Content owed and not yet resolved, by token id.
    owed: RefCell<BTreeMap<u64, PendingContent>>,
    /// Loads that failed and that no [`Settled`] has reported yet.
    failures: RefCell<VecDeque<ContentFailure>>,
    /// The number of entries in `owed`, as a signal.
    pending: Binding<usize>,
    /// Tasks awaiting a [`Settled`] future.
    wakers: RefCell<Vec<Waker>>,
}

impl LedgerState {
    /// Publishes a change of `owed` or `failures`: the pending count follows
    /// the record, and every waiting [`Settled`] re-polls once the ledger has
    /// nothing left to wait for.
    fn changed(&self) {
        let owed = self.owed.borrow().len();
        // No borrow is held across the set: a watcher of the count may mount
        // a view that owes content of its own.
        self.pending.set(owed);
        if owed == 0 || !self.failures.borrow().is_empty() {
            let wakers = core::mem::take(&mut *self.wakers.borrow_mut());
            for waker in wakers {
                waker.wake();
            }
        }
    }

    fn resolve(&self, id: u64) -> Option<PendingContent> {
        self.owed.borrow_mut().remove(&id)
    }
}

impl ContentLedger {
    /// Creates an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(LedgerState {
            next_id: Cell::new(0),
            owed: RefCell::new(BTreeMap::new()),
            failures: RefCell::new(VecDeque::new()),
            pending: Binding::container(0),
            wakers: RefCell::new(Vec::new()),
        }))
    }

    /// Records that the view `V` owes content beyond what its mount shows.
    ///
    /// `detail` names what the view is waiting for — a source URL, a
    /// resource name — so a capture that gives up can say which load was
    /// late. Called from the view's `body`, so the content counts as owed
    /// from mount onward.
    ///
    /// Returns `None` when `env` carries no ledger: nothing observes the
    /// content outside a capture, and nothing is recorded.
    ///
    /// # Panics
    ///
    /// Panics if one ledger hands out more than `u64::MAX` entries.
    #[must_use]
    pub fn owe<V: 'static>(
        env: &Environment,
        detail: impl Into<Cow<'static, str>>,
    ) -> Option<OwedContent> {
        let ledger = env.get::<Self>()?;
        let state = &ledger.0;
        let id = state.next_id.get();
        state.next_id.set(
            id.checked_add(1)
                .expect("content ledger token ids overflowed"),
        );
        state.owed.borrow_mut().insert(
            id,
            PendingContent {
                view: view_name::<V>(),
                detail: detail.into(),
            },
        );
        state.changed();
        Some(OwedContent {
            state: Rc::downgrade(state),
            id,
        })
    }

    /// The number of owed entries not yet resolved.
    #[must_use]
    pub fn pending(&self) -> Computed<usize> {
        self.0.pending.clone().computed()
    }

    /// A future that resolves once nothing is owed, or with the first load
    /// failure no earlier [`Settled`] reported.
    ///
    /// A ledger with nothing owed is settled at once; content registered
    /// after it settled makes the next [`Self::settled`] wait again.
    pub fn settled(&self) -> Settled {
        Settled {
            state: Rc::clone(&self.0),
        }
    }

    /// The content still owed, in registration order.
    #[must_use]
    pub fn outstanding(&self) -> Vec<PendingContent> {
        self.0.owed.borrow().values().cloned().collect()
    }

    /// Awaits the owed content of a mounted subtree, with a bound.
    ///
    /// Each pass waits until the ledger [settles](Self::settled) or `expiry`
    /// completes, whichever comes first, then runs `drain` — the host's
    /// commit, layout or scene pump, which runs the lifecycle hooks and
    /// relayouts that resolved content triggers — and checks the ledger
    /// again. It returns once a full pass leaves nothing owed: content a
    /// drain mounted late is awaited by the next pass.
    ///
    /// `expiry` is the host's timer for `deadline`; the duration itself only
    /// labels the error.
    ///
    /// # Errors
    ///
    /// [`ContentError::Pending`] naming every view that still owes content
    /// when `expiry` completes first, and [`ContentError::Failed`] when a
    /// load fails.
    #[expect(
        clippy::future_not_send,
        reason = "the ledger lives on the UI thread with the views that resolve it"
    )]
    pub async fn wait<D: Future<Output = ()>>(
        &self,
        deadline: Duration,
        expiry: impl Future<Output = ()>,
        mut drain: impl FnMut() -> D,
    ) -> Result<(), ContentError> {
        let mut expiry = pin!(expiry);
        loop {
            let mut settled = self.settled();
            let outcome = poll_fn(|cx| {
                if let Poll::Ready(result) = Pin::new(&mut settled).poll(cx) {
                    return Poll::Ready(Some(result));
                }
                if expiry.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(None);
                }
                Poll::Pending
            })
            .await;
            match outcome {
                Some(Ok(())) => {}
                Some(Err(failure)) => return Err(ContentError::Failed(failure)),
                None => {
                    return Err(ContentError::Pending {
                        outstanding: self.outstanding(),
                        deadline,
                    });
                }
            }
            drain().await;
            if self.0.owed.borrow().is_empty() && self.0.failures.borrow().is_empty() {
                return Ok(());
            }
        }
    }
}

impl Default for ContentLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ContentLedger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentLedger")
            .field("owed", &*self.0.owed.borrow())
            .field("failures", &self.0.failures.borrow().len())
            .finish_non_exhaustive()
    }
}

/// The unqualified name of `V`: its path and generic arguments dropped, so
/// a message names `Photo`, not `waterui_media::photo::Photo`.
fn view_name<V: 'static>() -> &'static str {
    let full = core::any::type_name::<V>();
    let path = full.split_once('<').map_or(full, |(path, _)| path);
    path.rsplit_once("::").map_or(path, |(_, name)| name)
}

/// One entry of the ledger: the view that owes content and what it is
/// waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PendingContent {
    /// The owing view's type name, without its path.
    view: &'static str,
    /// What the view is waiting for, as it described it.
    detail: Cow<'static, str>,
}

impl PendingContent {
    /// Creates a `PendingContent` from its parts.
    #[must_use]
    pub const fn new(view: &'static str, detail: Cow<'static, str>) -> Self {
        Self { view, detail }
    }

    /// The owing view's type name, without its path.
    #[must_use]
    pub const fn view(&self) -> &'static str {
        self.view
    }

    /// What the view is waiting for, as it described it.
    #[must_use]
    pub const fn detail(&self) -> &Cow<'static, str> {
        &self.detail
    }
}

impl fmt::Display for PendingContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.view, self.detail)
    }
}

/// Content a view owes a capture, held until the content is in place.
///
/// Resolve it with [`Self::fulfil`] or [`Self::fail`]. Dropping it
/// unresolved releases the entry: the view was unmounted, so the capture no
/// longer waits for it, and that is not a failure.
#[must_use = "an owed entry dropped at once releases the content it was meant to hold the capture for"]
pub struct OwedContent {
    state: Weak<LedgerState>,
    id: u64,
}

impl OwedContent {
    /// The content is in place.
    pub fn fulfil(self) {
        if let Some(state) = self.state.upgrade()
            && state.resolve(self.id).is_some()
        {
            state.changed();
        }
    }

    /// Loading the content failed; the capture reports `error` with the
    /// view that owed it.
    pub fn fail(self, error: impl core::error::Error + Send + Sync + 'static) {
        if let Some(state) = self.state.upgrade()
            && let Some(entry) = state.resolve(self.id)
        {
            state.failures.borrow_mut().push_back(ContentFailure {
                view: entry.view,
                detail: entry.detail,
                source: Box::new(error),
            });
            state.changed();
        }
    }
}

impl Drop for OwedContent {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade()
            && state.resolve(self.id).is_some()
        {
            state.changed();
        }
    }
}

impl fmt::Debug for OwedContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwedContent")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A load a view reported failed through [`OwedContent::fail`].
#[derive(Debug, thiserror::Error)]
#[error("{view} ({detail}) failed to load its content")]
#[non_exhaustive]
pub struct ContentFailure {
    /// The owing view's type name, without its path.
    view: &'static str,
    /// What the view was waiting for.
    detail: Cow<'static, str>,
    /// Why the load failed.
    #[source]
    source: Box<dyn core::error::Error + Send + Sync>,
}

impl ContentFailure {
    /// The owing view's type name, without its path.
    #[must_use]
    pub const fn view(&self) -> &'static str {
        self.view
    }

    /// What the view was waiting for.
    #[must_use]
    pub const fn detail(&self) -> &Cow<'static, str> {
        &self.detail
    }

    /// Why the load failed.
    #[must_use]
    pub fn get_source(&self) -> &(dyn core::error::Error + Send + Sync) {
        &*self.source
    }

    /// Takes the failure apart into the view's type name, what it was waiting
    /// for and why the load failed.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        &'static str,
        Cow<'static, str>,
        Box<dyn core::error::Error + Send + Sync>,
    ) {
        (self.view, self.detail, self.source)
    }
}

/// The future [`ContentLedger::settled`] returns.
#[must_use = "futures do nothing unless awaited"]
pub struct Settled {
    state: Rc<LedgerState>,
}

impl Future for Settled {
    type Output = Result<(), ContentFailure>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let state = &self.state;
        if let Some(failure) = state.failures.borrow_mut().pop_front() {
            return Poll::Ready(Err(failure));
        }
        if state.owed.borrow().is_empty() {
            return Poll::Ready(Ok(()));
        }
        let mut wakers = state.wakers.borrow_mut();
        if !wakers.iter().any(|waker| waker.will_wake(cx.waker())) {
            wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

impl fmt::Debug for Settled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settled").finish_non_exhaustive()
    }
}

/// Why [`ContentLedger::wait`] returned without the content in place.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ContentError {
    /// The deadline passed with content still owed.
    #[error("{} still owe{} content after {deadline:?}", DisplayList(outstanding), if outstanding.len() == 1 { "s" } else { "" })]
    Pending {
        /// Every entry still owed when the deadline passed.
        outstanding: Vec<PendingContent>,
        /// The bound the capture waited for.
        deadline: Duration,
    },
    /// A view reported that its content failed to load.
    #[error(transparent)]
    Failed(#[from] ContentFailure),
}

/// Displays owed entries separated by commas.
pub(crate) struct DisplayList<'a>(pub(crate) &'a [PendingContent]);

impl fmt::Display for DisplayList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, entry) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            fmt::Display::fmt(entry, f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

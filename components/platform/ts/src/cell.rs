//! One JavaScript reactive value bound to one nami signal.
//!
//! A cell is the whole of the `Signal<T> ↔ Binding<T>` mapping: a subscription
//! that carries JavaScript changes into a `Binding<T>`, a `watch` that carries
//! Rust changes back out, and the discipline that keeps one change from
//! becoming two without losing one that is real.
//!
//! # What the cell pushes
//!
//! Always the binding's current value, never the value the notification
//! carried. A watcher registered before the cell's own may write again while
//! the first change is still being delivered — nami snapshots its watchers, so
//! the outer notification still arrives afterwards, carrying a value that is
//! no longer the state. Reading the binding makes a late notification push the
//! latest value instead of resurrecting an old one; a push of a value
//! JavaScript already holds is deduplicated there and settles as it stands.
//!
//! # Two flags, and what the binding holds afterwards
//!
//! `Binding::set` notifies unconditionally — `distinct` is opt-in — so an
//! applied inbound value would bounce straight back out. While the cell
//! applies one, the inbound flag is raised and the watch does not push.
//!
//! What happens *after* the apply is the whole of the inbound direction: the
//! cell pushes what the binding holds, once, every time. A binding is not a
//! box that stores whatever it is handed. `Binding::filter` drops a write it
//! rejects, a mapped binding's setter may normalize the value before storing
//! it, and another watcher may answer the change with a write of its own —
//! after any of those, the value JavaScript sent is not the value Rust holds,
//! and only the binding can say which one that is. Reading it back covers all
//! of them with one rule, and needs no arithmetic over notifications, which
//! would be wrong the moment a setter notified a different number of times
//! than it was called.
//!
//! The push that follows an ordinary apply costs nothing, because the value
//! is already there: the runtime's `write` compares what the target holds
//! with the incoming value by the seam's own structural equality and skips a
//! write that would store the same thing. Without that, every inbound change
//! would come back as a fresh object and re-run every JavaScript subscriber.
//!
//! The outbound direction is the mirror image. A JavaScript write settles its
//! effects synchronously and the subscription fires during it, perhaps several
//! times; none of that ordering is Rust's business, so the outbound flag
//! suppresses the subscription for the whole write and `write` answers where
//! the value came to rest. If it did not stand, the cell takes the settled
//! value back — under the inbound flag — and pushes what the binding holds
//! afterwards, which is the same loop. It is bounded: two sides that answer
//! every value with a different one are a cycle, and a cycle is reported with
//! the binding's identity, not ridden into a stack overflow.
//!
//! # Queued delivery, and why the watch is renewed
//!
//! The inbound flag covers only the notifications delivered while the apply
//! runs. nami does not nest deliveries: a notification raised while the
//! binding is already delivering one — an apply made by the settle loop from
//! inside the cell's own watch, or from inside any other watcher of the same
//! binding — is queued and delivered after the round in progress, long after
//! the flag is lowered. What the cell needs to know about such a notification
//! is when it was raised, not what it carries: raised before the push that
//! follows the apply, it is covered by that push; raised after, it is a change
//! JavaScript has not seen.
//!
//! nami draws exactly that line for watcher registrations: a queued
//! notification goes only to the watchers that were registered when it was
//! raised and still are when it runs, because a watcher registered in between
//! has already read the state it describes. So the end of an apply retires
//! the cell's watch and registers a fresh one, before the binding is read back
//! for the push. What was raised up to then — the apply's own notification,
//! and anything a watcher raised in answer while it was delivered inline — is
//! addressed to the retired registration and never reaches the cell; a change
//! raised afterwards, including a watcher's answer to the queued notification,
//! reaches the fresh one and is pushed. Only the outermost of nested applies
//! renews the watch, since an inner one runs inside the outer one's scope and
//! the outer one's push covers it.
//!
//! Classifying each notification against a remembered pushed value instead,
//! which this branch did at first, is wrong twice over: a value pushed once
//! can arrive again later in the same settle and be dropped as an echo, and a
//! payload carrying a signal or a callback can never be recognised at all,
//! because a handle crossing back out of JavaScript is a fresh handle.
//! Counting the queued notifications an apply raised fails the same way the
//! arithmetic above does; the registration is the one record of "already
//! read" that nami itself keeps.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nami::watcher::{BoxWatcherGuard, Context};
use nami::{Binding, Computed, Signal};
use waterui_ts_engine::{JsError, JsFunction, JsValue};

use crate::bridge::{Bridge, Compare, Settled, WeakBridge};
use crate::callback::CallbackHandle;
use crate::convert::{FromJs, IntoJs};
use crate::error::kind_of;

/// How many times one change may bounce between the two sides before the cell
/// calls it a cycle.
///
/// A settled write needs one round. A correction — a JavaScript effect that
/// clamps the value, a Rust watcher that answers it — needs one more each.
/// Past this, the two sides are answering each other rather than converging,
/// and one more round would only deepen the recursion.
const SETTLE_ROUNDS: usize = 16;

/// Which way values travel between a cell's two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// JavaScript to Rust only: an accessor materialized as a `Computed<T>`.
    Inbound,
    /// Both ways: a JavaScript signal materialized as a `Binding<T>`, or a
    /// Rust `Binding<T>` exported as a JavaScript signal.
    TwoWay,
}

impl Flow {
    const fn watches(self) -> bool {
        matches!(self, Self::TwoWay)
    }
}

/// What each side is currently doing, so neither answers the other.
#[derive(Debug, Default)]
struct Echo {
    /// Raised while an inbound value is being applied to the binding. It
    /// suppresses the notifications delivered during the apply; the ones
    /// queued behind a delivery in progress are left to the retired watch.
    inbound: Cell<bool>,
    /// Raised while a Rust value is being written into JavaScript, for as long
    /// as that write settles.
    outbound: Cell<bool>,
}

/// Raises a flag for a scope, restoring what it was even if the scope unwinds.
///
/// The previous value is restored rather than cleared, because these scopes
/// nest: an engine call made inside one can call back into Rust and reach the
/// same cell, and a nested scope that lowered the flag on the way out would
/// unguard the one still running.
struct Raised<'a> {
    flag: &'a Cell<bool>,
    previous: bool,
}

impl<'a> Raised<'a> {
    const fn new(flag: &'a Cell<bool>) -> Self {
        Self {
            flag,
            previous: flag.replace(true),
        }
    }
}

impl Drop for Raised<'_> {
    fn drop(&mut self) {
        self.flag.set(self.previous);
    }
}

/// Converts one Rust value and writes it into JavaScript.
///
/// Neither failure it can meet has a caller to return to — a backend's
/// `Binding::set` is the writer — so both are reported through `tracing` and
/// the JavaScript side keeps the value it had.
fn push<T: IntoJs>(bridge: &Bridge, source: &JsValue, echo: &Echo, value: T) -> Option<Settled> {
    let value = match value.into_js(bridge) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "a bridged value could not be converted for JavaScript");
            return None;
        }
    };
    let compare = Compare::of(&value);
    let _outbound = Raised::new(&echo.outbound);
    match bridge.write_value(source, value, compare) {
        Ok(settled) => Some(settled),
        Err(error) => {
            tracing::error!(%error, "writing a bridged value into JavaScript failed");
            None
        }
    }
}

/// What a cell's subscription and its watch share: the binding, the
/// JavaScript value it is bound to, and the registration that pushes Rust
/// changes out.
struct Link<T: 'static> {
    binding: Binding<T>,
    source: JsValue,
    bridge: WeakBridge,
    echo: Echo,
    /// The current watch registration, `None` when the flow does not push.
    ///
    /// Replaced after every inbound apply, so it always dates from the last
    /// time the cell read the binding to push it (see the module docs).
    watch: RefCell<Option<BoxWatcherGuard>>,
}

impl<T: FromJs + IntoJs + Clone + 'static> Link<T> {
    /// Registers a watch that sees only the notifications raised from now on.
    ///
    /// The watch holds the link weakly, because the link owns the guard.
    fn watch_from_now(self: &Rc<Self>) -> BoxWatcherGuard {
        let link = Rc::downgrade(self);
        self.binding.watch(move |_: Context<T>| {
            // A round snapshots its watchers before it starts, so a cell
            // dropped by a watcher earlier in the same round is still called
            // once; there is nothing left to push into.
            let Some(link) = link.upgrade() else {
                return;
            };
            link.changed();
        })
    }

    /// The watch: a Rust change JavaScript has not seen yet.
    fn changed(self: &Rc<Self>) {
        if self.echo.inbound.get() {
            // The apply that is running pushes what the binding holds once it
            // is over, so every change delivered during it — its own, and any
            // a watcher makes in answer — is covered by that one push.
            return;
        }
        let Some(bridge) = self.bridge.upgrade() else {
            tracing::debug!("a bridged value changed after its TypeScript runtime was dropped");
            return;
        };
        self.settle(&bridge);
    }

    /// Applies one inbound value to the binding, so that no notification the
    /// write raises — delivered during it or queued behind a delivery in
    /// progress — bounces back out.
    fn apply(self: &Rc<Self>, value: T) {
        let outermost = !self.echo.inbound.get();
        {
            let _inbound = Raised::new(&self.echo.inbound);
            self.binding.set(value);
        }
        if outermost {
            self.rewatch();
        }
    }

    /// Retires the watch registration and replaces it with one that starts
    /// now, before the binding is read back for the push that covers the
    /// apply.
    fn rewatch(self: &Rc<Self>) {
        if self.watch.borrow().is_none() {
            // An inbound-only cell has no watch to renew.
            return;
        }
        // Register first, so the binding is never left without the cell's
        // watcher; the retired guard is dropped after the slot's borrow ends.
        let fresh = self.watch_from_now();
        let retired = self.watch.replace(Some(fresh));
        drop(retired);
    }

    /// Pushes the binding's current value, and keeps the two sides talking
    /// until they hold the same one.
    fn settle(self: &Rc<Self>, bridge: &Bridge) {
        for _ in 0..SETTLE_ROUNDS {
            match push(bridge, &self.source, &self.echo, self.binding.snapshot()) {
                // Nothing more to do: either the value stands, or the write
                // failed and was reported, and JavaScript keeps what it had.
                None | Some(Settled::Stood) => return,
                Some(Settled::Changed) => {}
            }
            // An effect rewrote it while the write settled, so what JavaScript
            // holds now is the value both sides must agree on.
            let settled = match bridge
                .read_value(&self.source)
                .and_then(|value| T::from_js(&value, bridge))
            {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(%error, "reading back the value a JavaScript write settled at failed");
                    return;
                }
            };
            self.apply(settled);
        }
        tracing::error!(
            rounds = SETTLE_ROUNDS,
            binding = ?self.binding.identity(),
            source = kind_of(&self.source),
            "a bridged value never settled: each side keeps answering the other's value with a \
             different one"
        );
    }
}

/// One JavaScript reactive value bound to one `Binding<T>`.
///
/// Dropping the cell disposes the JavaScript subscription and releases the
/// watch guard and the registry entry, so the lifetime of the bridged value is
/// the lifetime of whatever holds the cell.
pub struct ReactiveCell<T: 'static> {
    link: Rc<Link<T>>,
    bridge: WeakBridge,
    /// The dispose the subscription handed back, called exactly once.
    dispose: RefCell<Option<JsFunction>>,
    /// The registry entry JavaScript notifies through.
    _callback: CallbackHandle,
}

impl<T: FromJs + IntoJs + Clone + 'static> ReactiveCell<T> {
    /// Binds `source` — a JavaScript reactive value — to `binding`, in the
    /// directions `flow` allows.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the subscription cannot be installed, or when
    /// the callback registry is exhausted.
    pub(crate) fn attach(
        bridge: &Bridge,
        source: JsValue,
        binding: Binding<T>,
        flow: Flow,
    ) -> Result<Rc<Self>, JsError> {
        let link = Rc::new(Link {
            binding,
            source,
            bridge: bridge.downgrade(),
            echo: Echo::default(),
            watch: RefCell::new(None),
        });

        let callback = bridge.register_callback({
            let link = Rc::clone(&link);
            let two_way = flow.watches();
            move |args: &[JsValue]| {
                if link.echo.outbound.get() {
                    // Our own write, settling. The value it comes to rest at
                    // is read once, after the write returns.
                    return Ok(JsValue::Undefined);
                }
                let value = args.first().unwrap_or(&JsValue::Undefined);
                let bridge = link.bridge.upgrade().ok_or_else(|| {
                    JsError::new(
                        "Error",
                        "the TypeScript bridge was dropped while JavaScript was calling into it",
                    )
                })?;
                let converted = T::from_js(value, &bridge)?;
                link.apply(converted);
                if two_way {
                    // What the binding holds now is the answer — the value
                    // may have been filtered, normalized, or replaced by
                    // another watcher — and JavaScript has not seen it. The
                    // push is free when it is the value already there.
                    link.settle(&bridge);
                }
                Ok(JsValue::Undefined)
            }
        })?;
        let dispose = bridge.subscribe(&link.source, callback.id())?;

        if flow.watches() {
            let watch = link.watch_from_now();
            *link.watch.borrow_mut() = Some(watch);
        }

        Ok(Rc::new(Self {
            link,
            bridge: bridge.downgrade(),
            dispose: RefCell::new(dispose),
            _callback: callback,
        }))
    }

    /// The nami side of the cell.
    pub(crate) fn binding(&self) -> &Binding<T> {
        &self.link.binding
    }
}

impl<T: 'static> Drop for ReactiveCell<T> {
    fn drop(&mut self) {
        let Some(dispose) = self.dispose.borrow_mut().take() else {
            return;
        };
        let Some(bridge) = self.bridge.upgrade() else {
            // The engine died first and took the subscription with it.
            return;
        };
        if let Err(error) = bridge.call(&dispose, &[]) {
            tracing::error!(%error, "disposing a JavaScript subscription failed");
        }
    }
}

/// A Rust value pushed into JavaScript with nothing coming back.
///
/// This is what a `Computed<T>` exported to JavaScript owns: a watch that
/// writes every change into the signal the memo JavaScript holds reads from.
/// There is no inbound half, because a memo cannot be written to, and where
/// the value settled is not this cell's business: the next change overwrites
/// whatever an effect did to the signal underneath.
///
/// The cell holds the computed itself, not only the guard. A watcher
/// registration does not keep its signal alive, and a computed handed to
/// `into_js` is usually a temporary — a theme assembled at mount, a `map` of a
/// binding — so letting it drop here would release whatever it was watching
/// and the exported accessor would never move again.
pub struct OutboundCell<T: 'static> {
    #[expect(
        dead_code,
        reason = "held so the exported computed outlives the call that exported it"
    )]
    signal: Computed<T>,
    #[expect(dead_code, reason = "held to keep the watch registered")]
    guard: BoxWatcherGuard,
}

impl<T: IntoJs + Clone + 'static> OutboundCell<T> {
    /// Pushes every change of `computed` into `source`.
    pub(crate) fn push(bridge: &Bridge, source: JsValue, computed: &Computed<T>) -> Self {
        let echo = Echo::default();
        let weak = bridge.downgrade();
        let signal = computed.clone();
        let guard = computed.watch(move |_: Context<T>| {
            let Some(bridge) = weak.upgrade() else {
                tracing::debug!("a bridged value changed after its TypeScript runtime was dropped");
                return;
            };
            // The current value, not the notification's: a watcher that runs
            // before this one may have moved the source on already.
            push(&bridge, &source, &echo, signal.snapshot());
        });
        Self {
            signal: computed.clone(),
            guard,
        }
    }
}

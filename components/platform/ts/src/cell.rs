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
//! Classifying each notification against a remembered pushed value instead,
//! which this branch did at first, is wrong twice over: a value pushed once
//! can arrive again later in the same settle and be dropped as an echo, and a
//! payload carrying a signal or a callback can never be recognised at all,
//! because a handle crossing back out of JavaScript is a fresh handle.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nami::watcher::{BoxWatcherGuard, Context};
use nami::{Binding, Computed, Signal};
use waterui_ts_engine::{JsError, JsFunction, JsValue};

use crate::bridge::{Bridge, Settled, WeakBridge};
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
    /// Raised while an inbound value is being applied to the binding.
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
    let _outbound = Raised::new(&echo.outbound);
    match bridge.write_value(source, value) {
        Ok(settled) => Some(settled),
        Err(error) => {
            tracing::error!(%error, "writing a bridged value into JavaScript failed");
            None
        }
    }
}

/// Applies one inbound value to the binding, without the write bouncing back
/// out while it is being applied.
fn apply<T: Clone + 'static>(echo: &Echo, binding: &Binding<T>, value: T) {
    let _inbound = Raised::new(&echo.inbound);
    binding.set(value);
}

/// Pushes the binding's current value, and keeps the two sides talking until
/// they hold the same one.
fn settle<T: FromJs + IntoJs + Clone + 'static>(
    bridge: &Bridge,
    source: &JsValue,
    echo: &Echo,
    binding: &Binding<T>,
) {
    for _ in 0..SETTLE_ROUNDS {
        match push(bridge, source, echo, binding.get()) {
            // Nothing more to do: either the value stands, or the write
            // failed and was reported, and JavaScript keeps what it had.
            None | Some(Settled::Stood) => return,
            Some(Settled::Changed) => {}
        }
        // An effect rewrote it while the write settled, so what JavaScript
        // holds now is the value both sides must agree on.
        let settled = match bridge
            .read_value(source)
            .and_then(|value| T::from_js(&value, bridge))
        {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(%error, "reading back the value a JavaScript write settled at failed");
                return;
            }
        };
        apply(echo, binding, settled);
    }
    tracing::error!(
        rounds = SETTLE_ROUNDS,
        binding = ?binding.identity(),
        source = kind_of(source),
        "a bridged value never settled: each side keeps answering the other's value with a \
         different one"
    );
}

/// One JavaScript reactive value bound to one `Binding<T>`.
///
/// Dropping the cell disposes the JavaScript subscription and releases the
/// watch guard and the registry entry, so the lifetime of the bridged value is
/// the lifetime of whatever holds the cell.
pub struct ReactiveCell<T: 'static> {
    binding: Binding<T>,
    bridge: WeakBridge,
    /// The dispose the subscription handed back, called exactly once.
    dispose: RefCell<Option<JsFunction>>,
    /// The registry entry JavaScript notifies through.
    _callback: CallbackHandle,
    /// The nami watch that pushes Rust changes out.
    _watch: Option<BoxWatcherGuard>,
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
        let echo = Rc::new(Echo::default());

        let callback = bridge.register_callback({
            let binding = binding.clone();
            let echo = Rc::clone(&echo);
            let source = source.clone();
            let weak = bridge.downgrade();
            let two_way = flow.watches();
            move |args: &[JsValue]| {
                if echo.outbound.get() {
                    // Our own write, settling. The value it comes to rest at
                    // is read once, after the write returns.
                    return Ok(JsValue::Undefined);
                }
                let value = args.first().unwrap_or(&JsValue::Undefined);
                let bridge = weak.upgrade().ok_or_else(|| {
                    JsError::new(
                        "Error",
                        "the TypeScript bridge was dropped while JavaScript was calling into it",
                    )
                })?;
                let converted = T::from_js(value, &bridge)?;
                apply(&echo, &binding, converted);
                if two_way {
                    // What the binding holds now is the answer — the value
                    // may have been filtered, normalized, or replaced by
                    // another watcher — and JavaScript has not seen it. The
                    // push is free when it is the value already there.
                    settle(&bridge, &source, &echo, &binding);
                }
                Ok(JsValue::Undefined)
            }
        })?;
        let dispose = bridge.subscribe(&source, callback.id())?;

        let watch = flow.watches().then(|| {
            let weak = bridge.downgrade();
            let echo = Rc::clone(&echo);
            let binding = binding.clone();
            let source = source;
            binding.clone().watch(move |_: Context<T>| {
                if echo.inbound.get() {
                    // The apply that is running pushes what the binding holds
                    // once it is over, so every change made during it — its
                    // own, and any a watcher makes in answer — is covered by
                    // that one push.
                    return;
                }
                let Some(bridge) = weak.upgrade() else {
                    tracing::debug!(
                        "a bridged value changed after its TypeScript runtime was dropped"
                    );
                    return;
                };
                settle(&bridge, &source, &echo, &binding);
            })
        });

        Ok(Rc::new(Self {
            binding,
            bridge: bridge.downgrade(),
            dispose: RefCell::new(dispose),
            _callback: callback,
            _watch: watch,
        }))
    }

    /// The nami side of the cell.
    pub(crate) const fn binding(&self) -> &Binding<T> {
        &self.binding
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
            push(&bridge, &source, &echo, signal.get());
        });
        Self {
            signal: computed.clone(),
            guard,
        }
    }
}

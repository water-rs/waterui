//! One JavaScript reactive value bound to one nami signal.
//!
//! A cell is the whole of the `Signal<T> ↔ Binding<T>` mapping: a subscription
//! that carries JavaScript changes into a `Binding<T>`, a `watch` that carries
//! Rust changes back out, and the discipline that keeps one change from
//! becoming two.
//!
//! # Why a flag, and why a remembered value
//!
//! `Binding::set` notifies unconditionally — `distinct` is opt-in — so an
//! applied inbound value would bounce straight back out. While the cell
//! applies one, the inbound flag is raised and the watch stays quiet. That is
//! the rule the web view's state mirror follows for its inbound writes.
//!
//! The other direction needs more, because a JavaScript write settles its
//! effects synchronously: the subscription the cell itself installed fires
//! during the push. The cell therefore remembers exactly what it sent. A
//! notification carrying that value is its own echo and is dropped; a
//! notification carrying a *different* value is a correction — an effect
//! clamped or rewrote it — and is applied, so both sides converge. The web
//! view needs epochs for this because its writes cross an asynchronous
//! transport; here the seam is synchronous and in-process, and the remembered
//! value is exact.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nami::watcher::{BoxWatcherGuard, Context};
use nami::{Binding, Computed, Signal};
use waterui_ts_engine::{JsError, JsFunction, JsValue};

use crate::bridge::{Bridge, WeakBridge};
use crate::callback::CallbackHandle;
use crate::convert::{FromJs, IntoJs};

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
    /// The value currently being pushed into JavaScript, if any.
    pushed: RefCell<Option<JsValue>>,
}

impl Echo {
    /// Whether `value` is the one this cell is pushing right now.
    fn is_own_push(&self, value: &JsValue) -> bool {
        self.pushed.borrow().as_ref() == Some(value)
    }
}

/// Raises a flag for a scope, lowering it even if the scope unwinds.
struct Raised<'a>(&'a Cell<bool>);

impl<'a> Raised<'a> {
    fn new(flag: &'a Cell<bool>) -> Self {
        flag.set(true);
        Self(flag)
    }
}

impl Drop for Raised<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// Remembers a pushed value for a scope, forgetting it even on unwind.
struct Pushing<'a>(&'a RefCell<Option<JsValue>>);

impl<'a> Pushing<'a> {
    fn new(slot: &'a RefCell<Option<JsValue>>, value: JsValue) -> Self {
        *slot.borrow_mut() = Some(value);
        Self(slot)
    }
}

impl Drop for Pushing<'_> {
    fn drop(&mut self) {
        *self.0.borrow_mut() = None;
    }
}

/// The watcher that pushes a Rust value into the JavaScript side.
///
/// Neither failure it can meet has a caller to return to — a backend's
/// `Binding::set` is the writer — so both are reported through `tracing` and
/// the JavaScript side keeps the value it had.
fn outbound_watcher<T: IntoJs + 'static>(
    bridge: &Bridge,
    source: JsValue,
    echo: Rc<Echo>,
) -> impl Fn(Context<T>) + 'static {
    let weak = bridge.downgrade();
    move |context: Context<T>| {
        if echo.inbound.get() {
            return;
        }
        let Some(bridge) = weak.upgrade() else {
            tracing::debug!("a bridged value changed after its TypeScript runtime was dropped");
            return;
        };
        let value = match context.into_value().into_js(&bridge) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(%error, "a bridged value could not be converted for JavaScript");
                return;
            }
        };
        let _pushing = Pushing::new(&echo.pushed, value.clone());
        if let Err(error) = bridge.write_value(&source, value) {
            tracing::error!(%error, "writing a bridged value into JavaScript failed");
        }
    }
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
            let weak = bridge.downgrade();
            move |args: &[JsValue]| {
                let value = args.first().unwrap_or(&JsValue::Undefined);
                if echo.is_own_push(value) {
                    // Our own write, settling back through the very
                    // subscription we installed.
                    return Ok(JsValue::Undefined);
                }
                let bridge = weak.upgrade().ok_or_else(|| {
                    JsError::new(
                        "Error",
                        "the TypeScript bridge was dropped while JavaScript was calling into it",
                    )
                })?;
                let converted = T::from_js(value, &bridge)?;
                let _raised = Raised::new(&echo.inbound);
                binding.set(converted);
                Ok(JsValue::Undefined)
            }
        })?;
        let dispose = bridge.subscribe(&source, callback.id())?;

        let watch = flow
            .watches()
            .then(|| binding.watch(outbound_watcher(bridge, source, Rc::clone(&echo))));

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
/// There is no inbound half, because a memo cannot be written to.
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
        Self {
            signal: computed.clone(),
            guard: computed.watch(outbound_watcher(bridge, source, Rc::new(Echo::default()))),
        }
    }
}

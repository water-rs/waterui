//! The registry behind every Rust closure JavaScript can call.
//!
//! A host function is registered under a *name*, never handed over as a value,
//! so a Rust closure cannot itself become a JavaScript function. The bridge
//! registers one entry — `__waterui_host.invoke` — and keeps the closures in
//! this registry, keyed by id. JavaScript receives `makeCallback(id)`, a
//! wrapper that calls `invoke(id, …args)`.
//!
//! One mechanism serves both kinds of closure: a prop callback the view code
//! invokes, and the notification a materialized cell subscribes with.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use waterui_ts_engine::{JsError, JsValue};

/// A Rust closure JavaScript calls through `__waterui_host.invoke`.
pub type Callback = Rc<dyn Fn(&[JsValue]) -> Result<JsValue, JsError>>;

/// The id `makeCallback` wraps. A JavaScript `number` holds it exactly.
pub type CallbackId = u32;

/// Every live Rust closure JavaScript may call, keyed by id.
#[derive(Default)]
pub struct CallbackRegistry {
    next: Cell<CallbackId>,
    entries: std::cell::RefCell<BTreeMap<CallbackId, Callback>>,
}

impl CallbackRegistry {
    /// Registers `callback` and returns the handle that owns its entry.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] once the id space is exhausted, which takes four
    /// billion registrations in one runtime.
    pub(crate) fn register(
        self: &Rc<Self>,
        callback: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) -> Result<CallbackHandle, JsError> {
        let id = self.next.get();
        let next = id.checked_add(1).ok_or_else(|| {
            JsError::new(
                "RangeError",
                "the TypeScript runtime's callback registry is exhausted",
            )
        })?;
        self.next.set(next);
        self.entries.borrow_mut().insert(id, Rc::new(callback));
        Ok(CallbackHandle {
            id,
            registry: Rc::clone(self),
        })
    }

    /// Calls the closure registered under `id` with `args`.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when no closure is registered under `id` — a
    /// wrapper JavaScript kept past the closure's lifetime — or when the
    /// closure itself fails.
    pub(crate) fn invoke(&self, id: CallbackId, args: &[JsValue]) -> Result<JsValue, JsError> {
        // The entry is cloned out before the call: a callback may register or
        // drop another one, and the registry must not be borrowed while it
        // runs.
        let callback = self.entries.borrow().get(&id).cloned().ok_or_else(|| {
            JsError::new(
                "ReferenceError",
                format!(
                    "no Rust callback is registered under id {id}: the JavaScript side kept a \
                     callback past the lifetime of the value that owned it"
                ),
            )
        })?;
        callback(args)
    }
}

/// Ownership of one registered closure: dropping it unregisters the closure.
///
/// A cell keeps its notification callback here, and an exported Rust callback
/// keeps its own, so the entry disappears exactly when the value that owns it
/// does.
pub struct CallbackHandle {
    id: CallbackId,
    registry: Rc<CallbackRegistry>,
}

impl CallbackHandle {
    /// The id to hand `makeCallback`.
    pub(crate) const fn id(&self) -> CallbackId {
        self.id
    }
}

impl Drop for CallbackHandle {
    fn drop(&mut self) {
        self.registry.entries.borrow_mut().remove(&self.id);
    }
}

impl std::fmt::Debug for CallbackRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackRegistry")
            .field("next", &self.next.get())
            .field("live", &self.entries.borrow().len())
            .finish()
    }
}

impl std::fmt::Debug for CallbackHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

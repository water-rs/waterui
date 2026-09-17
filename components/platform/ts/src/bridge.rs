//! The bridge: the engine, the loaded runtime, and everything that crosses.
//!
//! A [`Bridge`] is what every conversion and every host call is handed. It
//! owns the engine, the runtime global a loaded bundle published, the registry
//! of Rust closures JavaScript can call, the `Environment` the mounted module
//! sees, and the open [`MountScope`]s that own what has been exported into
//! JavaScript. It is a cheap handle: cloning shares the same runtime, and it
//! is deliberately neither `Send` nor `Sync`, because the engine is pinned to
//! its thread.

use std::any::Any;
use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use nami::{Binding, Computed, Signal, SignalIdentity};
use waterui_core::{AnyView, Environment};
use waterui_ts_engine::{JsError, JsFunction, JsRuntime, JsValue};

use crate::Engine;
use crate::callback::{CallbackHandle, CallbackId, CallbackRegistry};
use crate::cell::{Flow, OutboundCell, ReactiveCell};
use crate::convert::{FromJs, IntoJs};
use crate::error::{TsError, kind_of};
use crate::runtime_global::RuntimeGlobal;
use crate::tether::Tethered;
use crate::view::ViewSlot;

/// What a reactive input from JavaScript turned out to be.
///
/// The classification is the contract in HOST.md: a `Signal` is writable, an
/// accessor is readable and pushes, and anything else is a constant. A
/// callable value crossed as a handle, so only JavaScript can tell a signal
/// from a memo — the bridge asks `isSignal`. A `{ read, … }` value crossed as
/// data, so its keys answer the question here.
#[derive(Debug, Clone)]
pub enum ReactiveSource {
    /// A writable reactive value: a JS `Signal`, or a `{ read, write,
    /// subscribe }` value.
    Signal(JsValue),
    /// A read-only reactive value: a memo, a thunk, or a
    /// `{ read, subscribe }` value.
    Accessor(JsValue),
    /// A value that never changes — including a `{ read }` value with no
    /// `subscribe`, which can be read once but never announces anything.
    Constant(JsValue),
}

/// What a write into JavaScript left behind.
///
/// A JavaScript write settles its effects synchronously, and an effect may
/// write again — clamping a value, rounding it, refusing it. The runtime's
/// `write` therefore answers whether reading the source back gives exactly
/// what was written, compared on the JavaScript side where a function or an
/// object is its own identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The written value is what the source now holds.
    Stood,
    /// An effect changed it while the write settled.
    Changed,
}

/// Everything one mount exported into JavaScript.
///
/// A value exported into JavaScript — a signal wrapping a `Binding<T>`, the
/// memo behind a `Computed<T>`, a registered callback — is owned by nothing on
/// the Rust side: JavaScript holds it. Somebody has to, or the subscription
/// feeding it dies immediately; that somebody is the scope of the mount the
/// export was made for, and disposing that mount releases every one of them.
#[derive(Debug, Default)]
struct Scope {
    /// The cells and registrations exported while this scope was open.
    retained: RefCell<Vec<Rc<dyn Any>>>,
    /// The JavaScript value already exported for a given Rust signal, so the
    /// same signal crossing twice is the same signal to JavaScript and costs
    /// one cell, not two.
    exported: RefCell<BTreeMap<SignalIdentity, JsValue>>,
}

/// The lifetime of everything a mount exports into JavaScript.
///
/// A mount opens one with [`Bridge::open_scope`] and holds it for as long as
/// the mounted module is on screen; dropping it releases every callback
/// registration, every outbound cell and every exported signal made while it
/// was open, so a mounted-and-disposed module leaves nothing behind. Exporting
/// with no scope open is an error rather than a leak with a runtime-long
/// lifetime.
///
/// Scopes nest: a mount inside a mount opens its own, and exports go to the
/// innermost one.
#[derive(Debug)]
#[must_use = "dropping the scope immediately releases everything exported into it"]
pub struct MountScope {
    bridge: WeakBridge,
    scope: Option<Rc<Scope>>,
}

impl MountScope {
    /// Closes the scope without releasing what it owns.
    ///
    /// Exports made while it was open stay alive for as long as the returned
    /// [`ScopeOwner`] does, and later exports go to whatever scope is open
    /// then. This is how a value shorter-lived than the mount owns its own
    /// exports: a `<For>` item's index accessor belongs to the item, so a list
    /// that churns releases each departed item's signal with the item instead
    /// of accumulating one per row in the mount's scope.
    ///
    /// # Panics
    ///
    /// Never from outside this crate: a scope's own `Drop` is the only other
    /// thing that empties it, and a value cannot be dropped and closed.
    pub fn close(mut self) -> ScopeOwner {
        let scope = self.scope.take().expect("a mount scope closes once");
        Self::detach(&self.bridge, &scope);
        ScopeOwner(scope)
    }

    /// Takes the scope off the bridge's stack, leaving what it owns alone.
    fn detach(bridge: &WeakBridge, scope: &Rc<Scope>) {
        if let Some(bridge) = bridge.upgrade() {
            bridge
                .0
                .scopes
                .borrow_mut()
                .retain(|open| !Rc::ptr_eq(open, scope));
        }
    }
}

impl Drop for MountScope {
    fn drop(&mut self) {
        let Some(scope) = self.scope.take() else {
            return;
        };
        Self::detach(&self.bridge, &scope);
    }
}

/// What a closed [`MountScope`] exported, still alive.
///
/// Dropping it releases every callback registration, outbound cell and
/// exported signal the scope owned, exactly as dropping the open scope would
/// have.
#[derive(Debug)]
#[must_use = "dropping the owner immediately releases everything the scope exported"]
pub struct ScopeOwner(
    #[expect(
        dead_code,
        reason = "the scope is held, not read: what it owns lives exactly as long as this value"
    )]
    Rc<Scope>,
);

/// The engine, the loaded runtime, and the state that crosses between them.
#[derive(Debug, Clone)]
pub struct Bridge(Rc<BridgeInner>);

/// A [`Bridge`] reference that does not keep the runtime alive.
///
/// Every closure the engine or a watcher holds captures one of these: the
/// engine lives inside the bridge, so a strong reference from a host function
/// or a cell would be a cycle that never releases the runtime.
#[derive(Debug, Clone)]
pub struct WeakBridge(Weak<BridgeInner>);

#[derive(Debug)]
struct BridgeInner {
    engine: Engine,
    runtime: OnceCell<RuntimeGlobal>,
    callbacks: Rc<CallbackRegistry>,
    environment: Environment,
    /// The mount scopes currently open, innermost last. Everything exported
    /// into JavaScript belongs to one of them.
    scopes: RefCell<Vec<Rc<Scope>>>,
}

impl Bridge {
    /// Creates a bridge over `engine` for a module mounted in `environment`.
    pub(crate) fn new(engine: Engine, environment: Environment) -> Self {
        Self(Rc::new(BridgeInner {
            engine,
            runtime: OnceCell::new(),
            callbacks: Rc::new(CallbackRegistry::default()),
            environment,
            scopes: RefCell::new(Vec::new()),
        }))
    }

    /// Opens the scope that owns what is exported into JavaScript while it is
    /// open.
    ///
    /// The mount leaf (water-rs/waterui#1044) opens one per mounted module and
    /// drops it when that mount is disposed. Until then a caller that exports
    /// a value opens one itself, because there is no other answer to the
    /// question of who owns an exported signal.
    pub fn open_scope(&self) -> MountScope {
        let scope = Rc::new(Scope::default());
        self.0.scopes.borrow_mut().push(Rc::clone(&scope));
        MountScope {
            bridge: self.downgrade(),
            scope: Some(scope),
        }
    }

    /// How many values the open mount scopes are currently keeping alive.
    ///
    /// One export is one cell: a JavaScript signal materialized inbound, or a
    /// `Binding`/`Computed` exported outbound. The number is what a devtools
    /// panel shows for a mount, and what a test watches while a view's content
    /// churns — a count that climbs as rows come and go is an export that
    /// outlived the row it belonged to.
    #[must_use]
    pub fn exported_count(&self) -> usize {
        self.0
            .scopes
            .borrow()
            .iter()
            .map(|scope| scope.retained.borrow().len())
            .sum()
    }

    /// The environment a mounted module's framework values come from.
    #[must_use]
    pub fn environment(&self) -> &Environment {
        &self.0.environment
    }

    /// The engine this bridge drives.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.0.engine
    }

    /// The runtime the loaded bundle published.
    ///
    /// # Errors
    ///
    /// [`TsError::NotLoaded`] before a bundle is loaded.
    pub fn runtime(&self) -> Result<&RuntimeGlobal, TsError> {
        self.0.runtime.get().ok_or(TsError::NotLoaded {
            what: "anything can cross the seam",
        })
    }

    /// Calls a JavaScript function.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the call throws or a value cannot cross.
    pub fn call(&self, function: &JsFunction, args: &[JsValue]) -> Result<JsValue, JsError> {
        self.0.engine.call(function, args)
    }

    /// Materializes a reactive input as a two-way `Binding<T>`.
    ///
    /// The input must be writable — a JavaScript `Signal` — because the native
    /// view on the other end writes to it. A memo, a thunk or a constant is an
    /// error naming what arrived, instead of a binding whose writes would
    /// vanish.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the input is read-only, when its current value
    /// is not a `T`, or when the subscription cannot be installed.
    pub fn materialize_binding<T: FromJs + IntoJs + Clone + 'static>(
        &self,
        value: &JsValue,
    ) -> Result<Binding<T>, JsError> {
        match self.classify(value)? {
            ReactiveSource::Signal(source) => {
                let seed = T::from_js(&self.read_value(&source)?, self)?;
                let cell =
                    ReactiveCell::attach(self, source, Binding::container(seed), Flow::TwoWay)?;
                Ok(Binding::custom(Tethered::new(
                    cell.binding().clone(),
                    cell as Rc<dyn Any>,
                )))
            }
            ReactiveSource::Accessor(_) => Err(JsError::conversion(
                "a two-way value needs a signal: this attribute was given a read-only accessor, \
                 whose writes would have nowhere to go",
            )),
            ReactiveSource::Constant(value) => Err(JsError::conversion(format!(
                "a two-way value needs a signal, found {}",
                kind_of(&value)
            ))),
        }
    }

    /// Materializes a reactive input as a read-only `Computed<T>`.
    ///
    /// A signal or an accessor becomes a pushed cell; anything else is a
    /// constant and becomes a constant `Computed<T>`, creating no subscription
    /// at all.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the current value is not a `T`, or when the
    /// subscription cannot be installed.
    pub fn materialize_computed<T: FromJs + IntoJs + Clone + 'static>(
        &self,
        value: &JsValue,
    ) -> Result<Computed<T>, JsError> {
        match self.classify(value)? {
            ReactiveSource::Signal(source) | ReactiveSource::Accessor(source) => {
                let seed = T::from_js(&self.read_value(&source)?, self)?;
                let cell =
                    ReactiveCell::attach(self, source, Binding::container(seed), Flow::Inbound)?;
                Ok(Computed::new(Tethered::new(
                    cell.binding().clone(),
                    cell as Rc<dyn Any>,
                )))
            }
            ReactiveSource::Constant(value) => Ok(Computed::constant(T::from_js(&value, self)?)),
        }
    }

    /// Exports a `Binding<T>` as a writable JavaScript signal.
    ///
    /// One Rust signal is one JavaScript signal: the same binding crossing
    /// again inside the same mount — a field of a value pushed on every
    /// change, a prop handed to two components — is the value already
    /// exported, not a second signal with a second cell watching the same
    /// state.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when no mount scope is open, when the current value
    /// cannot cross, or when the signal cannot be created.
    pub fn export_binding<T: FromJs + IntoJs + Clone + 'static>(
        &self,
        binding: &Binding<T>,
    ) -> Result<JsValue, JsError> {
        let scope = self.current_scope()?;
        let identity = binding.identity();
        if let Some(exported) = Self::already_exported(&scope, identity) {
            return Ok(exported);
        }
        let seed = binding.get().into_js(self)?;
        let signal = self.create_js_signal(seed)?;
        let cell = ReactiveCell::attach(self, signal.clone(), binding.clone(), Flow::TwoWay)?;
        Self::retain(&scope, cell, identity, &signal);
        Ok(signal)
    }

    /// Exports a `Computed<T>` as a read-only JavaScript accessor.
    ///
    /// The value is pushed into a signal the bridge owns and handed to
    /// JavaScript as a memo over it, so JavaScript tracks it like any other
    /// derived value and cannot write to it. Like a binding, one Rust computed
    /// is one JavaScript accessor for as long as the mount lives.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when no mount scope is open, when the current value
    /// cannot cross, or when the signal or memo cannot be created.
    pub fn export_computed<T: IntoJs + Clone + 'static>(
        &self,
        computed: &Computed<T>,
    ) -> Result<JsValue, JsError> {
        let scope = self.current_scope()?;
        let identity = computed.identity();
        if let Some(exported) = Self::already_exported(&scope, identity) {
            return Ok(exported);
        }
        let seed = computed.get().into_js(self)?;
        let signal = self.create_js_signal(seed)?;
        let cell = OutboundCell::push(self, signal.clone(), computed);
        let memo = self.create_js_memo(&signal)?;
        // The memo is what JavaScript was handed, so the memo is what the same
        // computed crossing again must be.
        Self::retain(&scope, Rc::new(cell), identity, &memo);
        Ok(memo)
    }

    /// Takes the `AnyView` out of a view slot JavaScript passed back.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the value is not a view slot, or when its view
    /// was already taken.
    pub fn take_view(&self, value: &JsValue) -> Result<AnyView, JsError> {
        ViewSlot::from_js_value(value)?.take()
    }

    /// Classifies a reactive input.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when asking JavaScript whether a function is a
    /// signal fails.
    pub fn classify(&self, value: &JsValue) -> Result<ReactiveSource, JsError> {
        match value {
            JsValue::Function(_) => {
                if self.is_js_signal(value)? {
                    Ok(ReactiveSource::Signal(value.clone()))
                } else {
                    Ok(ReactiveSource::Accessor(value.clone()))
                }
            }
            JsValue::Object(entries) => {
                let is_function = |name: &str| {
                    entries
                        .iter()
                        .any(|(key, value)| key == name && matches!(value, JsValue::Function(_)))
                };
                if !is_function("read") {
                    return Ok(ReactiveSource::Constant(value.clone()));
                }
                if !is_function("subscribe") {
                    // Readable, but it never announces a change: one read is
                    // everything this value will ever say.
                    return Ok(ReactiveSource::Constant(self.read_value(value)?));
                }
                if is_function("write") {
                    Ok(ReactiveSource::Signal(value.clone()))
                } else {
                    Ok(ReactiveSource::Accessor(value.clone()))
                }
            }
            other => Ok(ReactiveSource::Constant(other.clone())),
        }
    }

    /// A weak handle for a closure the engine or a watcher will hold.
    #[must_use]
    pub fn downgrade(&self) -> WeakBridge {
        WeakBridge(Rc::downgrade(&self.0))
    }

    /// Publishes the runtime a freshly evaluated bundle installed.
    pub(crate) fn set_runtime(&self, runtime: RuntimeGlobal) -> Result<(), TsError> {
        self.0
            .runtime
            .set(runtime)
            .map_err(|_| TsError::BundleAlreadyLoaded)
    }

    /// Registers a Rust closure JavaScript can call, and returns the handle
    /// that owns the registration.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the registry is exhausted.
    pub(crate) fn register_callback(
        &self,
        callback: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) -> Result<CallbackHandle, JsError> {
        self.0.callbacks.register(callback)
    }

    /// The registry, for the `invoke` host entry.
    pub(crate) fn callbacks(&self) -> &Rc<CallbackRegistry> {
        &self.0.callbacks
    }

    /// Keeps a registration alive for as long as its mount is.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when no mount scope is open.
    pub(crate) fn retain_export(&self, cell: Rc<dyn Any>) -> Result<(), JsError> {
        self.current_scope()?.retained.borrow_mut().push(cell);
        Ok(())
    }

    /// The innermost open mount scope.
    fn current_scope(&self) -> Result<Rc<Scope>, JsError> {
        self.0
            .scopes
            .borrow()
            .last()
            .map(Rc::clone)
            .ok_or_else(|| JsError::from(TsError::NoMountScope))
    }

    /// The JavaScript value this scope already exported for `identity`.
    fn already_exported(scope: &Scope, identity: Option<SignalIdentity>) -> Option<JsValue> {
        scope.exported.borrow().get(&identity?).cloned()
    }

    /// Gives `scope` the cell, and remembers what JavaScript received for it.
    ///
    /// A signal with no identity — a constant, which owns no observable state
    /// — is still retained, and simply never matches a later export.
    fn retain(
        scope: &Scope,
        cell: Rc<dyn Any>,
        identity: Option<SignalIdentity>,
        exported: &JsValue,
    ) {
        scope.retained.borrow_mut().push(cell);
        if let Some(identity) = identity {
            scope
                .exported
                .borrow_mut()
                .insert(identity, exported.clone());
        }
    }

    /// `makeCallback(id)`: the JavaScript function wrapping a registration.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] before a bundle is loaded, or when the call throws.
    pub(crate) fn make_callback(&self, id: CallbackId) -> Result<JsValue, JsError> {
        let runtime = self.runtime_for("a Rust callback can be wrapped for JavaScript")?;
        self.call(runtime.make_callback(), &[JsValue::from(id)])
    }

    /// `read(source)`: the current value, untracked.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] before a bundle is loaded, or when the read throws.
    pub(crate) fn read_value(&self, source: &JsValue) -> Result<JsValue, JsError> {
        let runtime = self.runtime_for("a reactive value can be read")?;
        self.call(runtime.read_value(), std::slice::from_ref(source))
    }

    /// `write(source, value)`: pushes a Rust value into JavaScript, and says
    /// whether it stood.
    ///
    /// The answer is computed in JavaScript, after the write has settled its
    /// effects, by comparing the source's current value with what was written.
    /// It has to be: the comparison turns on object identity — the same signal
    /// crossing back out is a fresh handle to Rust — and only JavaScript can
    /// see that two references are the same object.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] before a bundle is loaded, when the write throws —
    /// which it does when the target turned out not to be writable — or when
    /// `write` answers something other than a boolean.
    pub(crate) fn write_value(&self, source: &JsValue, value: JsValue) -> Result<Settled, JsError> {
        let runtime = self.runtime_for("a reactive value can be written")?;
        let answer = self.call(runtime.write(), &[source.clone(), value])?;
        match answer.as_bool() {
            Some(true) => Ok(Settled::Stood),
            Some(false) => Ok(Settled::Changed),
            None => Err(JsError::conversion(format!(
                "write() answered {}, not whether the written value stood",
                kind_of(&answer)
            ))),
        }
    }

    /// `subscribe(source, makeCallback(id))`: the push half of the mapping.
    ///
    /// Returns the dispose function the subscription handed back, or `None`
    /// when it handed back nothing to dispose.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] before a bundle is loaded, or when subscribing
    /// throws.
    pub(crate) fn subscribe(
        &self,
        source: &JsValue,
        callback: CallbackId,
    ) -> Result<Option<JsFunction>, JsError> {
        let runtime = self.runtime_for("a JavaScript signal can be subscribed to")?;
        let wrapper = self.make_callback(callback)?;
        match self.call(runtime.subscribe(), &[source.clone(), wrapper])? {
            JsValue::Function(dispose) => Ok(Some(dispose)),
            JsValue::Undefined | JsValue::Null => Ok(None),
            other => Err(JsError::conversion(format!(
                "subscribe() returned {}, not the dispose function the contract requires",
                kind_of(&other)
            ))),
        }
    }

    /// `isSignal(value)`: whether a callable reactive input is writable.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] before a bundle is loaded, or when the call throws.
    pub(crate) fn is_js_signal(&self, value: &JsValue) -> Result<bool, JsError> {
        let runtime = self.runtime_for("a reactive input can be classified")?;
        let answer = self.call(runtime.is_signal(), std::slice::from_ref(value))?;
        answer.as_bool().ok_or_else(|| {
            JsError::conversion(format!(
                "isSignal() answered {}, not a boolean",
                kind_of(&answer)
            ))
        })
    }

    /// `createSignal(seed)`: a fresh JavaScript signal holding `seed`.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] before a bundle is loaded, or when the call throws.
    pub(crate) fn create_js_signal(&self, seed: JsValue) -> Result<JsValue, JsError> {
        let runtime = self.runtime_for("a Rust binding can be exported")?;
        self.call(runtime.create_signal(), &[seed])
    }

    /// `createMemo(signal)`: a read-only accessor over a pushed signal.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] before a bundle is loaded, or when the call throws.
    pub(crate) fn create_js_memo(&self, signal: &JsValue) -> Result<JsValue, JsError> {
        let runtime = self.runtime_for("a Rust computed can be exported")?;
        self.call(runtime.create_memo(), std::slice::from_ref(signal))
    }

    /// The runtime, as a conversion failure when no bundle is loaded.
    fn runtime_for(&self, what: &'static str) -> Result<&RuntimeGlobal, JsError> {
        self.0
            .runtime
            .get()
            .ok_or_else(|| JsError::from(TsError::NotLoaded { what }))
    }
}

impl WeakBridge {
    /// The bridge, or `None` once the runtime has been dropped.
    #[must_use]
    pub fn upgrade(&self) -> Option<Bridge> {
        self.0.upgrade().map(Bridge)
    }
}

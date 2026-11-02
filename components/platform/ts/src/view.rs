//! How a Rust-composed view crosses to TypeScript and back.
//!
//! `AnyView` is not `Clone` — a view is consumed when it is realized — so a
//! view handed to JavaScript cannot be a copy. It crosses as a *slot*: an
//! opaque box JavaScript stores and passes along, holding the view until
//! whoever consumes it takes it out. Taking it twice is an error rather than a
//! silent second view, which is what makes `<Box>`-style double slotting and a
//! handle used after it was consumed fail where the mistake is.
//!
//! The same slot is the host handle of HOST.md: `create` hands one back, and
//! `modify` takes the view out of the slot it is given and returns a new one.

use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::handler::ViewBuilder;
use waterui_core::{AnyView, Metadata, Retain};
use waterui_ts_engine::{JsError, JsFunction, JsValue, Opaque};

use crate::bridge::{Bridge, WeakBridge};

/// An opaque handle to one `AnyView`, taken exactly once.
#[derive(Debug, Clone)]
pub struct ViewSlot(Rc<RefCell<Option<AnyView>>>);

impl ViewSlot {
    /// Puts `view` in a fresh slot.
    #[must_use]
    pub fn new(view: AnyView) -> Self {
        Self(Rc::new(RefCell::new(Some(view))))
    }

    /// Takes the view out.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the slot is already empty: a view reached two
    /// places in the tree, or a handle was used after the view was realized.
    pub fn take(&self) -> Result<AnyView, JsError> {
        self.0.borrow_mut().take().ok_or_else(|| {
            JsError::new(
                "TypeError",
                "this view was already taken: a Rust view crosses into TypeScript once, and a \
                 host handle is consumed by the call that receives it",
            )
        })
    }

    /// Whether the slot still holds its view.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.0.borrow().is_some()
    }

    /// The value JavaScript holds this slot as.
    #[must_use]
    pub fn to_js_value(&self) -> JsValue {
        JsValue::Opaque(Opaque::new(Rc::clone(&self.0)))
    }

    /// Recovers the slot from the value JavaScript passed back.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the value is not a view slot — a child that
    /// should have been a view, a handle from a different kind of host call.
    pub fn from_js_value(value: &JsValue) -> Result<Self, JsError> {
        let JsValue::Opaque(opaque) = value else {
            return Err(JsError::conversion(format!(
                "expected a view, found {}",
                crate::error::kind_of(value)
            )));
        };
        opaque
            .downcast::<RefCell<Option<AnyView>>>()
            .map(Self)
            .ok_or_else(|| {
                JsError::conversion(
                    "expected a view, found a native handle of another kind".to_owned(),
                )
            })
    }
}

/// A JavaScript function that builds a fresh view each time it is called.
///
/// A [`ViewSlot`] is a subtree that already exists and is taken once. A
/// navigation destination or a tab's root is not that: `ViewBuilder::build`
/// may run again whenever the container needs the view — a destination pushed
/// twice, a tab shown after it was torn down — and each build needs its own
/// subtree. So what crosses is the function, and every build calls it and
/// takes the slot it returned.
///
/// Each build runs under a scope of its own, closed into the built subtree, so
/// whatever the function exports into JavaScript while building — a signal it
/// materializes, a callback it registers — is released when that subtree is
/// dropped. Owned by the mount instead, a destination entered and left
/// repeatedly would leave one set of exports behind per visit.
///
/// The bridge is held weakly, because the function lives inside the engine
/// that lives inside the bridge. A build after the runtime is gone is an
/// error, not a crash.
#[derive(Clone)]
pub struct JsViewBuilder {
    function: JsFunction,
    bridge: WeakBridge,
}

impl core::fmt::Debug for JsViewBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JsViewBuilder").finish_non_exhaustive()
    }
}

impl JsViewBuilder {
    /// Wraps a JavaScript render function.
    #[must_use]
    pub fn new(function: JsFunction, bridge: &Bridge) -> Self {
        Self {
            function,
            bridge: bridge.downgrade(),
        }
    }

    /// The function this builder calls.
    #[must_use]
    pub fn function(&self) -> JsFunction {
        self.function.clone()
    }

    /// Calls the function and takes the view it built.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the runtime is gone, when the function throws,
    /// or when what it returned is not a view.
    pub fn try_build(&self) -> Result<AnyView, JsError> {
        let bridge = self.bridge.upgrade().ok_or_else(|| {
            JsError::new(
                "Error",
                "a TypeScript view builder was asked for a view after its runtime was dropped",
            )
        })?;
        let scope = bridge.open_scope();
        let built = bridge.call(&self.function, &[])?;
        let exports = scope.close();
        let view = ViewSlot::from_js_value(&built)?.take()?;
        Ok(AnyView::new(Metadata::new(view, Retain::new(exports))))
    }
}

impl ViewBuilder for JsViewBuilder {
    type Output = AnyView;

    /// # Panics
    ///
    /// Panics when the function throws or hands back something that is not a
    /// view. `ViewBuilder::build` has no error channel, and a container that
    /// silently showed nothing where a destination belongs would hide the
    /// throw at the point where it is still diagnosable.
    fn build(&self) -> Self::Output {
        self.try_build()
            .unwrap_or_else(|error| panic!("a TypeScript view builder failed: {error}"))
    }
}

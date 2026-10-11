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

use waterui_core::AnyView;
use waterui_ts_engine::{JsError, JsValue, Opaque};

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

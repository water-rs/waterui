//! The retained-value payload behind `JsFunction` and `JsObject`.
//!
//! The payload is the engine's own reference — `JSValue` on JavaScriptCore,
//! a `Persistent` plus its `Context` on QuickJS-NG so the handle keeps its
//! runtime alive — carried as `Rc<dyn Any>` so this crate stays engine-free.
//! Engine crates downcast back to their own type; `None` means the handle
//! belongs to a different engine, which they treat as an error.

use std::any::Any;
use std::fmt;
use std::rc::Rc;

/// A type-erased engine reference, cheap to clone.
#[derive(Clone)]
pub struct Handle(Rc<dyn Any>);

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Handle(..)")
    }
}

impl Handle {
    /// Wraps an engine reference.
    pub fn new<T: 'static>(value: T) -> Self {
        Self(Rc::new(value))
    }

    /// The reference back as the engine's own type, or `None` when the handle
    /// belongs to a different engine.
    #[must_use]
    pub fn downcast<T: 'static>(&self) -> Option<Rc<T>> {
        self.0.clone().downcast::<T>().ok()
    }

    /// Whether two handles retain the same engine reference.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

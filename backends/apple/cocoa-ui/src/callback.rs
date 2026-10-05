//! The guard every closure called from Objective-C runs inside.

use std::any::Any;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

/// Runs `callback`, aborting the process if it panics.
///
/// A panic must not unwind into the Objective-C frames that called us: that
/// is undefined behaviour, and the interface the callback was updating is left
/// half-changed anyway. The panic message is logged with the `site` it came
/// from, then the process aborts.
pub fn guarded<R>(site: &'static str, callback: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(payload) => {
            tracing::error!(
                site,
                message = panic_message(payload.as_ref()),
                "a callback from Objective-C panicked; aborting the process"
            );
            std::process::abort()
        }
    }
}

/// Runs `super_call`, then the slot's handler when one is set — the
/// whole callback inside one [`guarded`] boundary under `site`.
pub(crate) fn forward(
    site: &'static str,
    super_call: impl FnOnce(),
    slot: &RefCell<Option<Rc<dyn Fn()>>>,
) {
    guarded(site, || {
        super_call();
        let handler = slot.borrow().clone();
        if let Some(handler) = handler {
            handler();
        }
    });
}

/// Calls the slot's handler with `event` when one is set.
///
/// Opens no boundary of its own: dispatch always happens inside the
/// calling entry point's `guarded`, so each callback crosses exactly
/// one guard.
pub(crate) fn emit<T>(slot: &RefCell<Option<Rc<dyn Fn(T)>>>, event: T) {
    let handler = slot.borrow().clone();
    if let Some(handler) = handler {
        handler(event);
    }
}

/// The text a panic carried, when it carried text.
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("the panic payload is not a string")
}

#[cfg(test)]
mod tests {
    use super::panic_message;

    #[test]
    fn reads_both_kinds_of_panic_text() {
        let literal: Box<dyn std::any::Any + Send> = Box::new("literal");
        let formatted: Box<dyn std::any::Any + Send> = Box::new(String::from("formatted"));
        let other: Box<dyn std::any::Any + Send> = Box::new(7_u8);
        assert_eq!(panic_message(literal.as_ref()), "literal");
        assert_eq!(panic_message(formatted.as_ref()), "formatted");
        assert_eq!(
            panic_message(other.as_ref()),
            "the panic payload is not a string"
        );
    }
}

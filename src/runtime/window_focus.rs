//! A window's focus channel.
//!
//! One [`FocusRequest`] rides inside every [`Window`](crate::window::Window)
//! and is shared by `Rc` with the [`WindowHandle`](crate::window::WindowHandle)s
//! the window hands out — the per-window sibling of the close machine in
//! `window_close`, only simpler: a focus request is an action the platform
//! performs or declines, with no question, no reply and no exclusivity to
//! arbitrate. The backend arms the channel when it realizes the window, and
//! [`WindowHandle::focus`](crate::window::WindowHandle::focus) files
//! requests into it.
//!
//! A backend whose platform cannot activate a mapped window — a headless
//! runner, say — never arms the machine, and the handle's call panics
//! there, the same contract `WindowHandle::request_close` documents for
//! platforms without a close request.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::fmt;

/// The machine's share: the armed hook, the dispatch flag that re-entrant
/// requests check, and the requests they queued for the in-flight
/// dispatch to drain.
struct Shared {
    /// The platform's realization of a request; `None` until a backend
    /// arms the machine and again while it runs.
    hook: Option<Box<dyn FnMut()>>,
    /// Set when the backend realizes the window — tracked separately from
    /// the hook because the hook leaves its slot while it runs and must
    /// still count as armed for a request filed inside it.
    armed: bool,
    /// A request is dispatching: later requests queue instead of
    /// re-entering the `FnMut` hook.
    dispatching: bool,
    /// Requests filed while `dispatching` — drained in order by the
    /// in-flight request before it returns.
    queued: usize,
}

/// The focus machine one [`Window`](crate::window::Window) carries.
///
/// Cloning shares the machine: every [`WindowHandle`](crate::window::WindowHandle)
/// files requests into the machine the backend armed at realization.
#[derive(Clone)]
pub struct FocusRequest {
    inner: Rc<RefCell<Shared>>,
}

impl fmt::Debug for FocusRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shared = self.inner.borrow();
        f.debug_struct("FocusRequest")
            .field("armed", &shared.armed)
            .field("dispatching", &shared.dispatching)
            .field("queued", &shared.queued)
            .finish()
    }
}

/// The flag-clearing seat one dispatch holds for its whole run —
/// `dispatching` resets on the normal exit and through unwind alike.
struct Dispatch<'a> {
    shared: &'a RefCell<Shared>,
}

impl Drop for Dispatch<'_> {
    fn drop(&mut self) {
        self.shared.borrow_mut().dispatching = false;
    }
}

/// Re-seats the hook the running request took out — through unwind too,
/// so a hook that panics declining a request it cannot perform leaves the
/// machine armed for the kinds it can.
struct PutBack<'a> {
    shared: &'a RefCell<Shared>,
    hook: Option<Box<dyn FnMut()>>,
}

impl Drop for PutBack<'_> {
    fn drop(&mut self) {
        self.shared.borrow_mut().hook = self.hook.take();
    }
}

impl FocusRequest {
    /// The machine a fresh [`Window`](crate::window::Window) carries —
    /// unarmed until a backend realizes the window.
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Shared {
                hook: None,
                armed: false,
                dispatching: false,
                queued: 0,
            })),
        }
    }

    /// Records the hook that performs a request on the platform window —
    /// the realization call sites make it.
    pub(crate) fn arm(&self, hook: impl FnMut() + 'static) {
        let mut shared = self.inner.borrow_mut();
        shared.hook = Some(Box::new(hook));
        shared.armed = true;
    }

    /// Files a request: the armed hook runs now, or — filed re-entrantly
    /// from inside it — queued for the in-flight dispatch to drain.
    ///
    /// The hook runs unborrowed — taken out of the cell first — so the
    /// platform code inside it may publish back into the window's own
    /// bindings (a did-become-key writing focus state, say) and even file
    /// a further request, which queues rather than re-entering the
    /// `FnMut`. A hook that panics unwinds through the guards, which
    /// re-seat it and clear `dispatching`: the machine stays armed for
    /// the requests that follow.
    ///
    /// # Panics
    ///
    /// When the machine was never armed — the window was never realized,
    /// or its platform cannot activate a mapped window — or the hook
    /// itself panicked.
    pub(crate) fn request(&self) {
        {
            let mut shared = self.inner.borrow_mut();
            assert!(
                shared.armed,
                "a focus request was filed for a window that was never \
                 realized, or whose platform cannot activate a mapped window",
            );
            shared.queued += 1;
            if shared.dispatching {
                return;
            }
            shared.dispatching = true;
        }
        let _dispatch = Dispatch {
            shared: &self.inner,
        };
        loop {
            let hook = {
                let mut shared = self.inner.borrow_mut();
                if shared.queued == 0 {
                    return;
                }
                shared.queued -= 1;
                shared
                    .hook
                    .take()
                    .expect("an armed machine always holds its hook")
            };
            let mut put_back = PutBack {
                shared: &self.inner,
                hook: Some(hook),
            };
            (put_back
                .hook
                .as_mut()
                .expect("the hook leaves its slot only in drop"))();
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::FocusRequest;

    /// A filed request reaches the armed hook.
    #[test]
    fn a_request_runs_the_armed_hook() {
        let machine = FocusRequest::new();
        let calls = Rc::new(Cell::new(0u32));
        machine.arm({
            let calls = calls.clone();
            move || calls.set(calls.get() + 1)
        });
        machine.request();
        assert_eq!(calls.get(), 1);
    }

    /// The machine stays armed after a request — the next one reaches the
    /// same hook.
    #[test]
    fn the_machine_stays_armed_between_requests() {
        let machine = FocusRequest::new();
        let calls = Rc::new(Cell::new(0u32));
        machine.arm({
            let calls = calls.clone();
            move || calls.set(calls.get() + 1)
        });
        machine.request();
        machine.request();
        assert_eq!(calls.get(), 2);
    }

    /// A request filed inside the hook queues and drains in order —
    /// unborrowed hooks can publish back into app code that files again.
    #[test]
    fn a_hook_may_file_a_nested_request() {
        let machine = FocusRequest::new();
        let nested = machine.clone();
        let calls = Rc::new(Cell::new(0u32));
        machine.arm({
            let calls = calls.clone();
            move || {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    nested.request();
                }
            }
        });
        machine.request();
        assert_eq!(calls.get(), 2);
    }

    /// A hook that panics leaves the machine armed — the requests that
    /// follow still reach it.
    #[test]
    fn a_panicking_hook_leaves_the_machine_armed() {
        let machine = FocusRequest::new();
        let calls = Rc::new(Cell::new(0u32));
        machine.arm({
            let calls = calls.clone();
            move || {
                calls.set(calls.get() + 1);
                assert!(calls.get() != 1, "the platform declined the request");
            }
        });
        assert!(catch_unwind(AssertUnwindSafe(|| machine.request())).is_err());
        machine.request();
        assert_eq!(calls.get(), 2);
    }

    /// A request on a never-armed machine panics, as documented.
    #[test]
    #[should_panic(expected = "never realized")]
    fn a_request_before_realization_panics() {
        FocusRequest::new().request();
    }
}

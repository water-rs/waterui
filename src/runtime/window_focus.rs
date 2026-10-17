//! A window's raise/focus channel.
//!
//! One [`FocusRequest`] rides inside every [`Window`](crate::window::Window)
//! and is shared by `Rc` with the [`WindowHandle`](crate::window::WindowHandle)s
//! the window hands out — the per-window sibling of the close machine in
//! `window_close`, only simpler: a raise/focus request is an action the
//! platform performs or declines, with no question, no reply and no
//! exclusivity to arbitrate. The backend arms the channel when it realizes
//! the window; [`WindowHandle::raise`](crate::window::WindowHandle::raise)
//! and [`WindowHandle::focus`](crate::window::WindowHandle::focus) file
//! requests into the same machine.
//!
//! A backend whose platform cannot move a mapped window — a headless
//! runner, a compositor that exposes no raise primitive — never arms the
//! machine, and the handle calls panic there, the same contract
//! `WindowHandle::request_close` documents for platforms without a close
//! request.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::fmt;

/// What a [`WindowHandle`](crate::window::WindowHandle) asks the platform
/// to do with a mapped window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each request kind by kind on every platform"
)]
pub enum FocusRequestKind {
    /// Order the window in front of its siblings, leaving keyboard focus
    /// where it is.
    Raise,
    /// Order the window in front and give it keyboard focus — activating
    /// the application where the platform couples the two.
    Focus,
}

/// The machine's share: the armed hook, the dispatch flag that re-entrant
/// requests check, and the requests they queued for the in-flight
/// dispatch to drain.
struct Shared {
    /// The platform's realization of a request; `None` until a backend
    /// arms the machine and again while it runs.
    hook: Option<Box<dyn FnMut(FocusRequestKind)>>,
    /// Set when the backend realizes the window — tracked separately from
    /// the hook because the hook leaves its slot while it runs and must
    /// still count as armed for a request filed inside it.
    armed: bool,
    /// A request is dispatching: later requests queue instead of
    /// re-entering the `FnMut` hook.
    dispatching: bool,
    /// Requests filed while `dispatching` — drained in order by the
    /// in-flight request before it returns.
    queued: VecDeque<FocusRequestKind>,
}

/// The raise/focus machine one [`Window`](crate::window::Window) carries.
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
            .field("queued", &shared.queued.len())
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
    hook: Option<Box<dyn FnMut(FocusRequestKind)>>,
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
                queued: VecDeque::new(),
            })),
        }
    }

    /// Records the hook that performs a request on the platform window —
    /// the realization call sites make it.
    pub(crate) fn arm(&self, hook: impl FnMut(FocusRequestKind) + 'static) {
        let mut shared = self.inner.borrow_mut();
        shared.hook = Some(Box::new(hook));
        shared.armed = true;
    }

    /// Files `kind`: the armed hook runs now, or — filed re-entrantly
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
    /// or its platform cannot move a mapped window — or the hook itself
    /// panicked: a platform that cannot perform the requested kind may
    /// say so rather than silently deliver the wrong one.
    pub(crate) fn request(&self, kind: FocusRequestKind) {
        {
            let mut shared = self.inner.borrow_mut();
            assert!(
                shared.armed,
                "a raise/focus request was filed for a window that was never \
                 realized, or whose platform cannot move a mapped window",
            );
            shared.queued.push_back(kind);
            if shared.dispatching {
                return;
            }
            shared.dispatching = true;
        }
        let _dispatch = Dispatch {
            shared: &self.inner,
        };
        loop {
            let work = {
                let mut shared = self.inner.borrow_mut();
                shared.queued.pop_front().map(|kind| {
                    (
                        shared
                            .hook
                            .take()
                            .expect("an armed machine always holds its hook"),
                        kind,
                    )
                })
            };
            let Some((hook, kind)) = work else {
                return;
            };
            let mut put_back = PutBack {
                shared: &self.inner,
                hook: Some(hook),
            };
            (put_back
                .hook
                .as_mut()
                .expect("the hook leaves its slot only in drop"))(kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::{FocusRequest, FocusRequestKind};

    /// A filed request reaches the armed hook.
    #[test]
    fn a_request_runs_the_armed_hook() {
        let machine = FocusRequest::new();
        let seen = Rc::new(Cell::new(0u8));
        machine.arm({
            let seen = seen.clone();
            move |kind| {
                seen.set(
                    seen.get()
                        | match kind {
                            FocusRequestKind::Raise => 0b01,
                            FocusRequestKind::Focus => 0b10,
                        },
                );
            }
        });
        machine.request(FocusRequestKind::Raise);
        machine.request(FocusRequestKind::Focus);
        assert_eq!(seen.get(), 0b11);
    }

    /// The machine stays armed after a request — the next one reaches the
    /// same hook.
    #[test]
    fn the_machine_stays_armed_between_requests() {
        let machine = FocusRequest::new();
        let calls = Rc::new(Cell::new(0u32));
        machine.arm({
            let calls = calls.clone();
            move |_kind| calls.set(calls.get() + 1)
        });
        machine.request(FocusRequestKind::Focus);
        machine.request(FocusRequestKind::Raise);
        assert_eq!(calls.get(), 2);
    }

    /// A request filed inside the hook queues and drains in order —
    /// unborrowed hooks can publish back into app code that files again.
    #[test]
    fn a_hook_may_file_a_nested_request() {
        let machine = FocusRequest::new();
        let nested = machine.clone();
        let calls = Rc::new(core::cell::RefCell::new(Vec::new()));
        machine.arm({
            let calls = calls.clone();
            move |kind| {
                calls.borrow_mut().push(kind);
                if kind == FocusRequestKind::Raise {
                    nested.request(FocusRequestKind::Focus);
                }
            }
        });
        machine.request(FocusRequestKind::Raise);
        assert_eq!(
            calls.borrow().as_slice(),
            &[FocusRequestKind::Raise, FocusRequestKind::Focus]
        );
    }

    /// A hook that panics declining a request leaves the machine armed —
    /// the kinds it can perform still work afterwards.
    #[test]
    fn a_panicking_hook_leaves_the_machine_armed() {
        let machine = FocusRequest::new();
        machine.arm(|kind| match kind {
            FocusRequestKind::Raise => panic!("no raise primitive"),
            FocusRequestKind::Focus => {}
        });
        assert!(
            catch_unwind(AssertUnwindSafe(|| machine.request(FocusRequestKind::Raise))).is_err()
        );
        machine.request(FocusRequestKind::Focus);
    }

    /// A request on a never-armed machine panics, as documented.
    #[test]
    #[should_panic(expected = "never realized")]
    fn a_request_before_realization_panics() {
        FocusRequest::new().request(FocusRequestKind::Focus);
    }
}

//! Application lifecycle phase and phase-change hooks.
//!
//! One [`Phase`] rides inside [`AppParts`](crate::app::AppParts) to the runner
//! that hosts the application. [`App::new`](crate::app::App::new) inserts its
//! live [`Computed<AppPhase>`] into the application environment so views and
//! handlers can read or react to the current phase anywhere in the tree.
//!
//! Once the runner has assembled its composition-root environment, it starts
//! the machine with [`Phase::start`] or [`Phase::start_with`] and reports
//! every platform lifecycle change through the returned [`PhaseHandle`].
//! Redundant transitions (`current == next`) are ignored.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use nami::{Binding, Computed, Signal as _, SignalExt as _, impl_constant};
use waterui_core::Environment;

use crate::task::spawn_local;

/// The lifecycle phase of a running application.
///
/// Every platform runner maps its native foreground/inactive/background
/// lifecycle onto these three phases. The live phase is installed in the
/// application [`Environment`] as [`Computed<AppPhase>`] and delivered to
/// [`App::on_phase_change`](crate::app::App::on_phase_change) hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every runner maps its platform lifecycle onto these three phases; a new phase must fail their build instead of falling into a wildcard arm"
)]
pub enum AppPhase {
    /// The application is in the foreground and actively interacting with the
    /// user.
    #[default]
    Active,
    /// The application is visible or transitioning, but not currently active
    /// (for example unfocused on desktop, covered by Control Center or the App
    /// Switcher on iOS, paused while still started on Android, or a visible
    /// unfocused browser tab).
    Inactive,
    /// The application is hidden or backgrounded (for example backgrounded on
    /// iOS or Android, hidden on macOS, occluded or suspended on desktop, or a
    /// hidden browser tab).
    ///
    /// On mobile platforms this is the last reliable moment to persist state
    /// before the operating system suspends or terminates the process.
    Background,
}

impl_constant!(AppPhase);

/// One phase-change hook erased together with the future it produces.
///
/// Returns `None` when parameter extraction fails (after reporting the failure
/// through [`ReportExtraction`](waterui_core::handler::ReportExtraction)).
pub type PhaseChangeHook =
    Box<dyn FnMut(AppPhase, &Environment) -> Option<Pin<Box<dyn Future<Output = ()>>>>>;

/// The application's lifecycle phase binding and registered
/// [`App::on_phase_change`](crate::app::App::on_phase_change) hooks, carried
/// to a runner inside [`AppParts`](crate::app::AppParts).
pub struct Phase {
    binding: Binding<AppPhase>,
    hooks: Vec<PhaseChangeHook>,
}

impl Default for Phase {
    fn default() -> Self {
        Self {
            binding: Binding::new(AppPhase::default()),
            hooks: Vec::new(),
        }
    }
}

impl fmt::Debug for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Phase")
            .field("phase", &self.binding.snapshot())
            .field("hook_count", &self.hooks.len())
            .finish()
    }
}

impl Phase {
    /// Returns the read-only reactive signal tracking the application's phase.
    #[must_use]
    pub fn computed(&self) -> Computed<AppPhase> {
        self.binding.computed()
    }

    pub(crate) fn push_hook(&mut self, hook: PhaseChangeHook) {
        self.hooks.push(hook);
    }

    /// Starts the lifecycle phase machine using the current binding value
    /// ([`AppPhase::Active`]) as the initial phase.
    ///
    /// `env` is the runner's fully assembled composition-root environment that
    /// `on_phase_change` extractors read from.
    #[must_use]
    pub fn start(self, env: &Environment) -> PhaseHandle {
        let initial = self.binding.snapshot();
        self.start_with(env, initial)
    }

    /// Starts the lifecycle phase machine seeded with `initial` as the
    /// platform's current phase.
    ///
    /// Seeding updates the live [`Computed<AppPhase>`] binding if `initial`
    /// differs from its default, without firing `on_phase_change` hooks.
    /// Subsequent real transitions (`current != next`) update the binding and
    /// invoke every registered hook.
    #[must_use]
    pub fn start_with(self, env: &Environment, initial: AppPhase) -> PhaseHandle {
        if self.binding.snapshot() != initial {
            self.binding.set(initial);
        }
        PhaseHandle {
            inner: Rc::new(RefCell::new(Shared {
                current: initial,
                binding: self.binding,
                env: env.clone(),
                hooks: self.hooks,
            })),
        }
    }
}

struct Shared {
    current: AppPhase,
    binding: Binding<AppPhase>,
    env: Environment,
    hooks: Vec<PhaseChangeHook>,
}

/// The runner's handle to the live [`AppPhase`] state and its
/// `on_phase_change` hooks.
///
/// Cloning shares the same underlying state so every platform observer (scene
/// delegates, notification observers, focus/visibility listeners) reports into
/// one deduplicated phase machine.
#[derive(Clone)]
pub struct PhaseHandle {
    inner: Rc<RefCell<Shared>>,
}

impl fmt::Debug for PhaseHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhaseHandle")
            .field("current", &self.inner.borrow().current)
            .finish_non_exhaustive()
    }
}

impl PhaseHandle {
    /// Returns the most recently reported [`AppPhase`].
    #[must_use]
    pub fn current(&self) -> AppPhase {
        self.inner.borrow().current
    }

    /// Returns the read-only reactive signal tracking the live phase.
    #[must_use]
    pub fn computed(&self) -> Computed<AppPhase> {
        self.inner.borrow().binding.computed()
    }

    /// Transitions to `next` if it differs from the current phase, returning a
    /// future that runs all registered `on_phase_change` hooks concurrently.
    ///
    /// Returns `None` when `next` equals the current phase (deduplicated) or
    /// when no hooks produced work. Runners that need special execution
    /// wrapping — such as an iOS background task assertion, an Android
    /// synchronous `onStop` pump, or a hidden-tab web microtask spawn — drive
    /// the returned future directly.
    #[must_use]
    pub fn transition_future(&self, next: AppPhase) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
        let binding = {
            let mut shared = self.inner.borrow_mut();
            if shared.current == next {
                return None;
            }
            shared.current = next;
            shared.binding.clone()
        };
        // Release the RefCell borrow before notifying signal watchers so
        // watchers can safely read or re-enter the phase handle.
        binding.set(next);

        let (mut hooks, env) = {
            let mut shared = self.inner.borrow_mut();
            (core::mem::take(&mut shared.hooks), shared.env.clone())
        };
        let mut futures = Vec::with_capacity(hooks.len());
        for hook in &mut hooks {
            if let Some(future) = hook(next, &env) {
                futures.push(future);
            }
        }
        {
            let mut shared = self.inner.borrow_mut();
            if !shared.hooks.is_empty() {
                hooks.append(&mut shared.hooks);
            }
            shared.hooks = hooks;
        }

        match futures.len() {
            0 => None,
            1 => futures.pop(),
            _ => Some(Box::pin(JoinAll {
                futures: futures.into_iter().map(Some).collect(),
            })),
        }
    }

    /// Transitions to `next` if it differs from the current phase, spawning
    /// any registered `on_phase_change` hooks onto the runner's local
    /// executor.
    pub fn set_phase(&self, next: AppPhase) {
        if let Some(work) = self.transition_future(next) {
            spawn_local(work).detach();
        }
    }
}

/// Polls a fixed set of independent `on_phase_change` hook futures
/// concurrently until every one completes.
struct JoinAll {
    futures: Vec<Option<Pin<Box<dyn Future<Output = ()>>>>>,
}

impl Future for JoinAll {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut all_done = true;
        for slot in &mut self.futures {
            if let Some(future) = slot.as_mut() {
                if future.as_mut().poll(cx).is_ready() {
                    *slot = None;
                } else {
                    all_done = false;
                }
            }
        }
        if all_done {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::{Cell, RefCell};

    use waterui_core::{Environment, State};

    use super::super::parked_executor::{drain, install as install_executor};
    use super::*;
    use crate::app::App;

    #[test]
    fn app_new_installs_computed_app_phase_defaulting_to_active() {
        let parts = App::new(Environment::new()).into_parts();
        let phase = parts
            .env
            .get::<Computed<AppPhase>>()
            .expect("App::new must install Computed<AppPhase> in env");
        assert_eq!(phase.snapshot(), AppPhase::Active);
        assert_eq!(
            parts
                .env
                .extract::<State<Computed<AppPhase>>>()
                .expect("State<Computed<AppPhase>> must be extractable")
                .snapshot(),
            AppPhase::Active
        );
    }

    #[test]
    fn phase_transitions_update_computed_and_deduplicate() {
        install_executor();
        let observed_signal = Rc::new(RefCell::new(Vec::new()));
        let observed_hook = Rc::new(RefCell::new(Vec::new()));

        let hook_log = Rc::clone(&observed_hook);
        let parts = App::new(Environment::new())
            .on_phase_change(move |phase: AppPhase| {
                let hook_log = Rc::clone(&hook_log);
                async move {
                    hook_log.borrow_mut().push(phase);
                }
            })
            .into_parts();

        let signal = parts
            .env
            .get::<Computed<AppPhase>>()
            .expect("Computed<AppPhase> installed")
            .clone();
        let signal_log = Rc::clone(&observed_signal);
        let _guard = signal.watch(move |ctx| {
            signal_log.borrow_mut().push(ctx.into_value());
        });

        let handle = parts.phase.start(&parts.env);

        // Redundant transition to Active does nothing.
        handle.set_phase(AppPhase::Active);
        drain();
        assert!(observed_signal.borrow().is_empty());
        assert!(observed_hook.borrow().is_empty());

        handle.set_phase(AppPhase::Inactive);
        handle.set_phase(AppPhase::Inactive); // deduplicated
        handle.set_phase(AppPhase::Background);
        handle.set_phase(AppPhase::Background); // deduplicated
        handle.set_phase(AppPhase::Active);
        drain();

        assert_eq!(signal.snapshot(), AppPhase::Active);
        assert_eq!(
            *observed_signal.borrow(),
            [AppPhase::Inactive, AppPhase::Background, AppPhase::Active]
        );
        assert_eq!(
            *observed_hook.borrow(),
            [AppPhase::Inactive, AppPhase::Background, AppPhase::Active]
        );
    }

    #[test]
    fn start_with_seeds_initial_phase_without_firing_hooks() {
        install_executor();
        let fired = Rc::new(Cell::new(0_u32));
        let fired_hook = Rc::clone(&fired);
        let parts = App::new(Environment::new())
            .on_phase_change(move |_phase: AppPhase| {
                let fired_hook = Rc::clone(&fired_hook);
                async move {
                    fired_hook.set(fired_hook.get() + 1);
                }
            })
            .into_parts();

        let signal = parts.env.get::<Computed<AppPhase>>().unwrap().clone();
        let handle = parts.phase.start_with(&parts.env, AppPhase::Background);
        drain();

        assert_eq!(handle.current(), AppPhase::Background);
        assert_eq!(signal.snapshot(), AppPhase::Background);
        assert_eq!(fired.get(), 0, "initial seeding must not fire hooks");

        handle.set_phase(AppPhase::Background);
        drain();
        assert_eq!(fired.get(), 0, "redundant phase must not fire hooks");

        handle.set_phase(AppPhase::Inactive);
        drain();
        assert_eq!(fired.get(), 1);
        assert_eq!(signal.snapshot(), AppPhase::Inactive);
    }

    #[test]
    fn chained_on_phase_change_hooks_all_run_and_extract_app_state() {
        #[waterui_macros::state]
        #[derive(Clone)]
        struct Store {
            saves: Rc<Cell<u32>>,
        }

        impl Store {
            fn record_save(&self) {
                self.saves.set(self.saves.get() + 1);
            }
        }

        install_executor();

        let saves = Rc::new(Cell::new(0_u32));
        let store = Store {
            saves: Rc::clone(&saves),
        };
        let second_log = Rc::new(RefCell::new(Vec::new()));
        let second_log_for_hook = Rc::clone(&second_log);

        let parts = App::new(Environment::new())
            .state(&store)
            .on_phase_change(|phase: AppPhase, store: Store| async move {
                if phase == AppPhase::Background {
                    core::future::ready(()).await;
                    store.record_save();
                }
            })
            .on_phase_change(move |phase: AppPhase, State(raw_store): State<Store>| {
                let second_log_for_hook = Rc::clone(&second_log_for_hook);
                async move {
                    second_log_for_hook
                        .borrow_mut()
                        .push((phase, raw_store.saves.get()));
                }
            })
            .into_parts();

        let handle = parts.phase.start(&parts.env);
        handle.set_phase(AppPhase::Background);
        drain();

        assert_eq!(saves.get(), 1);
        assert_eq!(second_log.borrow().len(), 1);
        assert_eq!(second_log.borrow()[0].0, AppPhase::Background);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "failed to extract"))]
    fn missing_extractor_reports_and_skips_hook() {
        install_executor();
        let ran = Rc::new(Cell::new(false));
        let ran_hook = Rc::clone(&ran);

        let parts = App::new(Environment::new())
            .on_phase_change(move |_phase: AppPhase, _: State<u32>| {
                let ran_hook = Rc::clone(&ran_hook);
                async move {
                    ran_hook.set(true);
                }
            })
            .into_parts();

        let handle = parts.phase.start(&parts.env);
        handle.set_phase(AppPhase::Background);
        drain();
        assert!(!ran.get());
    }
}

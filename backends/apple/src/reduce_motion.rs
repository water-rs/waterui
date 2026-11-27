//! Reactive platform Reduce Motion preference installation.

use cocoa_ui::MainThreadMarker;
use cocoa_ui::reduce_motion::{observe_changes, system_reduce_motion};
use nami::{Binding, Signal as _};
use waterui_backend_core::Environment;
use waterui_structural_transition::ReduceMotion;

/// Syncs `binding` with `reduced`, skipping writes that repeat the held value.
fn sync(binding: &Binding<bool>, reduced: bool) {
    if binding.snapshot() != reduced {
        binding.set(reduced);
    }
}

/// Installs the system's Reduce Motion preference as a reactive [`ReduceMotion`]
/// in `env` and returns a keep-alive for the system observer.
#[must_use]
pub fn install(env: &mut Environment, mtm: MainThreadMarker) -> impl Sized + use<> {
    let binding = Binding::new(system_reduce_motion());
    env.insert(ReduceMotion::new(binding.clone()));
    observe_changes(mtm, move |reduced| sync(&binding, reduced))
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use alloc::rc::Rc;
    use nami::{Binding, Signal as _};
    use waterui_backend_core::Environment;
    use waterui_structural_transition::ReduceMotion;

    #[test]
    fn sync_updates_binding_only_on_change() {
        let setting = Binding::new(false);
        let flag = ReduceMotion::new(setting.clone());
        let mut env = Environment::new();
        env.insert(flag.clone());

        let notifications = Rc::new(Cell::new(0_u32));
        let guard = flag.watch({
            let notifications = Rc::clone(&notifications);
            move |_| notifications.set(notifications.get() + 1)
        });

        super::sync(&setting, false);
        assert_eq!(notifications.get(), 0);
        assert!(!ReduceMotion::is_reduced(&env));

        super::sync(&setting, true);
        assert!(ReduceMotion::is_reduced(&env));
        assert_eq!(notifications.get(), 1);

        super::sync(&setting, true);
        assert_eq!(notifications.get(), 1);

        super::sync(&setting, false);
        assert!(!ReduceMotion::is_reduced(&env));
        assert_eq!(notifications.get(), 2);
        drop(guard);
    }
}

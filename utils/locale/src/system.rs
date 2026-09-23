//! Bridge between regional callbacks and `WaterUI` locale signals.

use core::str::FromStr;
use std::cell::RefCell;
use std::mem::ManuallyDrop;

use nami::{Binding, Container};

use crate::locale::Locale;
use crate::regional::{self, ListenerHandle};

thread_local! {
    static RUNTIME_LOCALE_STATE: RefCell<Option<RuntimeLocaleState>> = const { RefCell::new(None) };
}

struct RuntimeLocaleState {
    binding: Binding<Locale>,
    listener: Option<ManuallyDrop<ListenerHandle>>,
}

impl RuntimeLocaleState {
    const fn has_listener(&self) -> bool {
        self.listener.is_some()
    }

    const fn set_listener(&mut self, listener: ListenerHandle) {
        self.listener = Some(ManuallyDrop::new(listener));
    }

    fn unregister_listener(&mut self) {
        if let Some(listener) = self.listener.take() {
            ManuallyDrop::into_inner(listener).unregister();
        }
    }
}

pub fn runtime_locale_binding() -> Binding<Locale> {
    RUNTIME_LOCALE_STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(existing) = slot.as_mut() {
            ensure_listener_registered(existing);
            return existing.binding.clone();
        }

        // macOS polling of locale/timezone currently interacts badly with AppKit/LaunchServices
        // and can stall UI responsiveness. Keep locale stable until native notifications are wired.
        #[cfg(not(target_os = "macos"))]
        regional::start_auto_refresh_default();

        let initial = locale_from_tag(regional::current_settings().locale_tag());
        let mut state = RuntimeLocaleState {
            binding: Binding::custom(Container::new(initial)),
            listener: None,
        };
        ensure_listener_registered(&mut state);

        let binding = state.binding.clone();
        *slot = Some(state);

        binding
    })
}

fn locale_from_tag(tag: &str) -> Locale {
    Locale::from_str(tag)
        .unwrap_or_else(|error| panic!("runtime locale tag '{tag}' is invalid: {error}"))
}

fn ensure_listener_registered(state: &mut RuntimeLocaleState) {
    if state.has_listener() {
        return;
    }

    let binding = state.binding.clone();
    let mailbox = binding.mailbox();
    let listener = regional::register_listener(move |context| {
        let locale = locale_from_tag(context.locale_tag());
        mailbox.handle(move |binding| {
            binding.set(locale);
        });
    });
    state.set_listener(listener);
}

fn reset_runtime_locale_state() {
    RUNTIME_LOCALE_STATE.with(|slot| {
        if let Some(mut state) = slot.borrow_mut().take() {
            state.unregister_listener();
        }
    });
}

#[cfg(test)]
/// Resets the per-thread runtime locale cache so tests can start from a clean state.
pub fn reset_runtime_locale_state_for_tests() {
    reset_runtime_locale_state();
}

/// Clears the current thread's runtime locale cache during controlled executor shutdown.
pub fn shutdown_current_thread_runtime_locale_state() {
    reset_runtime_locale_state();
}

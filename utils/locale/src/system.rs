//! Bridge between regional callbacks and WaterUI locale signals.

use core::str::FromStr;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use nami::Binding;

use crate::locale::{Locale, locales};
use crate::regional::{self, ListenerHandle};

thread_local! {
    static RUNTIME_LOCALE_STATE: RefCell<Option<RuntimeLocaleState>> = const { RefCell::new(None) };
}

struct RuntimeLocaleState {
    binding: Binding<Locale>,
    listener: Option<ListenerHandle>,
}

pub fn runtime_locale_binding() -> Binding<Locale> {
    RUNTIME_LOCALE_STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(existing) = slot.as_mut() {
            ensure_listener_registered(existing);
            return existing.binding.clone();
        }

        regional::start_auto_refresh_default();

        let initial = locale_from_tag(regional::current_settings().locale_tag());
        let mut state = RuntimeLocaleState {
            binding: Binding::container(initial),
            listener: None,
        };
        ensure_listener_registered(&mut state);

        let binding = state.binding.clone();
        *slot = Some(state);

        binding
    })
}

fn locale_from_tag(tag: &str) -> Locale {
    Locale::from_str(tag).unwrap_or(locales::EN_US)
}

fn ensure_listener_registered(state: &mut RuntimeLocaleState) {
    if state.listener.is_some() {
        return;
    }

    let binding = state.binding.clone();
    let listener = catch_unwind(AssertUnwindSafe(move || {
        let mailbox = binding.mailbox();
        regional::register_listener(move |context| {
            let locale = locale_from_tag(context.locale_tag());
            mailbox.handle(move |binding| {
                binding.set(locale);
            });
        })
    }))
    .ok();

    state.listener = listener;
}

#[cfg(test)]
pub(crate) fn reset_runtime_locale_state_for_tests() {
    RUNTIME_LOCALE_STATE.with(|slot| {
        let _ = slot.borrow_mut().take();
    });
}

//! Bridge between regional callbacks and WaterUI locale signals.

use core::str::FromStr;
use std::cell::RefCell;

use nami::Binding;

use crate::regional::{self, ListenerHandle};
use crate::locale::{Locale, locales};

thread_local! {
    static RUNTIME_LOCALE_STATE: RefCell<Option<RuntimeLocaleState>> = const { RefCell::new(None) };
}

struct RuntimeLocaleState {
    binding: Binding<Locale>,
    _listener: ListenerHandle,
}

pub(crate) fn runtime_locale_binding() -> Binding<Locale> {
    RUNTIME_LOCALE_STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(existing) = slot.as_ref() {
            return existing.binding.clone();
        }

        regional::start_auto_refresh_default();

        let initial = locale_from_tag(regional::current_settings().locale_tag());
        let binding = Binding::container(initial);
        let mailbox = binding.mailbox();
        let listener = regional::register_listener(move |context| {
            let locale = locale_from_tag(context.locale_tag());
            mailbox.handle(move |binding| {
                binding.set(locale);
            });
        });

        *slot = Some(RuntimeLocaleState {
            binding: binding.clone(),
            _listener: listener,
        });

        binding
    })
}

/// Ensures the runtime locale listener is registered once the executor is ready.
pub fn ensure_runtime_locale_listener_registered() {
    let _ = runtime_locale_binding();
}

fn locale_from_tag(tag: &str) -> Locale {
    Locale::from_str(tag).unwrap_or(locales::EN_US)
}

#[cfg(test)]
pub(crate) fn reset_runtime_locale_state_for_tests() {
    RUNTIME_LOCALE_STATE.with(|slot| {
        let _ = slot.borrow_mut().take();
    });
}

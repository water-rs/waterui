//! Bridge between regional callbacks and WaterUI locale signals.

use core::str::FromStr;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use nami::Binding;
use waterkit_regional::ListenerHandle;

use crate::locale::{Locale, locales};

thread_local! {
    static RUNTIME_LOCALE_BINDING: RefCell<Option<Binding<Locale>>> = const { RefCell::new(None) };
}

static RUNTIME_LOCALE_LISTENER: OnceLock<ListenerHandle> = OnceLock::new();

pub(crate) fn runtime_locale_binding() -> Binding<Locale> {
    let binding = RUNTIME_LOCALE_BINDING.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(existing) = slot.as_ref() {
            return existing.clone();
        }

        let initial = locale_from_tag(waterkit_regional::current_settings().locale_tag());
        let binding = Binding::container(initial);
        *slot = Some(binding.clone());
        binding
    });

    maybe_register_runtime_locale_listener(&binding);
    binding
}

/// Ensures the runtime locale listener is registered once the executor is ready.
pub fn ensure_runtime_locale_listener_registered() {
    let binding = runtime_locale_binding();
    maybe_register_runtime_locale_listener(&binding);
}

fn locale_from_tag(tag: &str) -> Locale {
    Locale::from_str(tag).unwrap_or(locales::EN_US)
}

fn maybe_register_runtime_locale_listener(binding: &Binding<Locale>) {
    if RUNTIME_LOCALE_LISTENER.get().is_some() {
        return;
    }
    waterkit_regional::start_auto_refresh_default();
    let listener = catch_unwind(AssertUnwindSafe(|| {
        let mailbox = binding.mailbox();
        waterkit_regional::register_listener(move |context| {
            let locale = locale_from_tag(context.locale_tag());
            mailbox.handle(move |binding| {
                binding.set(locale);
            });
        })
    }));

    if let Ok(listener) = listener {
        let _ = RUNTIME_LOCALE_LISTENER.set(listener);
    }
}

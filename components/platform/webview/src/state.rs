//! Reactive state shared with the page.
//!
//! A value exposed here is mirrored into JavaScript, so the page reads it as a
//! local property instead of asking across the boundary, and writes flow back
//! into the `Binding` it came from. Direction is decided by the type: a
//! [`Binding`] is read-write, anything else is read-only.
//!
//! Two things this has to get right, both learned from the shape of `nami`:
//!
//! * `Binding::set` notifies unconditionally — `distinct()` is opt-in — so an
//!   echo would oscillate forever. Applying an inbound write suppresses the push
//!   it causes.
//! * A page that writes while a push is in flight must still converge. Each key
//!   carries an epoch; a write reports the epoch it was based on, and if the
//!   authoritative value ends up different the page is corrected.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use waterui_core::{Binding, Computed, Signal};
use waterui_str::Str;

use waterui_core::reactive::watcher::BoxWatcherGuard;

use crate::message::JsReply;

/// The reserved handler the page writes through.
pub const SET_STATE_HANDLER: &str = "__wateruiSetState";

/// A value that can be mirrored into the page.
///
/// Implemented for [`Binding<T>`] (read-write) and [`Computed<T>`] (read-only).
/// Any other signal converts with
/// [`SignalExt::computed`](waterui_core::SignalExt::computed) first; a blanket
/// impl over every signal cannot coexist with these under Rust's coherence rules,
/// and naming the type makes the direction visible at the call site.
pub trait JsField {
    /// The mirrored value type.
    type Value: Serialize + DeserializeOwned + 'static;

    /// Splits into the parts the bridge needs: something to read and, when the
    /// field is writable, something to write.
    fn into_entry(self) -> FieldEntry<Self::Value>;
}

/// A field's read side and, when writable, its write side.
#[expect(
    missing_debug_implementations,
    reason = "holds boxed closures with no useful representation"
)]
pub struct FieldEntry<T> {
    /// The signal the bridge pushes from.
    pub(crate) read: Computed<T>,
    /// Present exactly when the page may assign to this field.
    pub(crate) write: Option<Rc<dyn Fn(T)>>,
}

impl<T: Serialize + DeserializeOwned + Clone + 'static> JsField for Binding<T> {
    type Value = T;

    fn into_entry(self) -> FieldEntry<T> {
        let write = self.clone();
        FieldEntry {
            read: Computed::from(self),
            write: Some(Rc::new(move |value| write.set(value))),
        }
    }
}

impl<T: Serialize + DeserializeOwned + Clone + 'static> JsField for Computed<T> {
    type Value = T;

    fn into_entry(self) -> FieldEntry<T> {
        FieldEntry {
            read: self,
            write: None,
        }
    }
}

impl<T> FieldEntry<T> {
    /// Drops the write side, so the page can read the field but not assign to it.
    #[must_use]
    pub fn readonly(mut self) -> Self {
        self.write = None;
        self
    }
}

/// Applies a JSON value the page wrote to the field behind it.
type ApplyWrite = Box<dyn Fn(serde_json::Value) -> Result<(), serde_json::Error>>;

/// One mirrored value, erased so fields of different types share a registry.
struct Field {
    /// Reads the current value as JSON.
    read: Box<dyn Fn() -> serde_json::Value>,
    /// Applies a value the page wrote. `None` for read-only fields.
    write: Option<ApplyWrite>,
    /// Bumped on every authoritative change, so the page can tell whether its
    /// optimistic value has been superseded.
    epoch: Cell<u64>,
    /// Set while an inbound write is being applied, so the notification it causes
    /// does not bounce straight back to the page.
    applying_write: Cell<bool>,
    /// Keeps the push subscription alive.
    _guard: BoxWatcherGuard,
}

/// What a push sends for one key.
#[derive(Serialize)]
struct FieldUpdate {
    v: serde_json::Value,
    e: u64,
}

/// The state a web view mirrors into its page.
#[derive(Default)]
pub struct StateRegistry {
    fields: RefCell<BTreeMap<Str, Rc<Field>>>,
    /// Keys changed since the last flush. Coalesced so a burst of writes costs
    /// one boundary crossing, not one per write.
    pending: RefCell<BTreeSet<Str>>,
}

/// What the page sends when it assigns to a field.
/// `Str` is not `Deserialize`, so the key arrives as a `String` and is converted
/// once here.
#[derive(serde::Deserialize)]
pub struct StateWrite {
    key: String,
    value: serde_json::Value,
    epoch: u64,
}

impl StateRegistry {
    /// Registers `field` under `name`, pushing on every later change.
    ///
    /// `on_change` is called when a key needs flushing; the caller decides when
    /// to drain, which is what keeps a burst to one crossing.
    pub(crate) fn insert<F: JsField>(
        self: &Rc<Self>,
        name: impl Into<Str>,
        field: F,
        on_change: impl Fn() + 'static,
    ) {
        let name = name.into();
        let FieldEntry { read, write } = field.into_entry();

        let guard = {
            let registry = Rc::downgrade(self);
            let name = name.clone();
            read.watch(move |_| {
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                let is_echo = registry
                    .fields
                    .borrow()
                    .get(&name)
                    .is_some_and(|field| field.applying_write.get());
                if is_echo {
                    return;
                }
                registry.pending.borrow_mut().insert(name.clone());
                on_change();
            })
        };

        let reader = read.clone();
        let entry = Field {
            // One closure serves both ways out — the seed script and every later
            // patch read through it — so tagging here covers both.
            read: Box::new(move || {
                let mut value =
                    serde_json::to_value(reader.get()).expect("exposed state must serialize");
                crate::big_integers::tag_unrepresentable(&mut value);
                value
            }),
            write: write.map(|write| {
                Box::new(move |mut value: serde_json::Value| {
                    crate::big_integers::untag(&mut value);
                    serde_json::from_value(value).map(|value| write(value))
                }) as ApplyWrite
            }),
            epoch: Cell::new(0),
            applying_write: Cell::new(false),
            _guard: guard,
        };
        self.fields
            .borrow_mut()
            .insert(name.clone(), Rc::new(entry));
        self.pending.borrow_mut().insert(name);
    }

    /// Renders the declarations that seed a freshly loaded document.
    pub(crate) fn seed_script(&self) -> String {
        let declarations: Vec<_> = self
            .fields
            .borrow()
            .iter()
            .map(|(name, field)| {
                (
                    name.as_str().to_owned(),
                    (field.read)(),
                    field.epoch.get(),
                    field.write.is_some(),
                )
            })
            .collect();
        let declarations =
            serde_json::to_string(&declarations).expect("state declarations must serialize");
        format!(
            "for (const [k, v, e, w] of {declarations}) globalThis.__wateruiState.define(k, v, e, w);"
        )
    }

    /// Drains the pending keys into one patch, or `None` when nothing changed.
    pub(crate) fn take_patch(&self) -> Option<String> {
        let keys: Vec<Str> = self.pending.borrow_mut().iter().cloned().collect();
        self.pending.borrow_mut().clear();
        if keys.is_empty() {
            return None;
        }
        let fields = self.fields.borrow();
        let patch: BTreeMap<&str, FieldUpdate> = keys
            .iter()
            .filter_map(|key| {
                let field = fields.get(key)?;
                field.epoch.set(field.epoch.get().wrapping_add(1));
                Some((
                    key.as_str(),
                    FieldUpdate {
                        v: (field.read)(),
                        e: field.epoch.get(),
                    },
                ))
            })
            .collect();
        Some(serde_json::to_string(&patch).expect("a state patch must serialize"))
    }

    /// Applies a write the page made.
    ///
    /// Returns whether the page's optimistic value still stands. When it does
    /// not — a validator clamped it, or a push crossed the write in flight — the
    /// key is queued so the authoritative value is sent back.
    pub(crate) fn apply_write(&self, write: &StateWrite) -> Result<bool, StateWriteError> {
        let key = Str::from(write.key.clone());
        let field = self
            .fields
            .borrow()
            .get(&key)
            .cloned()
            .ok_or_else(|| StateWriteError::Unknown(key.clone()))?;
        let Some(apply) = field.write.as_ref() else {
            return Err(StateWriteError::ReadOnly(key));
        };

        field.applying_write.set(true);
        let result = apply(write.value.clone());
        field.applying_write.set(false);
        result.map_err(|source| StateWriteError::Decode {
            key: key.clone(),
            source,
        })?;

        // Converge: if the value that landed differs from what the page assumed,
        // or a push overtook the write, correct the page.
        let settled = (field.read)();
        let stale_epoch = write.epoch != field.epoch.get();
        if settled == write.value && !stale_epoch {
            return Ok(true);
        }
        self.pending.borrow_mut().insert(key);
        Ok(false)
    }
}

/// A field recorded before the web view exists, ready to be registered once it
/// does.
///
/// `JsField` has an associated type and consumes `self`, so it is not object
/// safe; this closure is the erasure, in the same spirit as the private
/// `XxxImpl` shims elsewhere in the codebase.
pub type PendingField = Box<dyn FnOnce(&Rc<StateRegistry>, Rc<dyn Fn()>)>;

/// Erases one field registration.
pub fn pending_field<F: JsField + 'static>(name: Str, field: F) -> PendingField {
    Box::new(move |registry, on_change| {
        registry.insert(name, field, move || on_change());
    })
}

/// Attaches the reactive state bridge to a live web view.
///
/// Registers every field, seeds the current document, installs the reserved
/// handler the page writes through, and arranges for changes to be pushed.
///
/// Pushes are coalesced onto the local executor's next tick: sixty writes in one
/// tick cost one crossing, not sixty. That is a batching decision, not a
/// coarsening of the reactivity — the patch still carries only the keys that
/// changed.
pub fn install(webview: &crate::WebView, fields: Vec<PendingField>) {
    let registry = Rc::new(StateRegistry::default());
    let scheduled = Rc::new(Cell::new(false));

    let flush = {
        let registry = Rc::clone(&registry);
        let webview = webview.clone();
        let scheduled = Rc::clone(&scheduled);
        Rc::new(move || {
            if scheduled.replace(true) {
                return;
            }
            let registry = Rc::clone(&registry);
            let webview = webview.clone();
            let scheduled = Rc::clone(&scheduled);
            executor_core::spawn_local(async move {
                scheduled.set(false);
                let Some(patch) = registry.take_patch() else {
                    return;
                };
                let script =
                    crate::JsProgram::raw(format!("globalThis.__wateruiState.apply({patch});"));
                if let Err(error) = webview.exec(&script).await {
                    tracing::warn!(%error, "failed to push WaterUI state into the page");
                }
            })
            .detach();
        })
    };

    for field in fields {
        field(&registry, Rc::clone(&flush) as Rc<dyn Fn()>);
    }

    // The page must see correct values on its first line, so the declarations go
    // in as a document-start script rather than being pushed after load.
    webview.inject_script(
        &registry.seed_script(),
        crate::ScriptInjectionTime::DocumentStart,
    );

    webview.handle().add_handler(
        SET_STATE_HANDLER,
        Box::new({
            let registry = Rc::clone(&registry);
            let flush = Rc::clone(&flush);
            move |payload: &[u8]| {
                match serde_json::from_slice::<StateWrite>(payload)
                    .map_err(|source| StateWriteError::Decode {
                        key: Str::from_static("<unparsed>"),
                        source,
                    })
                    .and_then(|write| registry.apply_write(&write))
                {
                    Ok(true) => {}
                    Ok(false) => flush(),
                    Err(error) => {
                        tracing::warn!(%error, "page wrote a WaterUI state value that was refused");
                    }
                }
                // The page assigned to a state key; there is no value to answer with.
                Box::pin(core::future::ready(Ok(JsReply::Json(b"null".to_vec()))))
            }
        }),
    );

    // Seed the initial values.
    flush();
}

/// Why a write from the page could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum StateWriteError {
    /// The page assigned to a key that is not exposed.
    #[error("unknown WaterUI state key `{0}`")]
    Unknown(Str),
    /// The page assigned to a derived value.
    #[error("WaterUI state key `{0}` is read-only")]
    ReadOnly(Str),
    /// The page's value is not the field's type.
    #[error("WaterUI state key `{key}` rejected the value: {source}")]
    Decode {
        /// The key that was written.
        key: Str,
        /// The underlying serde error.
        #[source]
        source: serde_json::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::{JsField, StateRegistry, StateWrite};
    use std::rc::Rc;
    use waterui_core::{Binding, Computed, SignalExt};

    fn registry() -> Rc<StateRegistry> {
        Rc::new(StateRegistry::default())
    }

    #[test]
    fn a_binding_is_writable_and_a_computed_is_not() {
        let binding = Binding::container(1_u32);
        assert!(JsField::into_entry(binding.clone()).write.is_some());

        let derived: Computed<u32> = binding.map(|value| value * 2).computed();
        assert!(JsField::into_entry(derived).write.is_none());
    }

    #[test]
    fn registration_seeds_the_page_and_queues_a_first_push() {
        let registry = registry();
        registry.insert("count", Binding::container(7_u32), || {});

        // `SignalExt` is in scope for the `map` below and nami implements `Signal`
        // for plain values, so `str::contains` is named explicitly here.
        let seed = registry.seed_script();
        assert!(str::contains(&seed, "count"));
        assert!(registry.take_patch().is_some());
        // Drained.
        assert!(registry.take_patch().is_none());
    }

    #[test]
    fn a_burst_of_changes_coalesces_into_one_patch() {
        let registry = registry();
        let count = Binding::container(0_u32);
        registry.insert("count", count.clone(), || {});
        let _ = registry.take_patch();

        count.set(1);
        count.set(2);
        count.set(3);

        let patch = registry.take_patch().expect("one patch");
        assert_eq!(str::matches(&patch, "\"count\"").count(), 1);
        assert!(str::contains(&patch, "\"v\":3"));
    }

    /// `Binding::set` always notifies, so without suppression an inbound write
    /// would push straight back and oscillate.
    #[test]
    fn applying_a_write_does_not_echo_back_to_the_page() {
        let registry = registry();
        let theme = Binding::container(String::from("light"));
        registry.insert("theme", theme.clone(), || {});
        let _ = registry.take_patch();

        let accepted = registry
            .apply_write(&StateWrite {
                key: "theme".to_owned(),
                value: serde_json::json!("dark"),
                epoch: 1,
            })
            .expect("applies");

        assert!(accepted, "the page's value stands");
        assert_eq!(theme.get(), "dark");
        assert!(
            registry.take_patch().is_none(),
            "an accepted write must not be echoed"
        );
    }

    #[test]
    fn a_rejected_value_leaves_the_binding_alone_and_reports_why() {
        let registry = registry();
        registry.insert("count", Binding::container(1_u32), || {});

        let error = registry
            .apply_write(&StateWrite {
                key: "count".to_owned(),
                value: serde_json::json!("not a number"),
                epoch: 1,
            })
            .expect_err("rejects");
        assert!(matches!(error, super::StateWriteError::Decode { .. }));
    }

    #[test]
    fn writing_a_derived_value_is_refused() {
        let registry = registry();
        let count = Binding::container(1_u32);
        let derived: Computed<u32> = count.map(|value| value * 2).computed();
        registry.insert("doubled", derived, || {});

        let error = registry
            .apply_write(&StateWrite {
                key: "doubled".to_owned(),
                value: serde_json::json!(4),
                epoch: 1,
            })
            .expect_err("refuses");
        assert!(matches!(error, super::StateWriteError::ReadOnly(_)));
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let registry = registry();
        let error = registry
            .apply_write(&StateWrite {
                key: "nope".to_owned(),
                value: serde_json::json!(1),
                epoch: 0,
            })
            .expect_err("refuses");
        assert!(matches!(error, super::StateWriteError::Unknown(_)));
    }
}

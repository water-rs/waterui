//! Reactive state shared with the page.
//!
//! A value exposed here is mirrored into JavaScript, so the page reads it as a
//! local property instead of asking across the boundary, and writes flow back
//! into the `Binding` it came from. Direction is decided by the type: a
//! [`Binding`] is read-write, anything else is read-only.
//!
//! # The host never sends state unasked
//!
//! State reaches a page only as the reply to a request that page made. The
//! seed a document runs at its start carries the values and a cursor, and the
//! document then keeps one `__wateruiPullState { since }` request open. The
//! reply is held until a flush has changed something after `since`, carries
//! only those keys, and names the cursor to pull from next.
//!
//! Pushing instead, by evaluating each change into whatever document the web
//! view shows, cannot be made safe: no engine ties such an evaluation to a
//! document, so a page outside the admission policy that defined the bridge's
//! globals itself would receive the application's state. A reply, unlike
//! an evaluation, travels on the engine's channel back to the document that
//! sent the request, and is dropped when that document is gone.
//!
//! Two more things this has to get right, both learned from the shape of
//! `nami`:
//!
//! * `Binding::set` notifies unconditionally — `distinct()` is opt-in — so an
//!   echo would oscillate forever. Applying an inbound write suppresses the
//!   change it causes.
//! * A page that writes while a change is in flight must still converge. Each
//!   key carries the cursor it last changed at as its epoch; a write reports
//!   the epoch it was based on, and if the authoritative value ends up
//!   different the key is marked changed again, so the page is corrected.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use futures::channel::oneshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use suiteki::Str;
use waterui_core::{Binding, Computed, Signal};

use waterui_core::reactive::watcher::BoxWatcherGuard;

use crate::OriginPolicy;
use crate::message::JsReply;

/// The reserved handler the page writes through.
pub const SET_STATE_HANDLER: &str = "__wateruiSetState";

/// The reserved handler the page pulls changed state through.
pub const PULL_STATE_HANDLER: &str = "__wateruiPullState";

/// The function the seed is an application of. Its first statement is the
/// origin guard, so the declarations it carries never run in a document the
/// admission policy refuses, even on an engine that injects document-start
/// scripts into every document.
const SEED_SCRIPT: &str = include_str!("js/seed.js");

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
    /// The signal the bridge reads from.
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
    /// The cursor this key last changed at. It is also the key's epoch: the
    /// page reports the one its optimistic write was based on, so a write that
    /// a later change overtook is recognised as stale.
    changed: Cell<u64>,
    /// Set while an inbound write is being applied, so the notification it causes
    /// does not bounce straight back to the page.
    applying_write: Cell<bool>,
    /// Keeps the change subscription alive.
    _guard: BoxWatcherGuard,
}

impl Field {
    fn update(&self) -> FieldUpdate {
        FieldUpdate {
            v: (self.read)(),
            e: self.changed.get(),
            w: self.write.is_some(),
        }
    }
}

/// What a reply carries for one key.
///
/// Carries writability as well as the value, so a reply can define a key the
/// receiving document never saw seeded rather than only updating one it did.
#[derive(Serialize)]
struct FieldUpdate {
    v: serde_json::Value,
    e: u64,
    w: bool,
}

/// The reply to one pull: the keys changed after the cursor the page sent, and
/// the cursor to pull from next.
#[derive(Serialize)]
struct PullReply<'a> {
    cursor: u64,
    changes: BTreeMap<&'a str, FieldUpdate>,
}

/// What the page sends to pull: the cursor of the state it holds, or `null`
/// for a document that holds nothing it can trust — one restored from the
/// back/forward cache, which ran no seed and whose state is as old as the
/// moment it was frozen.
///
/// `since` is required: a pull that omits it is malformed, not a request for
/// every key. Deserializing through `Option::deserialize` is what keeps serde
/// from reading an absent field as `None`.
#[derive(serde::Deserialize)]
struct PullRequest {
    #[serde(deserialize_with = "Option::deserialize")]
    since: Option<u64>,
}

/// How a pull is answered.
enum Pull {
    /// The reply, ready now.
    Ready(Vec<u8>),
    /// Nothing changed after the cursor yet; the reply arrives on this.
    Parked(oneshot::Receiver<Vec<u8>>),
}

/// The state a web view mirrors into its page.
#[derive(Default)]
pub struct StateRegistry {
    fields: RefCell<BTreeMap<Str, Rc<Field>>>,
    /// Keys changed since the last flush. Coalesced so a burst of writes costs
    /// one reply, not one per write.
    dirty: RefCell<BTreeSet<Str>>,
    /// The cursor: bumped once by every flush that changed something.
    cursor: Cell<u64>,
    /// Pulls waiting for a change. Each was parked at the current cursor,
    /// since only a pull from the current cursor has nothing to receive, and
    /// every flush that moves the cursor answers them all.
    parked: RefCell<Vec<oneshot::Sender<Vec<u8>>>>,
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
    /// Registers `field` under `name`.
    ///
    /// `on_change` is called when a key has changed; the caller decides when
    /// to flush, which is what keeps a burst to one reply.
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
                registry.dirty.borrow_mut().insert(name.clone());
                on_change();
            })
        };

        let reader = read.clone();
        let entry = Field {
            // One closure serves both ways out — the seed script and every
            // reply read through it — so tagging here covers both.
            read: Box::new(move || {
                let mut value =
                    serde_json::to_value(reader.snapshot()).expect("exposed state must serialize");
                crate::big_integers::tag_unrepresentable(&mut value);
                value
            }),
            write: write.map(|write| {
                Box::new(move |mut value: serde_json::Value| {
                    crate::big_integers::untag(&mut value);
                    serde_json::from_value(value).map(|value| write(value))
                }) as ApplyWrite
            }),
            changed: Cell::new(self.cursor.get()),
            applying_write: Cell::new(false),
            _guard: guard,
        };
        self.fields.borrow_mut().insert(name, Rc::new(entry));
    }

    /// Renders the script that seeds a freshly loaded document: the current
    /// value of every key, and the cursor those values are current at.
    ///
    /// The values and the cursor are read together, so a change that lands
    /// after this is rendered — but before the document it seeds starts — is
    /// still delivered: the document's first pull names this cursor.
    pub(crate) fn seed_script(&self, policy: &OriginPolicy) -> String {
        #[derive(Serialize)]
        struct Seed<'a> {
            /// The admission rules, as the tokens `OriginRule::as_token`
            /// gives them, for the guard that opens the script.
            rules: Vec<&'a str>,
            cursor: u64,
            fields: Vec<(&'a str, FieldUpdate)>,
        }

        let rules = policy.rules();
        let fields = self.fields.borrow();
        let seed = Seed {
            rules: rules.iter().map(crate::OriginRule::as_token).collect(),
            cursor: self.cursor.get(),
            fields: fields
                .iter()
                .map(|(name, field)| (name.as_str(), field.update()))
                .collect(),
        };
        let seed = serde_json::to_string(&seed).expect("a state seed must serialize");
        format!("{SEED_SCRIPT}({seed});")
    }

    /// Answers a pull from the page.
    ///
    /// `None` gets every key now. A cursor gets the keys changed after it —
    /// now when there are any, otherwise once a flush changes one.
    fn pull(&self, since: Option<u64>) -> Result<Pull, PullError> {
        let Some(since) = since else {
            return Ok(Pull::Ready(self.reply(self.cursor.get(), |_| true)));
        };
        if since > self.cursor.get() {
            return Err(PullError::FutureCursor {
                since,
                cursor: self.cursor.get(),
            });
        }
        if since < self.cursor.get() {
            return Ok(Pull::Ready(self.changes_since(since)));
        }
        let (reply, parked) = oneshot::channel();
        self.parked.borrow_mut().push(reply);
        Ok(Pull::Parked(parked))
    }

    /// The reply to a pull from `since`, with every key changed after it.
    fn changes_since(&self, since: u64) -> Vec<u8> {
        self.reply(self.cursor.get(), |field| field.changed.get() > since)
    }

    /// Renders a reply at `cursor` carrying the keys `include` selects.
    fn reply(&self, cursor: u64, include: impl Fn(&Field) -> bool) -> Vec<u8> {
        let fields = self.fields.borrow();
        let reply = PullReply {
            cursor,
            changes: fields
                .iter()
                .filter(|(_, field)| include(field))
                .map(|(name, field)| (name.as_str(), field.update()))
                .collect(),
        };
        serde_json::to_vec(&reply).expect("a state reply must serialize")
    }

    /// Advances the cursor past every key changed since the last flush and
    /// answers the pulls that were waiting for a change.
    ///
    /// Does nothing when nothing changed, so the parked pulls stay parked.
    fn flush(&self) {
        let dirty = core::mem::take(&mut *self.dirty.borrow_mut());
        if dirty.is_empty() {
            return;
        }
        let since = self.cursor.get();
        let cursor = since + 1;
        self.cursor.set(cursor);
        {
            let fields = self.fields.borrow();
            for key in &dirty {
                if let Some(field) = fields.get(key) {
                    field.changed.set(cursor);
                }
            }
        }
        // Every parked pull asked from the cursor this flush moved past, so
        // one reply answers them all.
        Self::answer_parked(&self.parked, &self.changes_since(since));
    }

    /// Answers every parked pull with no changes, at the cursor they asked
    /// from.
    ///
    /// Called when the web view starts a navigation. A pull belongs to the
    /// document that sent it, and that document may be about to go away; its
    /// reply channel should not be held open for a change that may never come.
    /// A document that stays — the navigation was within it, or never
    /// committed — pulls again from the same cursor and loses nothing.
    fn settle_parked(&self) {
        Self::answer_parked(&self.parked, &self.reply(self.cursor.get(), |_| false));
    }

    /// Sends `reply` to every parked pull, which leaves none parked.
    fn answer_parked(parked: &RefCell<Vec<oneshot::Sender<Vec<u8>>>>, reply: &[u8]) {
        let parked = core::mem::take(&mut *parked.borrow_mut());
        for pull in parked {
            // A pull whose page went away dropped its receiver; there is
            // nobody to tell.
            let _ = pull.send(reply.to_vec());
        }
    }

    /// Applies a write the page made.
    ///
    /// Returns whether the page's optimistic value still stands. When it does
    /// not — a validator clamped it, or a change crossed the write in flight —
    /// the key is marked changed so the authoritative value is sent back.
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
        // or a change overtook the write, correct the page.
        let settled = (field.read)();
        let stale_epoch = write.epoch != field.changed.get();
        if settled == write.value && !stale_epoch {
            return Ok(true);
        }
        self.dirty.borrow_mut().insert(key);
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
/// Registers every field, seeds the documents `policy` admits, and installs the
/// reserved handlers the page writes and pulls through.
///
/// Changes are coalesced onto the local executor's next tick: sixty writes in
/// one tick advance the cursor once and cost each waiting pull one reply, not
/// sixty. That is a batching decision, not a coarsening of the reactivity — a
/// reply still carries only the keys that changed.
pub fn install(webview: &crate::WebView, fields: Vec<PendingField>, policy: &OriginPolicy) {
    let registry = Rc::new(StateRegistry::default());
    let scheduled = Rc::new(Cell::new(false));

    // Holds the registry weakly: the change subscriptions live inside it, so a
    // strong reference from their callback would keep it alive forever.
    let flush: Rc<dyn Fn()> = {
        let registry = Rc::downgrade(&registry);
        let scheduled = Rc::clone(&scheduled);
        Rc::new(move || {
            if scheduled.replace(true) {
                return;
            }
            let registry = registry.clone();
            let scheduled = Rc::clone(&scheduled);
            executor_core::spawn_local(async move {
                scheduled.set(false);
                if let Some(registry) = registry.upgrade() {
                    registry.flush();
                }
            })
            .detach();
        })
    };

    for field in fields {
        field(&registry, Rc::clone(&flush));
    }

    // The page must see correct values on its first line, so the declarations go
    // in as a document-start script rather than arriving with the first reply.
    //
    // The seed is a snapshot, so it is re-rendered and replaced under the same
    // key before every navigation. A seed that is stale by the time its
    // document starts is still correct, because it carries the cursor its
    // values are current at and the document's first pull asks from there.
    inject_seed(webview.handle(), &registry, policy);
    webview
        .handle()
        .watch({
            let registry = Rc::clone(&registry);
            let handle = webview.handle().downgrade();
            let policy = policy.clone();
            move |event| {
                if !matches!(
                    event,
                    crate::BackendEvent::Event(crate::WebViewEvent::WillNavigate { .. })
                ) {
                    return;
                }
                registry.settle_parked();
                if let Some(handle) = handle.upgrade() {
                    inject_seed(&handle, &registry, &policy);
                }
            }
        })
        .forget();

    webview.handle().add_handler(
        SET_STATE_HANDLER,
        Box::new({
            let registry = Rc::clone(&registry);
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

    webview.handle().add_handler(
        PULL_STATE_HANDLER,
        Box::new(move |payload: &[u8]| -> crate::HandlerFuture {
            let pull = serde_json::from_slice::<PullRequest>(payload)
                .map_err(PullError::Decode)
                .and_then(|request| registry.pull(request.since));
            match pull {
                Ok(Pull::Ready(reply)) => Box::pin(core::future::ready(Ok(JsReply::Json(reply)))),
                Ok(Pull::Parked(reply)) => Box::pin(async move {
                    reply.await.map(JsReply::Json).map_err(|_| {
                        String::from("the WaterUI web view closed before its state changed")
                    })
                }),
                Err(error) => {
                    tracing::warn!(%error, "page pulled WaterUI state with a malformed request");
                    Box::pin(core::future::ready(Err(error.to_string())))
                }
            }
        }),
    );
}

/// The key the mirrored-state seed is injected under.
///
/// Fixed so that re-injecting replaces the previous seed instead of stacking
/// another, staler copy in front of it.
const SEED_SCRIPT_KEY: &str = "waterui:state-seed";

/// Renders the current values and installs them as the document-start seed,
/// replacing whatever seed was there before.
fn inject_seed(
    handle: &crate::AnyWebViewHandle,
    registry: &Rc<StateRegistry>,
    policy: &OriginPolicy,
) {
    handle.inject_script(
        SEED_SCRIPT_KEY,
        &registry.seed_script(policy),
        crate::ScriptInjectionTime::DocumentStart,
    );
}

/// Why a pull from the page could not be answered.
#[derive(Debug, thiserror::Error)]
enum PullError {
    /// The request was not `{ since }`.
    ///
    /// The parse error is part of the message, which is what the page is
    /// rejected with, rather than a chained source.
    #[error("malformed WaterUI state pull: {0}")]
    Decode(serde_json::Error),
    /// The cursor is one this web view never handed out.
    #[error("WaterUI state pull from cursor {since}, but the state is at {cursor}")]
    FutureCursor {
        /// The cursor the page sent.
        since: u64,
        /// The current cursor.
        cursor: u64,
    },
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
    use super::{JsField, Pull, StateRegistry, StateWrite};
    use crate::{BridgeOrigins, OriginPolicy, Url};
    use std::rc::Rc;
    use waterui_core::{Binding, Computed, Signal, SignalExt};

    fn registry() -> Rc<StateRegistry> {
        Rc::new(StateRegistry::default())
    }

    fn json(bytes: &[u8]) -> serde_json::Value {
        serde_json::from_slice(bytes).expect("a reply is JSON")
    }

    fn ready(pull: Pull) -> serde_json::Value {
        match pull {
            Pull::Ready(reply) => json(&reply),
            Pull::Parked(_) => panic!("expected a reply now, got a parked pull"),
        }
    }

    fn parked(pull: Pull) -> futures::channel::oneshot::Receiver<Vec<u8>> {
        match pull {
            Pull::Parked(reply) => reply,
            Pull::Ready(reply) => panic!("expected a parked pull, got {:?}", json(&reply)),
        }
    }

    /// What a parked pull was answered with, once something answered it.
    fn answered(mut reply: futures::channel::oneshot::Receiver<Vec<u8>>) -> serde_json::Value {
        json(
            &reply
                .try_recv()
                .expect("the pull is still connected")
                .expect("the pull was answered"),
        )
    }

    #[test]
    fn a_binding_is_writable_and_a_computed_is_not() {
        let binding = Binding::container(1_u32);
        assert!(JsField::into_entry(binding.clone()).write.is_some());

        let derived: Computed<u32> = binding.map(|value| value * 2).computed();
        assert!(JsField::into_entry(derived).write.is_none());
    }

    #[test]
    fn the_seed_opens_with_the_origin_guard_and_carries_the_cursor() {
        let registry = registry();
        registry.insert("count", Binding::container(7_u32), || {});
        let initial: Url = "https://app.waterui.dev/start".parse().expect("parses");
        let policy = OriginPolicy::new(BridgeOrigins::Initial, &initial);

        let seed = registry.seed_script(&policy);
        let guard = seed.find("if (").expect("the guard");
        let declaration = seed.find("__wateruiState.define").expect("a declaration");
        assert!(guard < declaration, "the guard runs before any declaration");
        let data = seed
            .strip_prefix(super::SEED_SCRIPT)
            .and_then(|call| call.strip_prefix('('))
            .and_then(|call| call.strip_suffix(");"))
            .expect("the seed is the function applied to its data");
        assert_eq!(
            json(data.as_bytes()),
            serde_json::json!({
                "rules": ["https://app.waterui.dev"],
                "cursor": 0,
                "fields": [["count", {"v": 7, "e": 0, "w": true}]],
            })
        );
    }

    #[test]
    fn a_null_cursor_gets_every_key_now() {
        let registry = registry();
        registry.insert("count", Binding::container(7_u32), || {});
        registry.insert("theme", Binding::container(String::from("dark")), || {});

        assert_eq!(
            ready(registry.pull(None).expect("answers")),
            serde_json::json!({
                "cursor": 0,
                "changes": {
                    "count": {"v": 7, "e": 0, "w": true},
                    "theme": {"v": "dark", "e": 0, "w": true},
                },
            })
        );
    }

    #[test]
    fn a_pull_waits_for_a_flush_and_then_gets_only_what_changed() {
        let registry = registry();
        let count = Binding::container(0_u32);
        registry.insert("count", count.clone(), || {});
        registry.insert("theme", Binding::container(String::from("dark")), || {});

        let reply = parked(registry.pull(Some(0)).expect("answers"));
        count.set(1);
        count.set(2);
        count.set(3);
        registry.flush();

        assert_eq!(
            answered(reply),
            serde_json::json!({
                "cursor": 1,
                "changes": {"count": {"v": 3, "e": 1, "w": true}},
            }),
            "a burst coalesces into one reply carrying only the changed key"
        );
        // A document behind the cursor is answered at once.
        assert_eq!(
            ready(registry.pull(Some(0)).expect("answers"))["cursor"],
            serde_json::json!(1)
        );
    }

    #[test]
    fn a_navigation_settles_parked_pulls_with_nothing_at_their_own_cursor() {
        let registry = registry();
        let count = Binding::container(0_u32);
        registry.insert("count", count.clone(), || {});

        let reply = parked(registry.pull(Some(0)).expect("answers"));
        registry.settle_parked();
        assert_eq!(
            answered(reply),
            serde_json::json!({"cursor": 0, "changes": {}})
        );

        // The document that stayed pulls again and still gets the change.
        let reply = parked(registry.pull(Some(0)).expect("answers"));
        count.set(5);
        registry.flush();
        assert_eq!(answered(reply)["changes"]["count"]["v"], 5);
    }

    #[test]
    fn a_cursor_never_handed_out_is_refused() {
        let registry = registry();
        registry.insert("count", Binding::container(0_u32), || {});
        assert!(matches!(
            registry.pull(Some(9)),
            Err(super::PullError::FutureCursor {
                since: 9,
                cursor: 0
            })
        ));
    }

    #[test]
    fn a_pull_must_name_its_cursor_even_when_it_is_null() {
        let request = serde_json::from_slice::<super::PullRequest>(br#"{"since":null}"#)
            .expect("a null cursor is a pull for every key");
        assert_eq!(request.since, None);
        assert!(serde_json::from_slice::<super::PullRequest>(b"{}").is_err());
    }

    #[test]
    fn a_flush_with_nothing_changed_leaves_pulls_parked() {
        let registry = registry();
        registry.insert("count", Binding::container(0_u32), || {});
        let mut reply = parked(registry.pull(Some(0)).expect("answers"));
        registry.flush();
        assert!(matches!(reply.try_recv(), Ok(None)), "still waiting");
    }

    /// `Binding::set` always notifies, so without suppression an inbound write
    /// would be sent straight back and oscillate.
    #[test]
    fn applying_a_write_does_not_echo_back_to_the_page() {
        let registry = registry();
        let theme = Binding::container(String::from("light"));
        registry.insert("theme", theme.clone(), || {});

        let accepted = registry
            .apply_write(&StateWrite {
                key: "theme".to_owned(),
                value: serde_json::json!("dark"),
                epoch: 0,
            })
            .expect("applies");

        assert!(accepted, "the page's value stands");
        assert_eq!(theme.snapshot(), "dark");
        assert!(
            registry.dirty.borrow().is_empty(),
            "an accepted write must not be echoed"
        );
    }

    #[test]
    fn a_write_overtaken_by_a_change_is_corrected() {
        let registry = registry();
        let theme = Binding::container(String::from("light"));
        registry.insert("theme", theme.clone(), || {});
        theme.set(String::from("sepia"));
        registry.flush();

        let accepted = registry
            .apply_write(&StateWrite {
                key: "theme".to_owned(),
                value: serde_json::json!("dark"),
                epoch: 0,
            })
            .expect("applies");
        assert!(
            !accepted,
            "the write was based on an epoch a change replaced"
        );
        let reply = parked(registry.pull(Some(1)).expect("answers"));
        registry.flush();
        assert_eq!(answered(reply)["changes"]["theme"]["v"], "dark");
    }

    #[test]
    fn a_rejected_value_leaves_the_binding_alone_and_reports_why() {
        let registry = registry();
        registry.insert("count", Binding::container(1_u32), || {});

        let error = registry
            .apply_write(&StateWrite {
                key: "count".to_owned(),
                value: serde_json::json!("not a number"),
                epoch: 0,
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
                epoch: 0,
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

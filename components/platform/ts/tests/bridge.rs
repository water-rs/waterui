//! The reactive seam, end to end on the engine this target selects.
//!
//! Every test loads `fixtures/runtime.js`, a classic script that publishes the
//! runtime global a real bundle publishes, over a tiny push-based signal
//! implementation. What is under test is the Rust side: what materialization
//! creates, which way values travel, how many times one change propagates, and
//! what is disposed when a value is dropped.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use nami::{Binding, Computed, Signal};
use waterui_core::{AnyView, Environment, Error};
use waterui_graphics::ResolvedColor;
use waterui_graphics::color::{AccentColor, ColorScheme};
use waterui_ts::engine::{JsError, JsFunction, JsRuntime, JsValue};
use waterui_ts::schema::{TsProps, TsType, TypeSchema};
use waterui_ts::{Bridge, FromJs, HostTable, IntoJs, TsRuntime};

/// The runtime global a real bundle publishes, faked for a Rust test.
const FIXTURE: &str = include_str!("fixtures/runtime.js");

/// A bundle whose runtime is missing an entry.
const INCOMPLETE: &str = include_str!("fixtures/incomplete_runtime.js");

// ---------------------------------------------------------------------------
// A host table that records what it was asked for.
// ---------------------------------------------------------------------------

/// What one host call was.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Create(String),
    Modify(String),
    Text,
    Show,
    Each,
    Suspense,
}

/// A host table that records its calls and hands back empty views.
///
/// The component bindings are the host-table leaf's (water-rs/waterui#1042);
/// what this stands in for is everything the bridge needs from a table: the
/// modifier names, and a view to wrap in a slot.
#[derive(Clone, Default)]
struct TestHost {
    calls: Rc<RefCell<Vec<Call>>>,
}

impl HostTable for TestHost {
    fn modifiers(&self) -> Vec<suiteki::Str> {
        vec![
            suiteki::Str::from("padding"),
            suiteki::Str::from("background"),
        ]
    }

    fn create(
        &self,
        _bridge: &Bridge,
        component: &str,
        _config: &[(String, JsValue)],
        _children: &[JsValue],
    ) -> Result<AnyView, Error> {
        self.calls
            .borrow_mut()
            .push(Call::Create(component.to_owned()));
        Ok(AnyView::new(()))
    }

    fn modify(
        &self,
        _bridge: &Bridge,
        view: AnyView,
        name: &str,
        _value: &JsValue,
    ) -> Result<AnyView, Error> {
        self.calls.borrow_mut().push(Call::Modify(name.to_owned()));
        Ok(view)
    }

    fn text(&self, _bridge: &Bridge, _content: &JsValue) -> Result<AnyView, Error> {
        self.calls.borrow_mut().push(Call::Text);
        Ok(AnyView::new(()))
    }

    fn show(
        &self,
        _bridge: &Bridge,
        _when: &JsValue,
        _render: &JsFunction,
        _fallback: Option<&JsFunction>,
    ) -> Result<AnyView, Error> {
        self.calls.borrow_mut().push(Call::Show);
        Ok(AnyView::new(()))
    }

    fn each(
        &self,
        _bridge: &Bridge,
        _items: &JsValue,
        _render: &JsFunction,
        _by: Option<&JsFunction>,
    ) -> Result<AnyView, Error> {
        self.calls.borrow_mut().push(Call::Each);
        Ok(AnyView::new(()))
    }

    fn suspense(
        &self,
        _bridge: &Bridge,
        _children: &JsFunction,
        _fallback: Option<&JsFunction>,
    ) -> Result<AnyView, Error> {
        self.calls.borrow_mut().push(Call::Suspense);
        Ok(AnyView::new(()))
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A runtime with the fixture bundle loaded and the host installed.
fn runtime() -> TsRuntime {
    with_environment(Environment::new())
}

/// The same, over an environment the test prepared.
fn with_environment(environment: Environment) -> TsRuntime {
    let runtime = TsRuntime::new(environment, TestHost::default()).expect("the runtime constructs");
    runtime.load(FIXTURE).expect("the fixture bundle loads");
    runtime
}

/// The same, with the host table the test can inspect afterwards.
fn with_host(host: TestHost) -> TsRuntime {
    let runtime = TsRuntime::new(Environment::new(), host).expect("the runtime constructs");
    runtime.load(FIXTURE).expect("the fixture bundle loads");
    runtime
}

/// Evaluates an expression, which must not throw.
fn eval(runtime: &TsRuntime, source: &str) -> JsValue {
    runtime
        .bridge()
        .engine()
        .eval(source, "test.js")
        .unwrap_or_else(|error| panic!("evaluating `{source}`: {error}"))
}

/// Evaluates an expression that is expected to throw.
fn eval_error(runtime: &TsRuntime, source: &str) -> JsError {
    runtime
        .bridge()
        .engine()
        .eval(source, "test.js")
        .expect_err(source)
}

/// Evaluates an expression that yields a whole number.
///
/// Every count and every value these tests compare is an integer, and reading
/// it as one keeps the assertions exact instead of comparing doubles.
fn integer(runtime: &TsRuntime, source: &str) -> i64 {
    eval(runtime, source)
        .as_i64()
        .unwrap_or_else(|| panic!("`{source}` is not a whole number"))
}

/// Evaluates an expression that yields a function.
fn function(runtime: &TsRuntime, source: &str) -> JsFunction {
    match eval(runtime, source) {
        JsValue::Function(function) => function,
        other => panic!("`{source}` is {other:?}, not a function"),
    }
}

/// Counts how many times a signal notified its Rust watchers.
fn watch_count<S: Signal>(signal: &S) -> (Rc<Cell<usize>>, S::Guard) {
    let count = Rc::new(Cell::new(0));
    let guard = signal.watch({
        let count = Rc::clone(&count);
        move |_| count.set(count.get() + 1)
    });
    (count, guard)
}

// ---------------------------------------------------------------------------
// Materialization
// ---------------------------------------------------------------------------

#[test]
fn a_javascript_signal_becomes_a_two_way_binding() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    let source = eval(&runtime, "globalThis.fixture.counter");

    let binding: Binding<u32> = bridge
        .materialize_binding(&source)
        .expect("a signal materializes");
    assert_eq!(binding.get(), 1, "the binding is seeded from the signal");
    let (notifications, _guard) = watch_count(&binding);

    // JavaScript to Rust: one notification, and the value the binding came to
    // hold is confirmed back. The confirmation is a `write` crossing and
    // nothing more — the value is already there, so no `set` is made and no
    // JavaScript subscriber hears anything.
    eval(&runtime, "globalThis.fixture.counter.set(5)");
    assert_eq!(binding.get(), 5);
    assert_eq!(notifications.get(), 1, "one change, one notification");
    assert_eq!(
        integer(&runtime, "globalThis.fixture.writes"),
        1,
        "the inbound value is confirmed, not written back a second time"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.sets"),
        1,
        "the confirmation stored nothing: the value was already there"
    );

    // Rust to JavaScript: one write, and the echo it settles is dropped rather
    // than applied a second time.
    binding.set(9);
    assert_eq!(integer(&runtime, "globalThis.fixture.counter()"), 9);
    assert_eq!(integer(&runtime, "globalThis.fixture.writes"), 2);
    assert_eq!(
        notifications.get(),
        2,
        "the echo did not set the binding again"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.sets"),
        2,
        "the signal was set once from each side, and never oscillated"
    );
}

#[test]
fn an_effect_that_bounces_a_write_leaves_both_sides_agreeing() {
    let runtime = runtime();
    let source = eval(&runtime, "globalThis.fixture.counter");

    let binding: Binding<u32> = runtime
        .bridge()
        .materialize_binding(&source)
        .expect("a signal materializes");

    // An effect that rewrites the value twice while one write settles: 5
    // becomes 9, and 9 becomes 5 again. Every one of those notifications
    // reaches the cell's subscription, and none of them is where the value
    // came to rest.
    eval(
        &runtime,
        "globalThis.bounced = 0;
         globalThis.fixture.counter.__subscribe((value) => {
           if (globalThis.bounced >= 2) { return; }
           globalThis.bounced += 1;
           globalThis.fixture.counter.set(value === 5 ? 9 : 5);
         })",
    );

    binding.set(5);

    assert_eq!(
        integer(&runtime, "globalThis.bounced"),
        2,
        "the effect really did rewrite the value while the write settled"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.counter()"),
        5,
        "JavaScript came to rest at 5"
    );
    assert_eq!(
        binding.get(),
        5,
        "and so did Rust: an intermediate value the effect passed through is \
         not where the write ended"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.writes"),
        1,
        "one change, one write: the corrections are JavaScript's own"
    );
}

#[test]
fn an_effect_that_clamps_a_write_is_read_back_once() {
    let runtime = runtime();
    let source = eval(&runtime, "globalThis.fixture.counter");

    let binding: Binding<u32> = runtime
        .bridge()
        .materialize_binding(&source)
        .expect("a signal materializes");

    // A clamp: anything above 10 settles at 10.
    eval(
        &runtime,
        "globalThis.fixture.counter.__subscribe((value) => {
           if (value > 10) { globalThis.fixture.counter.set(10); }
         })",
    );
    let (notifications, _guard) = watch_count(&binding);

    binding.set(40);
    assert_eq!(integer(&runtime, "globalThis.fixture.counter()"), 10);
    assert_eq!(binding.get(), 10, "the correction came back to Rust");
    assert_eq!(
        notifications.get(),
        2,
        "the set and the one correction, and nothing after it"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.writes"),
        2,
        "the write, then the confirmation of the clamped value the binding \
         came to hold — and no third round, because that value stands"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.sets"),
        2,
        "the write and the clamp: the confirmation stored nothing"
    );
}

#[test]
fn a_rust_watcher_that_writes_during_an_inbound_apply_reaches_javascript() {
    let runtime = runtime();
    let source = eval(&runtime, "globalThis.fixture.counter");

    let binding: Binding<u32> = runtime
        .bridge()
        .materialize_binding(&source)
        .expect("a signal materializes");

    // JavaScript clamps anything above ten, and Rust answers a clamped ten
    // with a write of its own. Both corrections happen while one change is
    // still being delivered.
    eval(
        &runtime,
        "globalThis.fixture.counter.__subscribe((value) => {
           if (value > 10) { globalThis.fixture.counter.set(10); }
         })",
    );
    let _guard = binding.watch({
        let binding = binding.clone();
        move |context: nami::watcher::Context<u32>| {
            if context.into_value() == 10 {
                binding.set(0);
            }
        }
    });

    binding.set(40);

    assert_eq!(
        binding.get(),
        0,
        "the Rust answer to the clamp is the state"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.counter()"),
        0,
        "and JavaScript was told about it, instead of being left at the clamp"
    );
}

#[test]
fn a_rust_watcher_that_answers_an_inbound_change_reaches_javascript() {
    let runtime = runtime();
    let source = eval(
        &runtime,
        "globalThis.fixture.text = globalThis.__waterui_runtime.createSignal('start')",
    );

    let binding: Binding<String> = runtime
        .bridge()
        .materialize_binding(&source)
        .expect("a signal materializes");
    let _guard = binding.watch({
        let binding = binding.clone();
        move |context: nami::watcher::Context<String>| {
            if context.into_value() == "x" {
                binding.set(String::from("y"));
            }
        }
    });

    eval(&runtime, "globalThis.fixture.text.set('x')");

    assert_eq!(binding.get(), "y");
    assert_eq!(
        eval(&runtime, "globalThis.fixture.text()"),
        JsValue::String(String::from("y")),
        "a Rust write made while an inbound value was being applied is not lost"
    );
}

#[test]
fn a_host_accessor_becomes_a_pushed_computed() {
    let runtime = runtime();
    let source = eval(&runtime, "globalThis.fixture.label");

    let label: Computed<String> = runtime
        .bridge()
        .materialize_computed(&source)
        .expect("an accessor materializes");
    assert_eq!(label.get(), "start");

    eval(&runtime, "globalThis.fixture.pushLabel('next')");
    assert_eq!(label.get(), "next", "the push reached the Rust value");
    assert_eq!(
        integer(&runtime, "globalThis.fixture.writes"),
        0,
        "a read-only value is never written"
    );
}

#[test]
fn a_function_accessor_becomes_a_pushed_computed() {
    let runtime = runtime();
    let source = eval(&runtime, "globalThis.fixture.doubled");

    let doubled: Computed<u32> = runtime
        .bridge()
        .materialize_computed(&source)
        .expect("an accessor materializes");
    assert_eq!(doubled.get(), 2);

    eval(&runtime, "globalThis.fixture.counter.set(6)");
    assert_eq!(doubled.get(), 12);
}

#[test]
fn a_constant_becomes_a_constant_computed_and_subscribes_to_nothing() {
    let runtime = runtime();
    let before = integer(&runtime, "globalThis.fixture.subscribes");

    let plain: Computed<u32> = runtime
        .bridge()
        .materialize_computed(&JsValue::Number(7.0))
        .expect("a constant materializes");
    assert_eq!(plain.get(), 7);

    // Readable, but it announces nothing: one read is all it will ever say.
    let source = eval(&runtime, "globalThis.fixture.readOnce");
    let once: Computed<u32> = runtime
        .bridge()
        .materialize_computed(&source)
        .expect("a read-only value materializes");
    assert_eq!(once.get(), 41);

    assert_eq!(
        integer(&runtime, "globalThis.fixture.subscribes"),
        before,
        "a constant creates no subscription"
    );
}

#[test]
fn a_two_way_value_refuses_a_read_only_input() {
    let runtime = runtime();
    let source = eval(&runtime, "globalThis.fixture.label");

    let error = runtime
        .bridge()
        .materialize_binding::<String>(&source)
        .expect_err("a read-only accessor cannot be a binding");
    assert!(
        error.message.contains("needs a signal"),
        "the error names what was missing: {error}"
    );
}

#[test]
fn dropping_a_materialized_value_disposes_its_subscription() {
    let runtime = runtime();
    let source = eval(&runtime, "globalThis.fixture.counter");
    let before = integer(&runtime, "globalThis.fixture.disposes");

    let binding: Binding<u32> = runtime
        .bridge()
        .materialize_binding(&source)
        .expect("a signal materializes");
    assert_eq!(
        integer(&runtime, "globalThis.fixture.disposes"),
        before,
        "the subscription is live while the binding is"
    );

    drop(binding);
    assert_eq!(
        integer(&runtime, "globalThis.fixture.disposes"),
        before + 1,
        "dropping the last reference disposed the JavaScript subscription"
    );

    // And the JavaScript side really stopped feeding it: a later change must
    // not reach a dropped cell.
    eval(&runtime, "globalThis.fixture.counter.set(3)");
}

/// A plain data payload: what an ordinary object looks like crossing the seam.
#[derive(TsType, Debug, Clone, PartialEq, Eq)]
struct Point {
    x: u32,
    y: u32,
}

#[test]
fn an_object_payload_confirmed_back_does_not_re_notify_javascript() {
    let runtime = runtime();
    eval(
        &runtime,
        "globalThis.fixture.point = globalThis.__waterui_runtime.createSignal({ x: 1, y: 2 });
         globalThis.fixture.pointNotifications = 0;
         globalThis.fixture.point.__subscribe(() => {
           globalThis.fixture.pointNotifications += 1;
         });",
    );
    let source = eval(&runtime, "globalThis.fixture.point");

    let binding: Binding<Point> = runtime
        .bridge()
        .materialize_binding(&source)
        .expect("a signal materializes");
    assert_eq!(binding.get(), Point { x: 1, y: 2 });

    eval(&runtime, "globalThis.fixture.point.set({ x: 3, y: 4 })");
    assert_eq!(
        binding.get(),
        Point { x: 3, y: 4 },
        "the change reached Rust"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.pointNotifications"),
        1,
        "the value the cell confirms back is the one already there, and \
         costs nothing: a payload crossing the seam is a fresh object every \
         time, so it is compared structurally rather than by reference"
    );
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

#[test]
fn a_rust_binding_exported_to_javascript_round_trips() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    let binding = Binding::container(3_u32);
    let exported = binding
        .clone()
        .into_js(bridge)
        .expect("a binding exports as a signal");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    assert_eq!(integer(&runtime, "globalThis.fixture.held()"), 3);

    binding.set(4);
    assert_eq!(
        integer(&runtime, "globalThis.fixture.held()"),
        4,
        "a Rust change reaches the exported signal"
    );

    eval(&runtime, "globalThis.fixture.held.set(8)");
    assert_eq!(binding.get(), 8, "a JavaScript write reaches the binding");
}

#[test]
fn a_javascript_write_a_filter_rejects_is_corrected_in_javascript() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    let source = Binding::container(5_u32);
    // Single digits only. A rejected write stores nothing and notifies
    // nobody, so no count of notifications can tell the cell what became of
    // the value: only reading the binding back can.
    let filtered = source.filter(|value| *value < 10);
    let exported = filtered
        .into_js(bridge)
        .expect("a binding exports as a signal");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    eval(&runtime, "globalThis.fixture.held.set(50)");
    assert_eq!(source.get(), 5, "the filter rejected the write");
    assert_eq!(
        integer(&runtime, "globalThis.fixture.held()"),
        5,
        "and JavaScript holds what Rust holds, instead of a value Rust never took"
    );
}

#[test]
fn a_javascript_write_a_setter_normalizes_comes_back_normalized() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    let source = Binding::container(1_u32);
    // The setter clamps, so the binding notifies exactly once — carrying a
    // value JavaScript never sent. Counting the notification would call that
    // one the write's own echo and drop it.
    let clamped = Binding::mapping(
        &source,
        |value: u32| value,
        |binding: &Binding<u32>, value: u32| binding.set(value.min(10)),
    );
    let exported = clamped
        .into_js(bridge)
        .expect("a binding exports as a signal");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    eval(&runtime, "globalThis.fixture.held.set(50)");
    assert_eq!(source.get(), 10, "the setter normalized the write");
    assert_eq!(
        integer(&runtime, "globalThis.fixture.held()"),
        10,
        "and JavaScript was corrected to the value that stands"
    );
}

#[test]
fn a_watcher_registered_before_the_export_cannot_push_a_stale_value() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    // Registered first, so nami calls it first: by the time the cell's own
    // watcher runs, the value its notification carries is already history.
    let binding = Binding::container(0_u32);
    let _guard = binding.watch({
        let binding = binding.clone();
        move |context: nami::watcher::Context<u32>| {
            if context.into_value() == 1 {
                binding.set(2);
            }
        }
    });

    let exported = binding.clone().into_js(bridge).expect("a binding exports");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    binding.set(1);

    assert_eq!(binding.get(), 2);
    assert_eq!(
        integer(&runtime, "globalThis.fixture.held()"),
        2,
        "the cell pushes what the binding holds, not what its notification carried"
    );
}

#[test]
fn a_value_that_is_a_function_is_stored_and_not_called() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    let binding = Binding::container(JsValue::Number(1.0));
    let exported = binding.clone().into_js(bridge).expect("a binding exports");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    // A callback, a memo, a signal: anything a props payload may carry is a
    // function on the JavaScript side, and a signal's `set` reads a function
    // argument as an updater.
    let probe = eval(&runtime, "globalThis.fixture.probe");
    binding.set(probe);

    assert_eq!(
        eval(
            &runtime,
            "globalThis.fixture.held() === globalThis.fixture.probe"
        ),
        JsValue::Bool(true),
        "the function itself is what JavaScript holds"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.probeCalls"),
        0,
        "and writing it did not call it"
    );
}

#[test]
fn a_float_keeps_the_value_rust_set_it_to() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    let binding = Binding::container(0.0_f64);
    let exported = binding.clone().into_js(bridge).expect("a binding exports");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");
    eval(
        &runtime,
        "globalThis.notified = 0;
         globalThis.fixture.held.__subscribe(() => { globalThis.notified += 1; })",
    );

    binding.set(f64::NAN);
    assert!(binding.get().is_nan(), "Rust kept the NaN it set");
    assert_eq!(
        eval(&runtime, "Number.isNaN(globalThis.fixture.held())"),
        JsValue::Bool(true)
    );
    assert_eq!(integer(&runtime, "globalThis.notified"), 1);

    binding.set(f64::NAN);
    assert_eq!(
        integer(&runtime, "globalThis.notified"),
        1,
        "NaN written over NaN is not a change on either side"
    );

    binding.set(0.0);
    binding.set(-0.0);
    assert!(
        binding.get().is_sign_negative(),
        "Rust kept the negative zero it set, instead of being corrected to +0"
    );
    assert_eq!(
        eval(&runtime, "Object.is(globalThis.fixture.held(), -0)"),
        JsValue::Bool(true),
        "and JavaScript holds the same value, by the same equality"
    );
}

#[test]
fn a_rust_computed_exported_to_javascript_is_read_only_and_pushed() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    let source = Binding::container(2_u32);
    let computed = Computed::new(source.clone());
    let exported = computed
        .into_js(bridge)
        .expect("a computed exports as an accessor");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    assert_eq!(integer(&runtime, "globalThis.fixture.held()"), 2);
    source.set(5);
    assert_eq!(integer(&runtime, "globalThis.fixture.held()"), 5);
    assert_eq!(
        eval(&runtime, "typeof globalThis.fixture.held.set"),
        JsValue::String(String::from("undefined")),
        "an exported computed is a memo: JavaScript cannot write to it"
    );
}

#[test]
fn a_rust_callback_is_called_from_javascript_with_converted_arguments() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    let seen: Rc<RefCell<Vec<(u32, String)>>> = Rc::new(RefCell::new(Vec::new()));
    let callback: Box<dyn Fn(u32, String)> = Box::new({
        let seen = Rc::clone(&seen);
        move |count, label| seen.borrow_mut().push((count, label))
    });
    let exported = callback.into_js(bridge).expect("a callback exports");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    eval(&runtime, "globalThis.fixture.held(7, 'hi')");
    assert_eq!(*seen.borrow(), vec![(7, String::from("hi"))]);

    let error = eval_error(&runtime, "globalThis.fixture.held('nope', 'hi')");
    assert!(
        error.message.contains("argument 0"),
        "the error names the argument that failed: {error}"
    );
    assert_eq!(
        seen.borrow().len(),
        1,
        "a call that could not be converted never reached the closure"
    );
}

#[test]
fn one_rust_signal_is_one_javascript_signal_however_often_it_crosses() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    let _scope = bridge.open_scope();
    let hold = function(&runtime, "globalThis.fixture.hold");

    // A value that carries a signal, pushed on every change: the field is one
    // Rust binding throughout, so it must be one JavaScript signal throughout.
    let unread = Binding::container(0_u32);
    let card = |title: &str| Card {
        title: String::from(title),
        status: Status::Idle,
        change: Change::Cleared,
        subtitle: None,
        tags: Vec::new(),
        counts: BTreeMap::new(),
        unread: unread.clone(),
        identifier: 1,
    };

    let outer = Binding::container(card("first"));
    let exported = outer
        .clone()
        .into_js(bridge)
        .expect("a binding of a value carrying a signal exports");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");

    let signals = integer(&runtime, "globalThis.fixture.signals");
    let subscribes = integer(&runtime, "globalThis.fixture.subscribes");
    for title in ["second", "third", "fourth"] {
        outer.set(card(title));
    }

    assert_eq!(
        integer(&runtime, "globalThis.fixture.writes"),
        3,
        "one change, one write"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.signals"),
        signals,
        "the binding crossing again is the signal it crossed as the first time"
    );
    assert_eq!(
        integer(&runtime, "globalThis.fixture.subscribes"),
        subscribes,
        "so no second cell watches the same state"
    );

    // And what JavaScript holds is that live signal, not a snapshot of it.
    let unread_now = "globalThis.__waterui_runtime.read(globalThis.fixture.held).unread()";
    assert_eq!(integer(&runtime, unread_now), 0);
    unread.set(7);
    assert_eq!(integer(&runtime, unread_now), 7);
}

#[test]
fn exporting_outside_a_mount_is_refused() {
    let runtime = runtime();
    let error = Binding::container(1_u32)
        .into_js(runtime.bridge())
        .expect_err("nothing would own the cell");
    assert!(
        error.message.contains("mount scope"),
        "the error says what is missing: {error}"
    );
}

#[test]
fn disposing_a_mount_releases_everything_it_exported() {
    let scheme = Binding::container(ColorScheme::Light);
    let environment = Environment::new()
        .store::<ColorScheme, Computed<ColorScheme>>(Computed::new(scheme.clone()));
    let runtime = with_environment(environment);
    let bridge = runtime.bridge();

    let mut cost = Vec::new();
    for _ in 0..2 {
        let before = integer(&runtime, "globalThis.fixture.signals");
        let scope = bridge.open_scope();
        eval(
            &runtime,
            "globalThis.fixture.held = globalThis.__waterui_host.environment()",
        );
        let callback: Box<dyn Fn(u32)> = Box::new(|_| ());
        let JsValue::Function(exported) = callback.into_js(bridge).expect("a callback exports")
        else {
            panic!("a callback crosses as a function");
        };
        cost.push(integer(&runtime, "globalThis.fixture.signals") - before);

        // While the mount is open, the theme it exported is live.
        let writes = integer(&runtime, "globalThis.fixture.writes");
        scheme.set(ColorScheme::Dark);
        assert!(
            integer(&runtime, "globalThis.fixture.writes") > writes,
            "a theme change reaches the mount"
        );

        drop(scope);

        let writes = integer(&runtime, "globalThis.fixture.writes");
        scheme.set(ColorScheme::Light);
        assert_eq!(
            integer(&runtime, "globalThis.fixture.writes"),
            writes,
            "and stops reaching it once the mount is disposed"
        );
        let error = bridge
            .call(&exported, &[JsValue::Number(1.0)])
            .expect_err("the registration went with the mount");
        assert!(
            error.message.contains("no Rust callback is registered"),
            "the wrapper outlived what owned it: {error}"
        );
    }

    assert_eq!(
        cost[0], cost[1],
        "a second mount costs exactly what the first did, with nothing carried over"
    );
}

#[test]
fn a_view_slot_is_taken_exactly_once() {
    let runtime = runtime();
    let bridge = runtime.bridge();

    let slot = AnyView::new(())
        .into_js(bridge)
        .expect("a view crosses as a slot");
    let _taken = AnyView::from_js(&slot, bridge).expect("the first take succeeds");
    let error = AnyView::from_js(&slot, bridge).expect_err("the second take fails");
    assert!(
        error.message.contains("already taken"),
        "the error says the view is gone: {error}"
    );
}

// ---------------------------------------------------------------------------
// The derived conversions
// ---------------------------------------------------------------------------

/// Every variant is a unit variant, so the schema projects a string union.
#[derive(TsType, Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Idle,
    Running,
}

/// A variant carries data, so the schema projects an adjacently tagged object.
#[derive(TsType, Debug, Clone, PartialEq, Eq)]
enum Change {
    Cleared,
    Renamed(String),
    Resized { width: u32, height: u32 },
}

/// One nested type carrying every shape the mapping has to get right.
#[derive(TsType, Clone)]
struct Card {
    title: String,
    status: Status,
    change: Change,
    subtitle: Option<String>,
    tags: Vec<String>,
    counts: BTreeMap<String, u32>,
    unread: Binding<u32>,
    identifier: u64,
}

/// A props struct: one way, and carrying what only travels outwards.
///
/// `on_dismiss` is why props are one way. A callback is registered for
/// JavaScript to call, never read back out of a JavaScript value, so a props
/// struct that had to be readable could not carry one.
#[derive(TsProps)]
struct PromoProps {
    card: Card,
    on_dismiss: Box<dyn Fn(u32) + Send + Sync>,
}

#[test]
fn a_derived_struct_projects_the_shape_the_schema_declares() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = bridge.open_scope();

    let card = Card {
        title: String::from("Promo"),
        status: Status::Running,
        change: Change::Resized {
            width: 4,
            height: 3,
        },
        subtitle: None,
        tags: vec![String::from("new")],
        counts: BTreeMap::from([(String::from("seen"), 2_u32)]),
        unread: Binding::container(1_u32),
        identifier: u64::MAX,
    };

    let value = card.into_js(bridge).expect("a derived struct crosses");
    let entries = value.as_object().expect("a struct is an object").to_vec();
    let field = |name: &str| {
        entries.iter().find(|(key, _)| key == name).map_or_else(
            || panic!("the object carries `{name}`"),
            |(_, value)| value.clone(),
        )
    };

    assert_eq!(field("title"), JsValue::String(String::from("Promo")));
    assert_eq!(
        field("status"),
        JsValue::String(String::from("Running")),
        "a unit-only enum is a string"
    );
    assert_eq!(
        field("change"),
        JsValue::Object(vec![
            (
                String::from("type"),
                JsValue::String(String::from("Resized"))
            ),
            (
                String::from("value"),
                JsValue::Object(vec![
                    (String::from("width"), JsValue::Number(4.0)),
                    (String::from("height"), JsValue::Number(3.0)),
                ])
            ),
        ]),
        "an enum carrying data is adjacently tagged"
    );
    assert_eq!(field("subtitle"), JsValue::Null, "`None` is `null`");
    assert_eq!(
        field("tags"),
        JsValue::Array(vec![JsValue::String(String::from("new"))])
    );
    assert_eq!(
        field("counts"),
        JsValue::Object(vec![(String::from("seen"), JsValue::Number(2.0))])
    );
    assert!(
        matches!(field("unread"), JsValue::Function(_)),
        "a `Binding` is a JavaScript signal"
    );
    assert_eq!(
        field("identifier"),
        JsValue::BigInt(waterui_ts::engine::BigInt::Unsigned(u64::MAX)),
        "a 64-bit integer crosses as a bigint, exactly as the schema types it"
    );
}

#[test]
fn a_derived_struct_round_trips() {
    let runtime = runtime();
    let bridge = runtime.bridge();
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = bridge.open_scope();

    let card = Card {
        title: String::from("Promo"),
        status: Status::Idle,
        change: Change::Renamed(String::from("after")),
        subtitle: Some(String::from("subtitle")),
        tags: vec![String::from("a"), String::from("b")],
        counts: BTreeMap::from([(String::from("seen"), 2_u32), (String::from("left"), 9_u32)]),
        unread: Binding::container(4_u32),
        identifier: 9_007_199_254_740_993,
    };

    let value = card.into_js(bridge).expect("a derived struct crosses");
    let read = Card::from_js(&value, bridge).expect("and reads back");

    assert_eq!(read.title, "Promo");
    assert_eq!(read.status, Status::Idle);
    assert_eq!(read.change, Change::Renamed(String::from("after")));
    assert_eq!(read.subtitle.as_deref(), Some("subtitle"));
    assert_eq!(read.tags, vec![String::from("a"), String::from("b")]);
    assert_eq!(read.counts.get("left"), Some(&9));
    assert_eq!(
        read.identifier, 9_007_199_254_740_993,
        "an integer past 2^53 survives the round trip bit for bit"
    );

    // The binding came back bound to the same signal, so the two are one
    // value: writing the one JavaScript holds moves the one Rust read.
    assert_eq!(read.unread.get(), 4);
    read.unread.set(11);
    assert_eq!(card_unread(&value, bridge), 11);
}

/// The current value of the exported `unread` signal, read through JavaScript.
fn card_unread(card: &JsValue, bridge: &Bridge) -> i64 {
    let entries = card.as_object().expect("a struct is an object");
    let (_, signal) = entries
        .iter()
        .find(|(key, _)| key == "unread")
        .expect("the object carries `unread`");
    let runtime = bridge.runtime().expect("a bundle is loaded");
    bridge
        .call(runtime.read_value(), std::slice::from_ref(signal))
        .expect("reading the signal")
        .as_i64()
        .expect("the signal holds a whole number")
}

#[test]
fn the_tagged_variants_read_back() {
    let runtime = runtime();
    let bridge = runtime.bridge();

    for change in [
        Change::Cleared,
        Change::Renamed(String::from("name")),
        Change::Resized {
            width: 1,
            height: 2,
        },
    ] {
        let value = change.clone().into_js(bridge).expect("a variant crosses");
        assert_eq!(
            Change::from_js(&value, bridge).expect("and reads back"),
            change
        );
    }

    let error = Change::from_js(
        &JsValue::Object(vec![(
            String::from("type"),
            JsValue::String(String::from("Exploded")),
        )]),
        bridge,
    )
    .expect_err("an unknown variant fails");
    assert!(
        error.message.contains("Exploded"),
        "the error names the variant: {error}"
    );
}

#[test]
fn a_fixed_length_array_crosses_as_the_tuple_it_declares() {
    let runtime = runtime();
    let bridge = runtime.bridge();

    assert_eq!(
        <[u32; 3] as TsType>::SCHEMA.to_string(),
        "[number, number, number]",
        "the schema declares the length the conversion enforces"
    );

    let value = [1_u32, 2, 3].into_js(bridge).expect("an array crosses");
    assert_eq!(
        value,
        JsValue::Array(vec![
            JsValue::Number(1.0),
            JsValue::Number(2.0),
            JsValue::Number(3.0)
        ])
    );
    assert_eq!(
        <[u32; 3]>::from_js(&value, bridge).expect("and reads back"),
        [1, 2, 3]
    );

    let error = <[u32; 4]>::from_js(&value, bridge)
        .expect_err("an array of another length is not this type");
    assert!(
        error.message.contains("4 items"),
        "the error names the length the type declares: {error}"
    );
}

#[test]
fn an_inexact_number_is_refused_where_the_schema_says_bigint() {
    let runtime = runtime();
    let bridge = runtime.bridge();

    // 2^53 + 1 has no exact double, so a `number` carrying it is a value that
    // already lost a bit: it must not read as a `u64`.
    let error = u64::from_js(&JsValue::Number(9_007_199_254_740_993.0), bridge)
        .expect_err("an inexact number is not a u64");
    assert!(error.message.contains("u64"), "{error}");

    assert_eq!(
        u64::from_js(&JsValue::Number(42.0), bridge).expect("an exact number reads"),
        42,
        "a number that holds the integer exactly is still accepted"
    );
}

#[test]
fn props_cross_one_way_with_their_contract_intact() {
    assert!(matches!(PromoProps::SCHEMA, TypeSchema::Struct(_)));
    assert_eq!(
        PromoProps::CONTRACT_HASH,
        waterui_ts::schema::contract_hash(PromoProps::ENCODED),
        "the props contract is still the one the CLI reads back"
    );

    let runtime = runtime();
    let bridge = runtime.bridge();
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = bridge.open_scope();
    let dismissed = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let props = PromoProps {
        card: Card {
            title: String::from("Promo"),
            status: Status::Idle,
            change: Change::Cleared,
            subtitle: None,
            tags: Vec::new(),
            counts: BTreeMap::new(),
            unread: Binding::container(0_u32),
            identifier: 1,
        },
        on_dismiss: Box::new({
            let dismissed = std::sync::Arc::clone(&dismissed);
            move |count| dismissed.store(count, std::sync::atomic::Ordering::Relaxed)
        }),
    };

    let value = props.into_js(bridge).expect("props cross");
    let entries = value.as_object().expect("props are an object");
    let (_, card) = entries
        .iter()
        .find(|(key, _)| key == "card")
        .expect("the props carry the card");
    assert!(card.as_object().is_some(), "a nested type is an object");

    let (_, callback) = entries
        .iter()
        .find(|(key, _)| key == "on_dismiss")
        .expect("the props carry the callback");
    let JsValue::Function(callback) = callback else {
        panic!("a callback crosses as a function");
    };
    bridge
        .call(callback, &[JsValue::Number(3.0)])
        .expect("JavaScript calls it");
    assert_eq!(dismissed.load(std::sync::atomic::Ordering::Relaxed), 3);
}

// ---------------------------------------------------------------------------
// Loading, the host, and the environment
// ---------------------------------------------------------------------------

#[test]
fn a_bundle_without_a_runtime_is_refused() {
    let runtime =
        TsRuntime::new(Environment::new(), TestHost::default()).expect("the runtime constructs");
    let error = runtime.load("1 + 1").expect_err("a bundle with no runtime");
    assert!(
        error.to_string().contains("installRuntimeGlobal"),
        "the error says what the bundle entry must do: {error}"
    );
}

#[test]
fn a_bundle_with_an_incomplete_runtime_names_the_entry() {
    let runtime =
        TsRuntime::new(Environment::new(), TestHost::default()).expect("the runtime constructs");
    let error = runtime
        .load(INCOMPLETE)
        .expect_err("a runtime missing an entry");
    assert!(
        error.to_string().contains("subscribe"),
        "the error names the entry: {error}"
    );
}

#[test]
fn a_second_bundle_is_refused_before_it_runs() {
    let runtime = runtime();
    let error = runtime
        .load("globalThis.fixture.tampered = true; globalThis.__waterui_runtime = {};")
        .expect_err("one context evaluates one bundle");
    assert!(error.to_string().contains("already loaded"), "{error}");
    assert_eq!(
        eval(&runtime, "globalThis.fixture.tampered"),
        JsValue::Undefined,
        "the refused bundle never ran"
    );
    assert_eq!(
        eval(&runtime, "typeof globalThis.__waterui_runtime.read"),
        JsValue::String(String::from("function")),
        "so the runtime the first bundle published is still there"
    );
}

#[test]
fn a_bundle_cannot_replace_a_host_function_before_it_is_installed() {
    let host = TestHost::default();
    let runtime = TsRuntime::new(Environment::new(), host.clone()).expect("the runtime constructs");
    // `__waterui_host` is an ordinary mutable global, and this bundle
    // reassigns two of its entries on the way to installing the runtime: the
    // one the table carries, and the one every callback dispatches through.
    let tampered = format!(
        "globalThis.__waterui_host.create = () => 'hijacked';
         globalThis.__waterui_host.invoke = () => 'hijacked';\n{FIXTURE}"
    );
    runtime.load(&tampered).expect("the bundle loads");

    eval(
        &runtime,
        "globalThis.fixture.hold(globalThis.fixture.host.create('VStack', {}, []))",
    );
    assert_eq!(
        *host.calls.borrow(),
        vec![Call::Create(String::from("VStack"))],
        "the installed table carries the function the engine registered"
    );

    let bridge = runtime.bridge();
    let _scope = bridge.open_scope();
    let called = Rc::new(Cell::new(0_u32));
    let callback: Box<dyn Fn(u32)> = Box::new({
        let called = Rc::clone(&called);
        move |value| called.set(value)
    });
    let exported = callback.into_js(bridge).expect("a callback exports");
    let hold = function(&runtime, "globalThis.fixture.hold");
    bridge
        .call(&hold, &[exported])
        .expect("JavaScript holds it");
    eval(&runtime, "globalThis.fixture.held(5)");
    assert_eq!(
        called.get(),
        5,
        "and a callback dispatches through the invoke the engine registered"
    );
}

#[test]
fn a_bundle_whose_host_installation_fails_can_be_loaded_again() {
    let runtime =
        TsRuntime::new(Environment::new(), TestHost::default()).expect("the runtime constructs");
    let refusing = format!(
        "{FIXTURE}
         globalThis.__waterui_runtime = {{
           ...globalThis.__waterui_runtime,
           installHost: () => {{ throw new Error('this host is not welcome'); }},
         }};"
    );

    let error = runtime
        .load(&refusing)
        .expect_err("the host installation throws");
    assert!(error.to_string().contains("not welcome"), "{error}");

    // Nothing was published, so the runtime is still loadable.
    runtime.load(FIXTURE).expect("a second attempt loads");
    assert_eq!(
        eval(&runtime, "typeof globalThis.fixture.host.create"),
        JsValue::String(String::from("function")),
        "and the host it installs is the real one"
    );
}

#[test]
fn a_bundle_that_published_before_it_threw_leaves_nothing_for_the_next_one() {
    let runtime =
        TsRuntime::new(Environment::new(), TestHost::default()).expect("the runtime constructs");
    let throwing = format!("{FIXTURE}\nthrow new Error('the bundle failed after publishing');");
    let error = runtime.load(&throwing).expect_err("the bundle throws");
    assert!(
        error.to_string().contains("failed after publishing"),
        "{error}"
    );

    // The next bundle is judged on what it published, never on the residue
    // of the one before it.
    let error = runtime
        .load("1 + 1")
        .expect_err("a bundle that publishes no runtime");
    assert!(
        error.to_string().contains("installRuntimeGlobal"),
        "the bundle that published nothing is refused: {error}"
    );
}

#[test]
fn a_loaded_bundle_carries_its_modules() {
    let runtime = runtime();
    runtime
        .module("src/promo.tsx")
        .expect("the bundle carries the module");
    let error = runtime
        .module("src/missing.tsx")
        .expect_err("and carries no other");
    assert!(error.to_string().contains("src/missing.tsx"), "{error}");
}

#[test]
fn the_host_table_is_installed_and_reached_from_javascript() {
    let host = TestHost::default();
    let runtime = with_host(host.clone());

    assert_eq!(
        eval(&runtime, "globalThis.fixture.host.modifiers.has('padding')"),
        JsValue::Bool(true),
        "the modifier names crossed as an array and became a set"
    );

    eval(
        &runtime,
        "globalThis.fixture.hold(globalThis.__waterui_host.create('VStack', {}, []))",
    );
    eval(
        &runtime,
        "globalThis.__waterui_host.modify(globalThis.fixture.held, 'padding', 8)",
    );
    assert_eq!(
        *host.calls.borrow(),
        vec![
            Call::Create(String::from("VStack")),
            Call::Modify(String::from("padding"))
        ]
    );

    let error = eval_error(
        &runtime,
        "globalThis.__waterui_host.modify(globalThis.fixture.held, 'padding', 8)",
    );
    assert!(
        error.message.contains("already taken"),
        "a handle is consumed by the call that receives it: {error}"
    );
}

#[test]
fn the_environment_reaches_javascript_as_theme_and_locale() {
    let environment = Environment::new()
        .store::<ColorScheme, Computed<ColorScheme>>(Computed::constant(ColorScheme::Dark))
        .store::<AccentColor, Computed<ResolvedColor>>(Computed::constant(ResolvedColor {
            red: 0.25,
            green: 0.5,
            blue: 0.75,
            headroom: 1.0,
            opacity: 1.0,
        }));
    let runtime = with_environment(environment);
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = runtime.bridge().open_scope();

    eval(
        &runtime,
        "globalThis.fixture.held = globalThis.__waterui_host.environment()",
    );
    assert_eq!(
        eval(
            &runtime,
            "globalThis.__waterui_runtime.read(globalThis.fixture.held.theme).colorScheme"
        ),
        JsValue::String(String::from("dark"))
    );
    assert_eq!(
        eval(
            &runtime,
            "globalThis.__waterui_runtime.read(globalThis.fixture.held.theme).accent.blue"
        ),
        JsValue::Number(0.75),
        "an installed colour token is there, in linear light"
    );
    assert_eq!(
        eval(
            &runtime,
            "globalThis.__waterui_runtime.read(globalThis.fixture.held.theme).background"
        ),
        JsValue::Undefined,
        "a token the environment does not install is absent, not defaulted"
    );
    assert!(
        matches!(
            eval(
                &runtime,
                "typeof globalThis.__waterui_runtime.read(globalThis.fixture.held.locale).identifier"
            ),
            JsValue::String(ref kind) if kind == "string"
        ),
        "the locale carries its identifier"
    );
    assert_eq!(
        eval(&runtime, "globalThis.fixture.held.safeArea"),
        JsValue::Undefined,
        "WaterUI publishes no ambient safe-area value, and does not invent one"
    );
}

#[test]
fn a_theme_change_reaches_javascript() {
    let scheme = Binding::container(ColorScheme::Light);
    let environment = Environment::new()
        .store::<ColorScheme, Computed<ColorScheme>>(Computed::new(scheme.clone()));
    let runtime = with_environment(environment);
    // Everything exported into JavaScript belongs to the mount that asked
    // for it; the mount leaf opens this scope, a test opens its own.
    let _scope = runtime.bridge().open_scope();

    eval(
        &runtime,
        "globalThis.fixture.held = globalThis.__waterui_host.environment()",
    );
    assert_eq!(
        eval(
            &runtime,
            "globalThis.__waterui_runtime.read(globalThis.fixture.held.theme).colorScheme"
        ),
        JsValue::String(String::from("light"))
    );

    scheme.set(ColorScheme::Dark);
    assert_eq!(
        eval(
            &runtime,
            "globalThis.__waterui_runtime.read(globalThis.fixture.held.theme).colorScheme"
        ),
        JsValue::String(String::from("dark")),
        "the whole theme is one reactive value, pushed on every change"
    );
}

#[test]
fn an_environment_without_a_colour_scheme_says_so() {
    let runtime = runtime();
    let _scope = runtime.bridge().open_scope();
    let error = eval_error(&runtime, "globalThis.__waterui_host.environment()");
    assert!(
        error.message.contains("ColorScheme"),
        "the error names what a backend must install: {error}"
    );
}

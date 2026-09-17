//! The contract every `JsRuntime` implementation is held to.
//!
//! Each function here is one conformance check, generic over the engine.
//! Engine crates expand them into `#[test]` functions with
//! [`conformance_tests!`](crate::conformance_tests):
//!
//! ```ignore
//! waterui_ts_engine::conformance_tests!(MyEngine);
//! ```

use std::rc::Rc;

use crate::{BigInt, JsError, JsRuntime, JsValue, Opaque};

fn runtime<R: JsRuntime>() -> R {
    R::new().expect("the engine constructs")
}

fn identity_function<R: JsRuntime>(runtime: &R) -> crate::JsFunction {
    let JsValue::Function(function) = runtime
        .eval("(function (v) { return v; })", "conformance.js")
        .expect("identity evaluates")
    else {
        panic!("a function expression converts to JsValue::Function");
    };
    function
}

/// Every `JsValue` kind is produced by `eval`.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn eval_returns_each_value_kind<R: JsRuntime>() {
    let rt = runtime::<R>();
    let eval = |source: &str| rt.eval(source, "conformance.js").expect(source);

    assert_eq!(eval("undefined"), JsValue::Undefined);
    assert_eq!(eval("null"), JsValue::Null);
    assert_eq!(eval("true"), JsValue::Bool(true));
    assert_eq!(eval("1.5"), JsValue::Number(1.5));
    assert_eq!(eval("42"), JsValue::Number(42.0));
    assert_eq!(
        eval("42n"),
        JsValue::BigInt(BigInt::Signed(42)),
        "a small bigint converts exactly"
    );
    assert_eq!(eval("'hello'"), JsValue::String(String::from("hello")));
    assert_eq!(
        eval("[1, 'two', true]"),
        JsValue::Array(vec![
            JsValue::Number(1.0),
            JsValue::String(String::from("two")),
            JsValue::Bool(true),
        ])
    );
    assert_eq!(
        eval("({a: 1, b: 'x'})"),
        JsValue::Object(vec![
            (String::from("a"), JsValue::Number(1.0)),
            (String::from("b"), JsValue::String(String::from("x"))),
        ]),
        "object entries keep insertion order"
    );
    assert!(
        matches!(eval("(function () {})"), JsValue::Function(_)),
        "a function converts to a held JsFunction"
    );
}

/// Every `JsValue` kind is accepted by `call`.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn call_passes_each_value_kind<R: JsRuntime>() {
    let rt = runtime::<R>();
    let identity = identity_function(&rt);

    for value in [
        JsValue::Undefined,
        JsValue::Null,
        JsValue::Bool(true),
        JsValue::Number(-0.5),
        JsValue::BigInt(BigInt::Signed(i64::MIN)),
        JsValue::BigInt(BigInt::Unsigned(u64::MAX)),
        JsValue::String(String::from("text")),
        JsValue::Array(vec![JsValue::Null, JsValue::Number(1.0)]),
        JsValue::Object(vec![(String::from("k"), JsValue::Bool(false))]),
    ] {
        assert_eq!(
            rt.call(&identity, std::slice::from_ref(&value))
                .expect("call"),
            value
        );
    }

    // A function argument crosses as a callable, not a copy of anything.
    let returned = rt
        .call(&identity, &[JsValue::Function(identity.clone())])
        .expect("call");
    assert!(matches!(returned, JsValue::Function(_)));
}

/// JavaScript calls into Rust and reads the returned value.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn host_functions_round_trip<R: JsRuntime>() {
    let rt = runtime::<R>();
    rt.register("add", |args| {
        let a = args.first().and_then(JsValue::as_f64).unwrap_or(0.0);
        let b = args.get(1).and_then(JsValue::as_f64).unwrap_or(0.0);
        Ok(JsValue::Number(a + b))
    })
    .expect("registers");
    rt.register("describe", |args| {
        let name = args
            .first()
            .and_then(JsValue::as_str)
            .unwrap_or("<none>")
            .to_owned();
        Ok(JsValue::Object(vec![
            (String::from("name"), JsValue::String(name)),
            (String::from("count"), JsValue::Number(2.0)),
        ]))
    })
    .expect("registers");

    assert_eq!(
        rt.eval("__waterui_host.add(2, 3)", "conformance.js")
            .expect("host call"),
        JsValue::Number(5.0)
    );
    assert_eq!(
        rt.eval("__waterui_host.describe('item').name", "conformance.js")
            .expect("host call"),
        JsValue::String(String::from("item"))
    );
}

/// `i64::MAX`, `i64::MIN` and `u64::MAX` round-trip through `bigint` exactly.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn big_integers_round_trip_exactly<R: JsRuntime>() {
    let rt = runtime::<R>();
    let identity = identity_function(&rt);

    for value in [
        JsValue::from(i64::MAX),
        JsValue::from(i64::MIN),
        JsValue::from(u64::MAX),
    ] {
        assert_eq!(
            rt.call(&identity, std::slice::from_ref(&value))
                .expect("call"),
            value
        );
    }

    // And the same values arriving from JavaScript literals.
    assert_eq!(
        rt.eval("9223372036854775807n", "conformance.js").unwrap(),
        JsValue::BigInt(BigInt::Signed(i64::MAX))
    );
    assert_eq!(
        rt.eval("(-9223372036854775808n)", "conformance.js")
            .unwrap(),
        JsValue::BigInt(BigInt::Signed(i64::MIN))
    );
    assert_eq!(
        rt.eval("18446744073709551615n", "conformance.js").unwrap(),
        JsValue::BigInt(BigInt::Unsigned(u64::MAX))
    );
}

/// An object inside an array inside an object survives intact.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn nested_values_round_trip<R: JsRuntime>() {
    let rt = runtime::<R>();
    let identity = identity_function(&rt);
    let value = JsValue::Object(vec![
        (
            String::from("list"),
            JsValue::Array(vec![
                JsValue::Number(1.0),
                JsValue::Object(vec![(String::from("deep"), JsValue::Null)]),
            ]),
        ),
        (String::from("id"), JsValue::from(i64::MAX)),
        (String::from("flag"), JsValue::Bool(true)),
    ]);
    assert_eq!(
        rt.call(&identity, std::slice::from_ref(&value))
            .expect("call"),
        value
    );
}

/// A thrown JavaScript `Error` arrives as `JsError` with its stack.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn js_exceptions_arrive_with_stack<R: JsRuntime>() {
    let rt = runtime::<R>();
    let error = rt
        .eval(
            "(function () { function inner() { throw new TypeError('boom'); } inner(); })()",
            "conformance.js",
        )
        .expect_err("the throw propagates");
    assert_eq!(error.name, "TypeError");
    assert!(error.message.contains("boom"), "{}", error.message);
    assert!(error.stack.is_some(), "the JavaScript stack is carried");

    // A syntax error is an error too, not a panic and not `undefined`.
    let error = rt.eval("(", "conformance.js").expect_err("fails to parse");
    assert!(!error.message.is_empty());
}

/// A `Err` from a host function throws in JavaScript, catchably.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn host_errors_are_catchable_in_js<R: JsRuntime>() {
    let rt = runtime::<R>();
    rt.register("fail", |_| Err(JsError::new("Error", "host says no")))
        .expect("registers");
    assert_eq!(
        rt.eval(
            "try { __waterui_host.fail(); 'unreached' } catch (e) { e.message }",
            "conformance.js",
        )
        .expect("the catch path evaluates"),
        JsValue::String(String::from("host says no"))
    );
}

/// An opaque `Rc` survives a JavaScript round trip and downcasts to the same
/// allocation.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn opaque_values_round_trip<R: JsRuntime>() {
    let rt = runtime::<R>();
    let identity = identity_function(&rt);
    let value = Rc::new(String::from("state"));
    let returned = rt
        .call(&identity, &[JsValue::Opaque(Opaque::new(value.clone()))])
        .expect("call");
    let JsValue::Opaque(opaque) = returned else {
        panic!("the box comes back as JsValue::Opaque");
    };
    let back = opaque.downcast::<String>().expect("the same type");
    assert!(Rc::ptr_eq(&back, &value), "the same allocation");
    assert_eq!(back.as_str(), "state");
}

/// A `JsFunction` held in Rust keeps working across calls.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn held_functions_call_repeatedly<R: JsRuntime>() {
    let rt = runtime::<R>();
    let JsValue::Function(add) = rt
        .eval("(function (a, b) { return a + b; })", "conformance.js")
        .expect("evaluates")
    else {
        panic!("a function expression converts to JsValue::Function");
    };
    assert_eq!(
        rt.call(&add, &[1.0.into(), 2.0.into()]).expect("call"),
        JsValue::Number(3.0)
    );
    assert_eq!(
        rt.call(&add, &[10.0.into(), 5.0.into()]).expect("call"),
        JsValue::Number(15.0)
    );
}

/// A retained object handle passes the same object back into JavaScript.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn retained_objects_stay_live<R: JsRuntime>() {
    let rt = runtime::<R>();
    let object = rt
        .retain(&JsValue::Object(vec![(
            String::from("answer"),
            JsValue::Number(42.0),
        )]))
        .expect("retains");
    let JsValue::Function(read) = rt
        .eval(
            "(function (o) { o.seen = true; return o.answer; })",
            "conformance.js",
        )
        .expect("evaluates")
    else {
        panic!("a function expression converts to JsValue::Function");
    };
    assert_eq!(
        rt.call(&read, &[JsValue::ObjectRef(object)]).expect("call"),
        JsValue::Number(42.0)
    );
}

/// Values with no `JsValue` are typed errors, never silent `undefined`.
///
/// # Panics
///
/// Panics on conformance failure — each function is a test body.
pub fn unconvertible_values_are_typed_errors<R: JsRuntime>() {
    let rt = runtime::<R>();
    assert!(
        rt.eval("Symbol('x')", "conformance.js").is_err(),
        "a symbol cannot convert"
    );
    assert!(
        rt.eval("(2n ** 100n)", "conformance.js").is_err(),
        "a bigint beyond u64 cannot convert"
    );
}

/// Expands the conformance suite into `#[test]` functions for an engine.
///
/// ```ignore
/// waterui_ts_engine::conformance_tests!(QuickJsRuntime);
/// ```
#[macro_export]
macro_rules! conformance_tests {
    ($engine:ty) => {
        /// Every `JsValue` kind is produced by `eval`.
        #[test]
        fn eval_returns_each_value_kind() {
            $crate::conformance::eval_returns_each_value_kind::<$engine>();
        }

        /// Every `JsValue` kind is accepted by `call`.
        #[test]
        fn call_passes_each_value_kind() {
            $crate::conformance::call_passes_each_value_kind::<$engine>();
        }

        /// JavaScript calls into Rust and reads the returned value.
        #[test]
        fn host_functions_round_trip() {
            $crate::conformance::host_functions_round_trip::<$engine>();
        }

        /// `i64::MAX`, `i64::MIN` and `u64::MAX` round-trip exactly.
        #[test]
        fn big_integers_round_trip_exactly() {
            $crate::conformance::big_integers_round_trip_exactly::<$engine>();
        }

        /// An object inside an array inside an object survives intact.
        #[test]
        fn nested_values_round_trip() {
            $crate::conformance::nested_values_round_trip::<$engine>();
        }

        /// A thrown JavaScript `Error` arrives as `JsError` with its stack.
        #[test]
        fn js_exceptions_arrive_with_stack() {
            $crate::conformance::js_exceptions_arrive_with_stack::<$engine>();
        }

        /// A `Err` from a host function throws in JavaScript, catchably.
        #[test]
        fn host_errors_are_catchable_in_js() {
            $crate::conformance::host_errors_are_catchable_in_js::<$engine>();
        }

        /// An opaque `Rc` survives a JavaScript round trip.
        #[test]
        fn opaque_values_round_trip() {
            $crate::conformance::opaque_values_round_trip::<$engine>();
        }

        /// A `JsFunction` held in Rust keeps working across calls.
        #[test]
        fn held_functions_call_repeatedly() {
            $crate::conformance::held_functions_call_repeatedly::<$engine>();
        }

        /// A retained object handle passes the same object back.
        #[test]
        fn retained_objects_stay_live() {
            $crate::conformance::retained_objects_stay_live::<$engine>();
        }

        /// Values with no `JsValue` are typed errors.
        #[test]
        fn unconvertible_values_are_typed_errors() {
            $crate::conformance::unconvertible_values_are_typed_errors::<$engine>();
        }
    };
}

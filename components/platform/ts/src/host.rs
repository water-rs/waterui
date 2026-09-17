//! The native side of the runtime: the host table, and how it is installed.
//!
//! [`HostTable`] is the Rust half of the `Host` interface in HOST.md. The
//! component bindings that implement it — resolving `"VStack"` against the
//! catalog, applying `"padding"`, reconciling `each` — are the host-table leaf
//! (water-rs/waterui#1042); what lives here is the seam: the host functions
//! the engine registers, the argument shapes they hold JavaScript to, and the
//! table object `installHost` receives.
//!
//! Two entries never reach the table. `environment()` is answered from the
//! `Environment` the runtime was constructed with, because the framework owns
//! those values, and `invoke` is the bridge's own: it dispatches every
//! registered Rust closure, from a prop callback to a signal subscription.

use std::rc::Rc;

use suiteki::Str;
use waterui_core::{AnyView, Error};
use waterui_ts_engine::{JsError, JsFunction, JsRuntime, JsValue};

use crate::bridge::{Bridge, WeakBridge};
use crate::environment::host_environment;
use crate::error::{TsError, kind_of};
use crate::view::ViewSlot;

/// The native views a TypeScript module's JSX turns into.
///
/// Every value a method is handed is exactly what HOST.md describes: `config`
/// entries and modifier values are reactive inputs, materialized with
/// [`Bridge::materialize_binding`] or [`Bridge::materialize_computed`] at the
/// moment they reach a native view; `children` are view slots, strings,
/// numbers or accessors; `render` callbacks return `{ handle, dispose }`
/// branches the implementation drives.
///
/// An error is thrown into JavaScript where the call was made, with its
/// message intact.
pub trait HostTable: 'static {
    /// The attribute names the component catalog declares as modifiers.
    ///
    /// Read once at install: the runtime keeps the set and classifies
    /// attributes locally, so this never costs a crossing per element.
    fn modifiers(&self) -> Vec<Str>;

    /// Creates a native view for a JSX element.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] for a component the catalog does not have, or a
    /// configuration value the component cannot take.
    fn create(
        &self,
        bridge: &Bridge,
        component: &str,
        config: &[(String, JsValue)],
        children: &[JsValue],
    ) -> Result<AnyView, Error>;

    /// Applies one modifier, in the order the attributes were written.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] for a modifier the catalog does not have, or a value
    /// it cannot take.
    fn modify(
        &self,
        bridge: &Bridge,
        view: AnyView,
        name: &str,
        value: &JsValue,
    ) -> Result<AnyView, Error>;

    /// Materializes a text leaf, localized like Rust's own `text(…)`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the content is neither text nor a reactive value
    /// producing text.
    fn text(&self, bridge: &Bridge, content: &JsValue) -> Result<AnyView, Error>;

    /// `<Show>`: presents one branch or the other as `when` changes.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when a branch cannot be rendered.
    fn show(
        &self,
        bridge: &Bridge,
        when: &JsValue,
        render: &JsFunction,
        fallback: Option<&JsFunction>,
    ) -> Result<AnyView, Error>;

    /// `<For>`: reconciles a list by identity.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the list or a branch cannot be rendered.
    fn each(
        &self,
        bridge: &Bridge,
        items: &JsValue,
        render: &JsFunction,
        by: Option<&JsFunction>,
    ) -> Result<AnyView, Error>;

    /// `<Suspense>`: presents the fallback while the children are pending.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when a branch cannot be rendered.
    fn suspense(
        &self,
        bridge: &Bridge,
        children: &JsFunction,
        fallback: Option<&JsFunction>,
    ) -> Result<AnyView, Error>;
}

/// The property of `__waterui_host` one host function is registered under,
/// and the expression that reads it back for the table object.
struct HostEntry {
    name: &'static str,
    source: &'static str,
}

const CREATE: HostEntry = HostEntry {
    name: "create",
    source: "globalThis.__waterui_host.create",
};
const MODIFY: HostEntry = HostEntry {
    name: "modify",
    source: "globalThis.__waterui_host.modify",
};
const TEXT: HostEntry = HostEntry {
    name: "text",
    source: "globalThis.__waterui_host.text",
};
const SHOW: HostEntry = HostEntry {
    name: "show",
    source: "globalThis.__waterui_host.show",
};
const EACH: HostEntry = HostEntry {
    name: "each",
    source: "globalThis.__waterui_host.each",
};
const SUSPENSE: HostEntry = HostEntry {
    name: "suspense",
    source: "globalThis.__waterui_host.suspense",
};
const ENVIRONMENT: HostEntry = HostEntry {
    name: "environment",
    source: "globalThis.__waterui_host.environment",
};

/// The entry every registered Rust closure is dispatched through.
const INVOKE: &str = "invoke";

/// The script name the host reads are attributed to in a stack trace.
const SOURCE_NAME: &str = "waterui:host";

/// Registers every host function on the engine.
///
/// Registration happens before the bundle is evaluated, so the functions exist
/// no matter when JavaScript first reaches for one.
///
/// # Errors
///
/// Returns [`JsError`] when the engine cannot install a function.
pub fn register(bridge: &Bridge, table: &Rc<dyn HostTable>) -> Result<(), JsError> {
    let engine = bridge.engine();

    engine.register(INVOKE, {
        let callbacks = Rc::clone(bridge.callbacks());
        move |args: &[JsValue]| {
            let id = args
                .first()
                .and_then(JsValue::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| {
                    JsError::conversion(
                        "__waterui_host.invoke(id, …) takes the callback id makeCallback was \
                         given as its first argument",
                    )
                })?;
            callbacks.invoke(id, args.get(1..).unwrap_or_default())
        }
    })?;

    engine.register(
        CREATE.name,
        host_call(bridge, table, |table, bridge, args| {
            let component = string_argument(args, 0, "create(component, config, children)")?;
            let config = object_argument(args, 1, "create(component, config, children)")?;
            let children = array_argument(args, 2, "create(component, config, children)")?;
            table.create(bridge, component, config, children)
        }),
    )?;

    engine.register(
        MODIFY.name,
        host_call(bridge, table, |table, bridge, args| {
            let view = bridge.take_view(argument(args, 0, "modify(handle, name, value)")?)?;
            let name = string_argument(args, 1, "modify(handle, name, value)")?;
            let value = argument(args, 2, "modify(handle, name, value)")?;
            table.modify(bridge, view, name, value)
        }),
    )?;

    engine.register(
        TEXT.name,
        host_call(bridge, table, |table, bridge, args| {
            table.text(bridge, argument(args, 0, "text(content)")?)
        }),
    )?;

    engine.register(
        SHOW.name,
        host_call(bridge, table, |table, bridge, args| {
            let when = argument(args, 0, "show(when, render, fallback?)")?;
            let render = function_argument(args, 1, "show(when, render, fallback?)")?;
            let fallback = optional_function_argument(args, 2, "show(when, render, fallback?)")?;
            table.show(bridge, when, render, fallback)
        }),
    )?;

    engine.register(
        EACH.name,
        host_call(bridge, table, |table, bridge, args| {
            let items = argument(args, 0, "each(each, render, by?)")?;
            let render = function_argument(args, 1, "each(each, render, by?)")?;
            let by = optional_function_argument(args, 2, "each(each, render, by?)")?;
            table.each(bridge, items, render, by)
        }),
    )?;

    engine.register(
        SUSPENSE.name,
        host_call(bridge, table, |table, bridge, args| {
            let children = function_argument(args, 0, "suspense(children, fallback?)")?;
            let fallback = optional_function_argument(args, 1, "suspense(children, fallback?)")?;
            table.suspense(bridge, children, fallback)
        }),
    )?;

    engine.register(ENVIRONMENT.name, {
        let weak = bridge.downgrade();
        move |_args: &[JsValue]| host_environment(&upgrade(&weak)?)
    })?;

    Ok(())
}

/// Installs the table object on the JavaScript side.
///
/// Called once, after the bundle is evaluated and before anything is mounted:
/// the installer itself lives in the bundle, so it cannot run earlier.
///
/// # Errors
///
/// Returns [`TsError`] when a host function cannot be read back, or when
/// `installHost` rejects the table.
pub fn install(bridge: &Bridge, table: &Rc<dyn HostTable>) -> Result<(), TsError> {
    let entries = [CREATE, MODIFY, TEXT, SHOW, EACH, SUSPENSE, ENVIRONMENT];
    let mut object = Vec::with_capacity(entries.len() + 1);
    for entry in entries {
        match bridge.engine().eval(entry.source, SOURCE_NAME)? {
            JsValue::Function(function) => {
                object.push((entry.name.to_owned(), JsValue::Function(function)));
            }
            other => {
                return Err(TsError::RuntimeEntry {
                    name: entry.name,
                    found: kind_of(&other),
                });
            }
        }
    }
    // A `Set` is not a plain object and cannot cross the seam, so the names
    // travel as an array and `installHost` builds the set once.
    object.push((
        String::from("modifiers"),
        JsValue::Array(
            table
                .modifiers()
                .into_iter()
                .map(|name| JsValue::String(name.as_str().to_owned()))
                .collect(),
        ),
    ));

    let runtime = bridge.runtime()?;
    bridge.call(runtime.install_host(), &[JsValue::Object(object)])?;
    Ok(())
}

/// Wraps one table method as a host function: upgrade, call, wrap the view.
fn host_call(
    bridge: &Bridge,
    table: &Rc<dyn HostTable>,
    call: impl Fn(&dyn HostTable, &Bridge, &[JsValue]) -> Result<AnyView, Error> + 'static,
) -> impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static {
    let weak = bridge.downgrade();
    let table = Rc::clone(table);
    move |args: &[JsValue]| {
        let bridge = upgrade(&weak)?;
        let view = call(table.as_ref(), &bridge, args)
            .map_err(|error| JsError::new("Error", format!("{error:#}")))?;
        Ok(ViewSlot::new(view).to_js_value())
    }
}

/// The bridge, or the error JavaScript sees once the runtime is gone.
fn upgrade(weak: &WeakBridge) -> Result<Bridge, JsError> {
    weak.upgrade().ok_or_else(|| {
        JsError::new(
            "Error",
            "the TypeScript runtime was dropped while JavaScript was calling into it",
        )
    })
}

/// One argument, or an error naming the call that wanted it.
fn argument<'a>(args: &'a [JsValue], index: usize, call: &str) -> Result<&'a JsValue, JsError> {
    args.get(index).ok_or_else(|| {
        JsError::conversion(format!(
            "{call} was called with fewer than {} arguments",
            index + 1
        ))
    })
}

fn string_argument<'a>(args: &'a [JsValue], index: usize, call: &str) -> Result<&'a str, JsError> {
    let value = argument(args, index, call)?;
    value.as_str().ok_or_else(|| {
        JsError::conversion(format!(
            "{call}: argument {index} is {}, not a string",
            kind_of(value)
        ))
    })
}

fn object_argument<'a>(
    args: &'a [JsValue],
    index: usize,
    call: &str,
) -> Result<&'a [(String, JsValue)], JsError> {
    let value = argument(args, index, call)?;
    value.as_object().ok_or_else(|| {
        JsError::conversion(format!(
            "{call}: argument {index} is {}, not an object",
            kind_of(value)
        ))
    })
}

fn array_argument<'a>(
    args: &'a [JsValue],
    index: usize,
    call: &str,
) -> Result<&'a [JsValue], JsError> {
    let value = argument(args, index, call)?;
    value.as_array().ok_or_else(|| {
        JsError::conversion(format!(
            "{call}: argument {index} is {}, not an array",
            kind_of(value)
        ))
    })
}

fn function_argument<'a>(
    args: &'a [JsValue],
    index: usize,
    call: &str,
) -> Result<&'a JsFunction, JsError> {
    match argument(args, index, call)? {
        JsValue::Function(function) => Ok(function),
        other => Err(JsError::conversion(format!(
            "{call}: argument {index} is {}, not a function",
            kind_of(other)
        ))),
    }
}

/// An optional trailing function argument: absent, `undefined` and `null` all
/// mean the caller did not pass one.
fn optional_function_argument<'a>(
    args: &'a [JsValue],
    index: usize,
    call: &str,
) -> Result<Option<&'a JsFunction>, JsError> {
    match args.get(index) {
        None | Some(JsValue::Undefined | JsValue::Null) => Ok(None),
        Some(JsValue::Function(function)) => Ok(Some(function)),
        Some(other) => Err(JsError::conversion(format!(
            "{call}: argument {index} is {}, not a function",
            kind_of(other)
        ))),
    }
}

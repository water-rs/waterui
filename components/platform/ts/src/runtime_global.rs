//! `globalThis.__waterui_runtime`: what a bundle hands the bridge.
//!
//! A bundle is a classic script, so nothing it declares is reachable from
//! Rust. Its entry therefore ends with `installRuntimeGlobal(modules)`, which
//! publishes the reactive helpers, the host installer, `mount` and the module
//! table on one global object. [`RuntimeGlobal::read`] picks that object apart
//! right after `eval` and refuses a bundle that is missing an entry, naming
//! it.
//!
//! Each entry is read through the exact expression spelled next to it, so the
//! only JavaScript this module contains is a fixed list of property reads.

use std::collections::BTreeMap;

use suiteki::Str;
use waterui_ts_engine::{JsFunction, JsRuntime, JsValue};

use crate::error::{TsError, kind_of};

/// The script name the runtime reads are attributed to in a stack trace.
const SOURCE_NAME: &str = "waterui:runtime-global";

/// One entry of the runtime global.
struct Entry {
    /// The property name, for error messages.
    name: &'static str,
    /// The expression that reads it.
    source: &'static str,
}

/// The whole object, read first so a bundle without the runtime is one clear
/// error rather than a missing-property error per entry.
const RUNTIME_SOURCE: &str = "globalThis.__waterui_runtime";

const INSTALL_HOST: Entry = Entry {
    name: "installHost",
    source: "globalThis.__waterui_runtime.installHost",
};
const UNINSTALL_HOST: Entry = Entry {
    name: "uninstallHost",
    source: "globalThis.__waterui_runtime.uninstallHost",
};
const MOUNT: Entry = Entry {
    name: "mount",
    source: "globalThis.__waterui_runtime.mount",
};
const IS_SIGNAL: Entry = Entry {
    name: "isSignal",
    source: "globalThis.__waterui_runtime.isSignal",
};
const IS_ACCESSOR: Entry = Entry {
    name: "isAccessor",
    source: "globalThis.__waterui_runtime.isAccessor",
};
const READ: Entry = Entry {
    name: "read",
    source: "globalThis.__waterui_runtime.read",
};
const WRITE: Entry = Entry {
    name: "write",
    source: "globalThis.__waterui_runtime.write",
};
const SUBSCRIBE: Entry = Entry {
    name: "subscribe",
    source: "globalThis.__waterui_runtime.subscribe",
};
const TO_SIGNAL: Entry = Entry {
    name: "toSignal",
    source: "globalThis.__waterui_runtime.toSignal",
};
const TO_ACCESSOR: Entry = Entry {
    name: "toAccessor",
    source: "globalThis.__waterui_runtime.toAccessor",
};
const CREATE_SIGNAL: Entry = Entry {
    name: "createSignal",
    source: "globalThis.__waterui_runtime.createSignal",
};
const CREATE_MEMO: Entry = Entry {
    name: "createMemo",
    source: "globalThis.__waterui_runtime.createMemo",
};
const MAKE_CALLBACK: Entry = Entry {
    name: "makeCallback",
    source: "globalThis.__waterui_runtime.makeCallback",
};
const MODULES: Entry = Entry {
    name: "modules",
    source: "globalThis.__waterui_runtime.modules",
};

/// The runtime a loaded bundle published, resolved to callable handles.
///
/// Every entry is a function the bridge calls; `modules` is the table a
/// mounted module is looked up in. The accessors hand out the handle to pass
/// to [`JsRuntime::call`](waterui_ts_engine::JsRuntime::call).
#[derive(Debug)]
pub struct RuntimeGlobal {
    install_host: JsFunction,
    uninstall_host: JsFunction,
    mount: JsFunction,
    is_signal: JsFunction,
    is_accessor: JsFunction,
    read: JsFunction,
    write: JsFunction,
    subscribe: JsFunction,
    to_signal: JsFunction,
    to_accessor: JsFunction,
    create_signal: JsFunction,
    create_memo: JsFunction,
    make_callback: JsFunction,
    modules: BTreeMap<Str, JsFunction>,
}

impl RuntimeGlobal {
    /// Reads the runtime a just-evaluated bundle published.
    ///
    /// # Errors
    ///
    /// [`TsError::MissingRuntimeGlobal`] when the bundle published nothing,
    /// [`TsError::RuntimeEntry`] when an entry is missing or is not a
    /// function, and [`TsError::Module`] when the module table holds something
    /// that is not a module function.
    pub(crate) fn read<R: JsRuntime>(engine: &R) -> Result<Self, TsError> {
        match engine.eval(RUNTIME_SOURCE, SOURCE_NAME)? {
            JsValue::Object(_) => {}
            _ => return Err(TsError::MissingRuntimeGlobal),
        }

        Ok(Self {
            install_host: function(engine, &INSTALL_HOST)?,
            uninstall_host: function(engine, &UNINSTALL_HOST)?,
            mount: function(engine, &MOUNT)?,
            is_signal: function(engine, &IS_SIGNAL)?,
            is_accessor: function(engine, &IS_ACCESSOR)?,
            read: function(engine, &READ)?,
            write: function(engine, &WRITE)?,
            subscribe: function(engine, &SUBSCRIBE)?,
            to_signal: function(engine, &TO_SIGNAL)?,
            to_accessor: function(engine, &TO_ACCESSOR)?,
            create_signal: function(engine, &CREATE_SIGNAL)?,
            create_memo: function(engine, &CREATE_MEMO)?,
            make_callback: function(engine, &MAKE_CALLBACK)?,
            modules: modules(engine)?,
        })
    }

    /// `installHost(host)` — installs the native host table.
    #[must_use]
    pub const fn install_host(&self) -> &JsFunction {
        &self.install_host
    }

    /// `uninstallHost()` — drops the installed host when a bundle unloads.
    #[must_use]
    pub const fn uninstall_host(&self) -> &JsFunction {
        &self.uninstall_host
    }

    /// `mount(render)` — mounts one module under a fresh root scope.
    #[must_use]
    pub const fn mount(&self) -> &JsFunction {
        &self.mount
    }

    /// `isSignal(value)` — whether a reactive input is writable.
    #[must_use]
    pub const fn is_signal(&self) -> &JsFunction {
        &self.is_signal
    }

    /// `isAccessor(value)` — whether a value is readable as a reactive value.
    #[must_use]
    pub const fn is_accessor(&self) -> &JsFunction {
        &self.is_accessor
    }

    /// `read(value)` — the current value, untracked.
    #[must_use]
    pub const fn read_value(&self) -> &JsFunction {
        &self.read
    }

    /// `write(target, value)` — pushes a value into a writable reactive value.
    #[must_use]
    pub const fn write(&self) -> &JsFunction {
        &self.write
    }

    /// `subscribe(source, callback)` — the push half of the mapping; returns
    /// the dispose function the subscribing cell owns.
    #[must_use]
    pub const fn subscribe(&self) -> &JsFunction {
        &self.subscribe
    }

    /// `toSignal(source)` — materializes any reactive input as a JS signal.
    #[must_use]
    pub const fn to_signal(&self) -> &JsFunction {
        &self.to_signal
    }

    /// `toAccessor(value)` — lifts any reactive input to a plain thunk.
    #[must_use]
    pub const fn to_accessor(&self) -> &JsFunction {
        &self.to_accessor
    }

    /// `createSignal(value)` — the JS signal a `Binding<T>` is exported as.
    #[must_use]
    pub const fn create_signal(&self) -> &JsFunction {
        &self.create_signal
    }

    /// `createMemo(compute)` — the read-only accessor a `Computed<T>` is
    /// exported as.
    #[must_use]
    pub const fn create_memo(&self) -> &JsFunction {
        &self.create_memo
    }

    /// `makeCallback(id)` — the JS function wrapping a registered Rust
    /// closure.
    #[must_use]
    pub const fn make_callback(&self) -> &JsFunction {
        &self.make_callback
    }

    /// The module the bundle carries under `id`.
    ///
    /// # Errors
    ///
    /// [`TsError::UnknownModule`] when the bundle carries no such module —
    /// which a verified bundle cannot, because its manifest is checked against
    /// the modules the binary mounts.
    pub fn module(&self, id: &str) -> Result<&JsFunction, TsError> {
        self.modules.get(id).ok_or_else(|| TsError::UnknownModule {
            id: Str::from(id.to_owned()),
        })
    }

    /// Every module id the bundle carries, in sorted order.
    pub fn module_ids(&self) -> impl Iterator<Item = &Str> {
        self.modules.keys()
    }
}

/// Reads one entry and holds it to being a function.
fn function<R: JsRuntime>(engine: &R, entry: &Entry) -> Result<JsFunction, TsError> {
    match engine.eval(entry.source, SOURCE_NAME)? {
        JsValue::Function(function) => Ok(function),
        other => Err(TsError::RuntimeEntry {
            name: entry.name,
            found: kind_of(&other),
        }),
    }
}

/// Reads the module table: an object of module id to module function.
fn modules<R: JsRuntime>(engine: &R) -> Result<BTreeMap<Str, JsFunction>, TsError> {
    let value = engine.eval(MODULES.source, SOURCE_NAME)?;
    let JsValue::Object(entries) = value else {
        return Err(TsError::ModuleTable {
            found: kind_of(&value),
        });
    };
    entries
        .into_iter()
        .map(|(id, value)| match value {
            JsValue::Function(function) => Ok((Str::from(id), function)),
            other => Err(TsError::Module {
                id: Str::from(id),
                found: kind_of(&other),
            }),
        })
        .collect()
}

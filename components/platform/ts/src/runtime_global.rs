//! `globalThis.__waterui_runtime`: what a bundle hands the bridge.
//!
//! A bundle is a classic script, so nothing it declares is reachable from
//! Rust. Its entry therefore ends with
//! `installRuntimeGlobal(modules, contracts)`, which publishes the reactive
//! helpers, the host installer, `mount`, the module table and the props
//! contract each module was built against on one global object.
//! [`RuntimeGlobal::read`] picks that object apart right after `eval` and
//! refuses a bundle that is missing an entry, naming it.
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
const CONTRACTS: Entry = Entry {
    name: "contracts",
    source: "globalThis.__waterui_runtime.contracts",
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
    contracts: BTreeMap<Str, u64>,
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
            contracts: contracts(engine)?,
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
    /// the modules the binary mounts. The error names the ids the bundle does
    /// carry, because the usual cause is a bundle from another build and the
    /// list is what says so.
    pub fn module(&self, id: &str) -> Result<&JsFunction, TsError> {
        self.modules.get(id).ok_or_else(|| TsError::UnknownModule {
            id: Str::from(id.to_owned()),
            available: self.available(),
        })
    }

    /// Every module id the bundle carries, in sorted order.
    pub fn module_ids(&self) -> impl Iterator<Item = &Str> {
        self.modules.keys()
    }

    /// The props contract hash the bundle declares for module `id`.
    ///
    /// # Errors
    ///
    /// [`TsError::MissingContract`] when the bundle declares none. Mounting a
    /// module whose contract the bundle does not state is refused rather than
    /// taken on trust: the check exists because a bundle and a binary can come
    /// from different builds, and a missing hash is exactly that case.
    pub fn contract(&self, id: &str) -> Result<u64, TsError> {
        self.contracts
            .get(id)
            .copied()
            .ok_or_else(|| TsError::MissingContract {
                id: Str::from(id.to_owned()),
            })
    }

    /// The module ids the bundle carries, as one line for an error message.
    fn available(&self) -> Str {
        if self.modules.is_empty() {
            return Str::from("no modules at all");
        }
        let mut listed = String::new();
        for id in self.modules.keys() {
            if !listed.is_empty() {
                listed.push_str(", ");
            }
            listed.push('"');
            listed.push_str(id);
            listed.push('"');
        }
        Str::from(listed)
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

/// Reads the contract table: an object of module id to the hexadecimal props
/// contract hash the module was built against.
///
/// The hash crosses as text, not as a number: it is 64 bits, and a JavaScript
/// number holds only 53 of them exactly, so a numeric entry would compare
/// equal to a contract it is not.
fn contracts<R: JsRuntime>(engine: &R) -> Result<BTreeMap<Str, u64>, TsError> {
    let value = engine.eval(CONTRACTS.source, SOURCE_NAME)?;
    let JsValue::Object(entries) = value else {
        return Err(TsError::ContractTable {
            found: kind_of(&value),
        });
    };
    entries
        .into_iter()
        .map(|(id, value)| {
            let JsValue::String(text) = value else {
                return Err(TsError::ContractHash {
                    id: Str::from(id),
                    found: Str::from(kind_of(&value)),
                });
            };
            let hash = u64::from_str_radix(text.strip_prefix("0x").unwrap_or(&text), 16).map_err(
                |_| TsError::ContractHash {
                    id: Str::from(id.clone()),
                    found: Str::from(text),
                },
            )?;
            Ok((Str::from(id), hash))
        })
        .collect()
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

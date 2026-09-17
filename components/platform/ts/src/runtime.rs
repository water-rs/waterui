//! The runtime an application holds: one engine, one bundle, one host.

use std::rc::Rc;

use waterui_core::Environment;
use waterui_ts_engine::{JsFunction, JsRuntime};

use crate::Engine;
use crate::bridge::Bridge;
use crate::error::TsError;
use crate::host::{self, HostFunctions, HostTable};
use crate::runtime_global::RuntimeGlobal;

/// The script name a bundle is attributed to in a stack trace.
const BUNDLE_NAME: &str = "bundle.js";

/// Clears whatever is published at the runtime global.
const CLEAR_RUNTIME_GLOBAL: &str = "globalThis.__waterui_runtime = undefined;";

/// The script name the clearing is attributed to in a stack trace.
const CLEAR_NAME: &str = "waterui:load";

/// A JavaScript engine with `WaterUI`'s runtime loaded into it.
///
/// Constructing one creates the engine and registers the host functions;
/// [`load`](Self::load) evaluates the bundle, reads the runtime it published
/// and installs the host table. Mounting a module — turning
/// [`module`](Self::module) into a view — is the `tsx!` leaf
/// (water-rs/waterui#1044).
///
/// The runtime is pinned to the thread that created it, like the engine
/// inside it, and is deliberately neither `Send` nor `Sync`.
pub struct TsRuntime {
    bridge: Bridge,
    table: Rc<dyn HostTable>,
    /// The host functions as the engine installed them, captured before any
    /// bundle could reassign a property of `__waterui_host`.
    host: HostFunctions,
}

impl core::fmt::Debug for TsRuntime {
    /// The host table is application code with no useful representation, so
    /// the bridge is what a runtime shows.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TsRuntime")
            .field("bridge", &self.bridge)
            .finish_non_exhaustive()
    }
}

impl TsRuntime {
    /// Creates a runtime for a module mounted in `environment`.
    ///
    /// # Errors
    ///
    /// Returns [`TsError`] when the engine cannot be created, or when a host
    /// function cannot be registered.
    pub fn new(environment: Environment, table: impl HostTable) -> Result<Self, TsError> {
        let engine = Engine::new()?;
        let bridge = Bridge::new(engine, environment);
        let table: Rc<dyn HostTable> = Rc::new(table);
        let host = host::register(&bridge, &table)?;
        Ok(Self {
            bridge,
            table,
            host,
        })
    }

    /// The bridge every conversion and host call is handed.
    #[must_use]
    pub const fn bridge(&self) -> &Bridge {
        &self.bridge
    }

    /// Evaluates `bundle`, reads the runtime it published, and installs the
    /// host table.
    ///
    /// # Errors
    ///
    /// Returns [`TsError`] when a bundle is already loaded, when the bundle
    /// throws, when it published no runtime or an incomplete one, or when
    /// `installHost` rejects the table.
    pub fn load(&self, bundle: &str) -> Result<(), TsError> {
        // Before evaluating, not after: a second bundle refused on the way out
        // would already have run, replacing the runtime global and everything
        // else the first one installed.
        if self.bridge.runtime().is_ok() {
            return Err(TsError::BundleAlreadyLoaded);
        }
        // Only this evaluation's publication may be read. A bundle that
        // publishes a runtime and then throws leaves one behind, and without
        // this the next load would read that residue and accept a bundle that
        // published nothing at all.
        self.clear_runtime_global()?;
        let outcome = self.evaluate_and_install(bundle);
        if outcome.is_err()
            && let Err(error) = self.clear_runtime_global()
        {
            tracing::error!(%error, "clearing the runtime global after a failed load failed");
        }
        outcome
    }

    /// Evaluates `bundle`, reads the runtime it published, installs the host
    /// table, and publishes the runtime to the bridge.
    fn evaluate_and_install(&self, bundle: &str) -> Result<(), TsError> {
        self.bridge.engine().eval(bundle, BUNDLE_NAME)?;
        let runtime = RuntimeGlobal::read(self.bridge.engine())?;
        // Installed first, published second. The `OnceCell` behind the bridge
        // is the record that a bundle loaded, so publishing a runtime whose
        // host installation then failed would burn it: every retry would be
        // refused as an already-loaded bundle although nothing is usable.
        host::install(&self.bridge, &self.table, &self.host, &runtime)?;
        self.bridge.set_runtime(runtime)?;
        Ok(())
    }

    /// Clears whatever is published at the runtime global.
    fn clear_runtime_global(&self) -> Result<(), TsError> {
        self.bridge
            .engine()
            .eval(CLEAR_RUNTIME_GLOBAL, CLEAR_NAME)?;
        Ok(())
    }

    /// The runtime the loaded bundle published.
    ///
    /// # Errors
    ///
    /// Returns [`TsError::NotLoaded`] before a bundle is loaded.
    pub fn runtime(&self) -> Result<&RuntimeGlobal, TsError> {
        self.bridge.runtime()
    }

    /// The module the bundle carries under `id`.
    ///
    /// # Errors
    ///
    /// Returns [`TsError`] before a bundle is loaded, or when the bundle
    /// carries no such module.
    pub fn module(&self, id: &str) -> Result<JsFunction, TsError> {
        self.bridge.runtime()?.module(id).cloned()
    }
}

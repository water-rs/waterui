//! The runtime an application holds: one engine, one bundle, one host.

use std::rc::Rc;

use waterui_core::Environment;
use waterui_ts_engine::{JsFunction, JsRuntime};

use crate::Engine;
use crate::bridge::Bridge;
use crate::error::TsError;
use crate::host::{self, HostTable};
use crate::runtime_global::RuntimeGlobal;

/// The script name a bundle is attributed to in a stack trace.
const BUNDLE_NAME: &str = "bundle.js";

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
        host::register(&bridge, &table)?;
        Ok(Self { bridge, table })
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
    /// Returns [`TsError`] when the bundle throws, when it published no
    /// runtime or an incomplete one, when a bundle is already loaded, or when
    /// `installHost` rejects the table.
    pub fn load(&self, bundle: &str) -> Result<(), TsError> {
        self.bridge.engine().eval(bundle, BUNDLE_NAME)?;
        let runtime = RuntimeGlobal::read(self.bridge.engine())?;
        self.bridge.set_runtime(runtime)?;
        host::install(&self.bridge, &self.table)?;
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

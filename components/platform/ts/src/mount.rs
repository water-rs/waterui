//! Mounting one TypeScript module as a `WaterUI` view.
//!
//! [`Mount`] is what `tsx!` expands to. It finds the loaded runtime in the
//! environment, checks that the bundle carries the module and that the module
//! was built against the same props contract this binary mounts it with,
//! converts the props across the seam, calls the module's default export under
//! a fresh root scope, and hands back the view it built.
//!
//! # What owns the mount
//!
//! Everything the mount exported into JavaScript — a `Binding<T>` prop as a
//! signal, a `Computed<T>` as a memo, a callback prop as a registration — is
//! held on the Rust side by the mount's [`MountScope`], and the JavaScript
//! tree itself is held by the root scope `mount` opened. Both have to live
//! exactly as long as the view does, so the view carries them: the returned
//! tree is wrapped in `Metadata<Retain>`, the framework's own way of keeping a
//! value alive for a subtree's lifetime, which every renderer honours. When
//! the view goes, the guard calls the module's `dispose` and releases the
//! scope with it.

use std::rc::Rc;

use suiteki::Str;
use waterui_core::{AnyView, Environment, Metadata, Retain, View};
use waterui_ts_engine::{JsError, JsFunction, JsValue};
use waterui_ts_schema::{TsProps, TsType, struct_name};

use crate::bridge::{Bridge, ScopeOwner, WeakBridge};
use crate::convert::IntoJs;
use crate::error::{TsError, kind_of};
use crate::runtime::TsRuntime;

/// The props contract of a module that takes none.
///
/// A module with no props is not a module with no contract: it is typed
/// against the empty object, whose hash the bundle declares like any other, so
/// the check that a bundle and a binary come from the same build holds for
/// every module rather than for most of them. `tsx!("./promo.tsx")` mounts
/// against this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, TsProps)]
pub struct NoProps {}

/// The loaded TypeScript runtime, as a mounted view reaches it.
///
/// [`TsRuntime`] owns a JavaScript engine and is pinned to the thread that
/// created it, so it is neither `Send` nor `Sync`. `Environment` stores values
/// behind `Rc` and requires nothing of them but `'static`, which is exactly
/// what a thread-pinned runtime can promise: the application's bundle loader
/// creates the runtime at launch and installs one of these, a test or preview
/// host installs the one [`configured_runtime`](crate::configured_runtime)
/// loads from the bundle the `water` CLI names, and every [`Mount`] below it
/// finds it there.
///
/// The handle is cheap to clone: every clone is the same runtime.
#[derive(Debug, Clone)]
pub struct RuntimeHandle(Rc<TsRuntime>);

impl RuntimeHandle {
    /// Takes ownership of a loaded runtime.
    #[must_use]
    pub fn new(runtime: TsRuntime) -> Self {
        Self(Rc::new(runtime))
    }

    /// The runtime itself.
    #[must_use]
    pub fn runtime(&self) -> &TsRuntime {
        &self.0
    }

    /// The environment with this runtime installed in it.
    ///
    /// This is what the bundle loader hands the application root, and what a
    /// test hands the view it mounts. It overlays rather than copies, so it
    /// costs one allocation however large the environment is.
    #[must_use]
    pub fn install(self, environment: &Environment) -> Environment {
        environment.extending(self)
    }

    /// The runtime installed in `environment`.
    ///
    /// # Errors
    ///
    /// Returns [`TsError::NoRuntimeInstalled`], naming `module`, when no
    /// runtime is installed — an application that mounts a TypeScript module
    /// without loading a bundle first.
    pub fn installed_in<'a>(
        environment: &'a Environment,
        module: &str,
    ) -> Result<&'a Self, TsError> {
        environment
            .get::<Self>()
            .ok_or_else(|| TsError::NoRuntimeInstalled {
                id: Str::from(module.to_owned()),
            })
    }
}

/// Converts one mount's props across the seam.
///
/// The props type is erased here rather than carried in [`Mount`]'s own
/// signature: a mount is one view type whatever it mounts, so a tree holding
/// twenty of them holds one type.
type Props = Box<dyn FnOnce(&Bridge) -> Result<JsValue, JsError>>;

/// One mounted TypeScript view module.
///
/// Written as `tsx!("./promo.tsx", PromoProps { … })`, which resolves the
/// module id and records the mount for the `water` CLI. Constructing one
/// directly is the general form, for a caller that already knows the id.
#[must_use]
pub struct Mount {
    /// The module id the bundle publishes the module under.
    module: &'static str,
    /// The name of the props type, for the message a contract mismatch
    /// carries.
    props_name: &'static str,
    /// The contract hash the binary's props type has.
    contract: u64,
    /// The props, still on the Rust side.
    props: Props,
}

impl core::fmt::Debug for Mount {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mount")
            .field("module", &self.module)
            .field("props", &self.props_name)
            .finish_non_exhaustive()
    }
}

impl Mount {
    /// Mounts `module` with `props`.
    ///
    /// The props type is the contract the module is built against: its
    /// [`TsProps::CONTRACT_HASH`] is checked against the hash the bundle
    /// declares for that module, so a bundle from another build is refused
    /// rather than handed an object of a shape it was not compiled for.
    ///
    /// A type that does not derive `TsProps` cannot be mounted:
    ///
    /// ```compile_fail
    /// use waterui_ts::Mount;
    ///
    /// struct Bare {
    ///     headline: String,
    /// }
    ///
    /// let _ = Mount::new::<Bare>(
    ///     "src/promo.tsx",
    ///     Bare {
    ///         headline: String::from("Welcome"),
    ///     },
    /// );
    /// ```
    pub fn new<P: TsProps + IntoJs + 'static>(module: &'static str, props: P) -> Self {
        Self {
            module,
            props_name: struct_name(&<P as TsType>::SCHEMA),
            contract: <P as TsProps>::CONTRACT_HASH,
            props: Box::new(move |bridge| props.into_js(bridge)),
        }
    }

    /// The module id this mount names.
    #[must_use]
    pub const fn module(&self) -> &'static str {
        self.module
    }

    /// Mounts the module, or says why it could not be mounted.
    ///
    /// This is the fallible form of [`View::body`], which has no error channel
    /// of its own. Every failure names the module: no runtime installed, no
    /// bundle loaded, a bundle that does not carry the module, a bundle whose
    /// module was built against another props contract, props that could not
    /// cross, or a module that threw while it built its tree.
    ///
    /// # Errors
    ///
    /// Returns [`TsError`] for any of the above. There is no partial mount: a
    /// failure leaves the runtime exactly as it was.
    pub fn try_mount(self, environment: &Environment) -> Result<AnyView, TsError> {
        let id = self.module;
        let handle = RuntimeHandle::installed_in(environment, id)?;
        let runtime = handle.runtime();
        let global = runtime.runtime()?;
        let module = global.module(id)?.clone();

        let declared = global.contract(id)?;
        if declared != self.contract {
            return Err(TsError::ContractMismatch {
                id: Str::from(id.to_owned()),
                props: self.props_name,
                expected: self.contract,
                declared,
            });
        }

        let bridge = runtime.bridge();
        // Opened before the props convert: converting is what exports them,
        // and an export belongs to the mount that asked for it.
        let scope = bridge.open_scope();
        let props = (self.props)(bridge)?;

        let mounted = call_mount(bridge, &module, props)?;
        let (view, dispose) = take_mounted(bridge, id, &mounted)?;

        // The scope is closed rather than dropped: what it exported outlives
        // this call, for as long as the view does.
        let guard = MountGuard {
            bridge: bridge.downgrade(),
            dispose,
            _scope: scope.close(),
        };
        Ok(AnyView::new(Metadata::new(view, Retain::new(guard))))
    }
}

/// `mount(() => module(props))`: the module's default export, called under a
/// fresh root scope.
///
/// The thunk is a Rust closure, because `mount` takes a render function and
/// the bridge's registry is the only way a Rust closure becomes a JavaScript
/// one. `mount` calls it synchronously, so the registration lives exactly as
/// long as this call does.
fn call_mount(bridge: &Bridge, module: &JsFunction, props: JsValue) -> Result<JsValue, TsError> {
    let global = bridge.runtime()?;
    // Weak, because the closure lives in the registry that lives in the
    // bridge: a strong handle here would be a cycle that never releases the
    // runtime.
    let weak = bridge.downgrade();
    let module = module.clone();
    let registration = bridge.register_callback(move |_| {
        let bridge = weak.upgrade().ok_or_else(|| {
            JsError::new(
                "Error",
                "a TypeScript module was rendered after its runtime was dropped",
            )
        })?;
        bridge.call(&module, std::slice::from_ref(&props))
    })?;
    let thunk = bridge.make_callback(registration.id())?;
    Ok(bridge.call(global.mount(), &[thunk])?)
}

/// The `{ handle, dispose }` a mounted tree is.
fn take_mounted(
    bridge: &Bridge,
    id: &str,
    mounted: &JsValue,
) -> Result<(AnyView, JsFunction), TsError> {
    let entries = mounted.as_object().ok_or_else(|| TsError::NotMounted {
        id: Str::from(id.to_owned()),
        found: kind_of(mounted),
    })?;
    let entry = |name: &'static str| {
        entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
            .ok_or_else(|| TsError::MountedEntry {
                id: Str::from(id.to_owned()),
                name,
                found: "nothing",
            })
    };
    let view = bridge.take_view(entry("handle")?)?;
    let dispose = entry("dispose")?;
    let JsValue::Function(dispose) = dispose else {
        return Err(TsError::MountedEntry {
            id: Str::from(id.to_owned()),
            name: "dispose",
            found: kind_of(dispose),
        });
    };
    Ok((view, dispose.clone()))
}

impl View for Mount {
    /// # Panics
    ///
    /// Panics when the module cannot be mounted, with the message
    /// [`try_mount`](Self::try_mount) would have returned. `View::body` has no
    /// error channel, and every one of those failures means the bundle and the
    /// binary disagree — a build-time mistake that a blank rectangle would
    /// hide at the one moment it is still diagnosable. A caller that wants to
    /// decide for itself calls `try_mount`.
    fn body(self, environment: &Environment) -> impl View {
        let module = self.module;
        self.try_mount(environment).unwrap_or_else(|error| {
            panic!("mounting the TypeScript module \"{module}\" failed: {error}")
        })
    }
}

/// What keeps one mount alive, and tears it down when its view goes.
///
/// Dropping it disposes the JavaScript tree — branch disposals,
/// subscriptions, cleanups — and then releases everything the mount exported
/// into JavaScript. In that order: disposal runs JavaScript that may still
/// reach a value the scope owns.
struct MountGuard {
    bridge: WeakBridge,
    dispose: JsFunction,
    _scope: ScopeOwner,
}

impl core::fmt::Debug for MountGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MountGuard").finish_non_exhaustive()
    }
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        let Some(bridge) = self.bridge.upgrade() else {
            // The runtime is gone, and with it the tree this would dispose.
            return;
        };
        if let Err(error) = bridge.call(&self.dispose, &[]) {
            tracing::error!(%error, "disposing a mounted TypeScript module failed");
        }
    }
}

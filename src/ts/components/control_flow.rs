//! `<Show>` and `<Suspense>`: one branch presented at a time.
//!
//! A branch is a JavaScript render callback's `{ handle, dispose }`: the view
//! it built, and the teardown for the reactive scope it built it under. The
//! host owns *when* a branch is presented and disposes it exactly once when it
//! stops being, which [`Branch`]'s own `Drop` guarantees — a branch that is
//! replaced is disposed by the assignment that replaced it.
//!
//! [`Branch::render`] is also where a render callback's *exports* get an
//! owner. A callback reaches back into the host while it builds — a nested
//! `<Show>` exports its `when` accessor, a control registers a callback, a
//! `<For>` materializes its list — and every one of those lands in whatever
//! mount scope is open. The scope open around a render callback is therefore
//! the branch's own: opened before the call, closed into a [`ScopeOwner`] the
//! branch holds, released when the branch is replaced or dropped. Without it a
//! `<Show>` inside a churning row would leave one cell, one JavaScript memo
//! and one dedup entry in the mount's scope per departed row, for the life of
//! the mount.

use core::cell::RefCell;
use std::rc::Rc;

use nami::{Computed, Signal, SignalExt as _};
use waterui_core::{AnyView, Dynamic, Metadata, Retain};
use waterui_ts::engine::{JsError, JsFunction, JsValue};

use waterui_ts::ViewSlot;
use waterui_ts::{Bridge, ScopeOwner, WeakBridge};

/// One realized branch: the view it produced, its teardown, and what it
/// exported while building.
pub struct Branch {
    dispose: Option<JsFunction>,
    bridge: WeakBridge,
    /// Everything the render callback exported into JavaScript while it ran.
    ///
    /// The branch owns it rather than the mount, because a branch is exactly
    /// as long-lived as the subtree it built: replacing it releases the
    /// subtree's signals with the subtree.
    #[expect(
        dead_code,
        reason = "the scope is held, not read: what the callback exported lives exactly as long \
                  as the branch it built"
    )]
    exports: ScopeOwner,
}

impl core::fmt::Debug for Branch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Branch").finish_non_exhaustive()
    }
}

impl Branch {
    /// Calls a render callback and takes the view out of the branch it
    /// returned.
    ///
    /// The callback runs under a scope of the branch's own, so whatever it
    /// exports while building belongs to the subtree it builds.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the callback throws, or when what it returned
    /// is not the `{ handle, dispose }` the contract requires.
    pub fn render(
        bridge: &Bridge,
        render: &JsFunction,
        arguments: &[JsValue],
    ) -> Result<(AnyView, Self), JsError> {
        let scope = bridge.open_scope();
        let branch = bridge.call(render, arguments)?;
        let exports = scope.close();
        let entries = branch.as_object().ok_or_else(|| {
            JsError::conversion(format!(
                "a control-flow render callback returned {}, not a {{ handle, dispose }} branch",
                waterui_ts::kind_of(&branch)
            ))
        })?;
        let entry = |name: &str| {
            entries
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value)
        };
        let handle = entry("handle").ok_or_else(|| {
            JsError::conversion("a branch carries the view it built under `handle`")
        })?;
        let view = ViewSlot::from_js_value(handle)?.take()?;
        let dispose = match entry("dispose") {
            Some(JsValue::Function(dispose)) => Some(dispose.clone()),
            None | Some(JsValue::Undefined | JsValue::Null) => None,
            Some(other) => {
                return Err(JsError::conversion(format!(
                    "a branch's `dispose` is {}, not the teardown the contract requires",
                    waterui_ts::kind_of(other)
                )));
            }
        };
        Ok((
            view,
            Self {
                dispose,
                bridge: bridge.downgrade(),
                exports,
            },
        ))
    }
}

impl Drop for Branch {
    /// Tears the branch's reactive scope down, once.
    fn drop(&mut self) {
        let (Some(dispose), Some(bridge)) = (self.dispose.take(), self.bridge.upgrade()) else {
            return;
        };
        if let Err(error) = bridge.call(&dispose, &[]) {
            tracing::error!(%error, "disposing a control-flow branch threw");
        }
    }
}

/// `<Show>`: presents the children while `when` reads truthy, the fallback
/// otherwise.
///
/// The branch is built once per activation, not once per change: `when` is
/// read as a truth value through a distinct signal, so a value that changes
/// while staying on the same side updates the accessor the branch was handed
/// and leaves the branch itself — and everything it owns — alone.
///
/// # Errors
///
/// Returns [`JsError`] when `when` cannot be materialized, when the accessor
/// cannot be exported, or when the first branch cannot be rendered.
pub fn show(
    bridge: &Bridge,
    when: &JsValue,
    render: &JsFunction,
    fallback: Option<&JsFunction>,
) -> Result<AnyView, JsError> {
    let source: Computed<JsValue> = bridge.materialize_computed(when)?;
    let item = bridge.export_computed(&source)?;
    let presented: Computed<bool> = source.map(|value| truthy(&value)).distinct().computed();

    let current: Rc<RefCell<Option<Branch>>> = Rc::new(RefCell::new(None));
    let build = {
        let bridge = bridge.clone();
        let render = render.clone();
        let fallback = fallback.cloned();
        let current = Rc::clone(&current);
        move |presented: bool| -> Result<AnyView, JsError> {
            let rendered = if presented {
                Some(Branch::render(
                    &bridge,
                    &render,
                    core::slice::from_ref(&item),
                )?)
            } else {
                fallback
                    .as_ref()
                    .map(|fallback| Branch::render(&bridge, fallback, &[]))
                    .transpose()?
            };
            let Some((view, branch)) = rendered else {
                // Nothing to present on this side: the outgoing branch is
                // disposed by the assignment, and the slot renders nothing.
                *current.borrow_mut() = None;
                return Ok(AnyView::new(()));
            };
            *current.borrow_mut() = Some(branch);
            Ok(view)
        }
    };

    let (handler, dynamic) = Dynamic::new();
    handler.set(build(presented.get())?);
    let guard = presented.watch(move |context| {
        let view = build(context.into_value())
            .unwrap_or_else(|error| panic!("a <Show> branch could not be rendered: {error}"));
        handler.set(view);
    });
    // `presented` is retained beside its guard, and `source` beside it: a
    // watch registration keeps neither alive, and `source` owns the
    // JavaScript subscription that drives both.
    Ok(AnyView::new(Metadata::new(
        dynamic,
        Retain::new((guard, current, presented, source)),
    )))
}

/// `<Suspense>`: presents the children.
///
/// The boundary is where a pending resource would be awaited, and the
/// JavaScript runtime has no resource primitive: nothing under a `<Suspense>`
/// can be pending, so the children branch — which renders synchronously — is
/// what is presented, and the fallback is never built. A Rust-composed view
/// slotted into the tree suspends through its own `Suspense`, which the
/// backend presents as it always has.
///
/// # Errors
///
/// Returns [`JsError`] when the children branch cannot be rendered.
pub fn suspense(
    bridge: &Bridge,
    children: &JsFunction,
    _fallback: Option<&JsFunction>,
) -> Result<AnyView, JsError> {
    let (view, branch) = Branch::render(bridge, children, &[])?;
    Ok(AnyView::new(Metadata::new(view, Retain::new(branch))))
}

/// JavaScript truthiness, which is what `<Show when={…}>` reads.
fn truthy(value: &JsValue) -> bool {
    match value {
        JsValue::Undefined | JsValue::Null => false,
        JsValue::Bool(value) => *value,
        JsValue::Number(value) => *value != 0.0 && !value.is_nan(),
        JsValue::BigInt(_) => value.as_i64() != Some(0),
        JsValue::String(value) => !value.is_empty(),
        JsValue::Array(_)
        | JsValue::Object(_)
        | JsValue::Function(_)
        | JsValue::ObjectRef(_)
        | JsValue::Opaque(_) => true,
    }
}

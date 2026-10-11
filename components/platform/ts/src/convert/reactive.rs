//! Reactive values and views: the types that need the runtime itself.

use nami::{Binding, Computed};
use waterui_core::AnyView;
use waterui_ts_engine::{JsError, JsValue};

use super::{FromJs, IntoJs};
use crate::bridge::Bridge;
use crate::view::ViewSlot;

impl<T: FromJs + IntoJs + Clone + 'static> IntoJs for Binding<T> {
    /// A `Binding<T>` is a `Signal<T>` on the TypeScript side: a real
    /// JavaScript signal, seeded from the binding and bound to it in both
    /// directions.
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        bridge.export_binding(&self)
    }
}

impl<T: FromJs + IntoJs + Clone + 'static> FromJs for Binding<T> {
    /// The mirror image: a JavaScript signal materialized as a two-way
    /// `Binding<T>`. A read-only input is an error, not a binding whose writes
    /// would vanish.
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        bridge.materialize_binding(value)
    }
}

impl<T: IntoJs + Clone + 'static> IntoJs for Computed<T> {
    /// A `Computed<T>` is an `Accessor<T>`: a memo over a signal the bridge
    /// pushes into, so JavaScript tracks it and cannot write to it.
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        bridge.export_computed(&self)
    }
}

impl<T: FromJs + IntoJs + Clone + 'static> FromJs for Computed<T> {
    /// Any reactive input becomes a read-only `Computed<T>`; a constant
    /// becomes a constant one, creating no subscription.
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        bridge.materialize_computed(value)
    }
}

impl IntoJs for AnyView {
    /// A Rust-composed subtree crosses as an opaque slot JSX can place as a
    /// child. `AnyView` is not `Clone`, so the slot is taken exactly once.
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(ViewSlot::new(self).to_js_value())
    }
}

impl FromJs for AnyView {
    /// Takes the view out of the slot. A second read is an error: the view is
    /// already somewhere in the tree.
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        ViewSlot::from_js_value(value)?.take()
    }
}

impl waterui_ts_schema::TsType for crate::view::JsViewBuilder {
    const SCHEMA: waterui_ts_schema::TypeSchema = waterui_ts_schema::TypeSchema::ViewBuilder;
}

impl FromJs for crate::view::JsViewBuilder {
    /// A render function, kept as one. Unlike a view handle it is not
    /// consumed: a destination is built again every time it is entered.
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::Function(function) => Ok(Self::new(function.clone(), bridge)),
            other => Err(crate::convert::expected(
                "a function returning a view, which is what a destination is",
                other,
            )),
        }
    }
}

impl IntoJs for crate::view::JsViewBuilder {
    /// The function itself: a builder that came from JavaScript goes back as
    /// what it was.
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Function(self.function()))
    }
}

impl IntoJs for ViewSlot {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(self.to_js_value())
    }
}

impl FromJs for ViewSlot {
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        Self::from_js_value(value)
    }
}

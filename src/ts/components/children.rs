//! What a JSX element's children become.
//!
//! The runtime normalizes children before the host sees them: `null`,
//! `undefined` and booleans are gone, nested arrays are flattened, and what
//! arrives is a list of handles, strings, numbers and accessors. Which slot
//! that list fills is the component's own — the catalog's
//! [`ChildrenSlot`](waterui_ts::schema::ChildrenSlot) — and this module turns
//! the list into the Rust value that slot takes: content views, a label, or
//! text.

use nami::{Computed, Signal, SignalExt as _};
use suiteki::Str;
use waterui_controls::label::{IntoLabel as _, Label};
use waterui_core::{AnyView, Dynamic, Metadata, Retain};
use waterui_text::Text;
use waterui_ts::engine::{JsError, JsValue};

use crate::ts::catalog::TextContent;
use waterui_ts::Bridge;
use waterui_ts::FromJs as _;
use waterui_ts::ViewSlot;

/// The content views of an element whose children are views.
///
/// A string or a number among them is materialized as text, exactly as
/// `text(…)` would, so `<VStack>Total{total()}</VStack>` is a stack of text
/// views rather than an error.
pub fn content(bridge: &Bridge, children: &[JsValue]) -> Result<Vec<AnyView>, JsError> {
    children
        .iter()
        .map(|child| view_of(bridge, child))
        .collect()
}

/// One child as a view.
fn view_of(bridge: &Bridge, child: &JsValue) -> Result<AnyView, JsError> {
    match child {
        JsValue::Opaque(_) => ViewSlot::from_js_value(child)?.take(),
        JsValue::String(_) | JsValue::Number(_) | JsValue::BigInt(_) => {
            let content = TextContent::from_js(child, bridge)?;
            Ok(AnyView::new(Text::new(content)))
        }
        JsValue::Function(_) | JsValue::Object(_) => reactive(bridge, child),
        other => Err(JsError::conversion(format!(
            "a child is {}, which is not a view, text or a reactive child slot",
            waterui_ts::kind_of(other)
        ))),
    }
}

/// A reactive child slot: the host swaps the child when the accessor produces
/// a new element.
///
/// One element, not a list: a child position that changes membership is a
/// collection — `<For each={…}>` — which reconciles by identity instead of
/// replacing the subtree, and a list arriving here says the author meant that.
fn reactive(bridge: &Bridge, child: &JsValue) -> Result<AnyView, JsError> {
    let source: Computed<JsValue> = bridge.materialize_computed(child)?;
    let (handler, dynamic) = Dynamic::new();
    let build = {
        let bridge = bridge.clone();
        move |value: &JsValue| -> Result<AnyView, JsError> {
            match value {
                JsValue::Undefined | JsValue::Null => Ok(AnyView::new(())),
                JsValue::Array(_) => Err(JsError::conversion(
                    "a reactive child produced a list. A child position whose membership \
                     changes is a collection: render it with <For each={…}>, which reconciles \
                     by identity instead of replacing the subtree",
                )),
                value => view_of(&bridge, value),
            }
        }
    };
    handler.set(build(&source.get())?);
    let guard = source.watch(move |context| {
        let view = build(&context.into_value()).unwrap_or_else(|error| {
            panic!("a reactive child of a TypeScript view could not be realized: {error}")
        });
        handler.set(view);
    });
    // The signal is retained beside its guard, not only the guard: a watch
    // registration does not keep what it watches alive, and this signal owns
    // the JavaScript subscription that feeds it — dropping it here would
    // unsubscribe and the child would never move again.
    Ok(AnyView::new(Metadata::new(
        dynamic,
        Retain::new((guard, source)),
    )))
}

/// The label of a control, which every control requires at construction.
///
/// `label` is the explicit form and wins when both are given, exactly as
/// HOST.md says; otherwise the children are the label. Text children become a
/// text label, and an explicit `<Label>` element is used as it was built.
pub fn label(
    bridge: &Bridge,
    component: &str,
    explicit: Option<AnyView>,
    children: &[JsValue],
) -> Result<Label, JsError> {
    if let Some(view) = explicit {
        return view
            .downcast::<Label>()
            .map(|label| *label)
            .map_err(|view| {
                JsError::conversion(format!(
                    "<{component} label={{…}}> takes a <Label>, found {}",
                    view.name()
                ))
            });
    }
    let Some(content) = text(bridge, children)? else {
        return Err(JsError::conversion(format!(
            "<{component}> has no label. Every control names itself for assistive technology, \
             so give it children — <{component}>Save</{component}> — or a label attribute"
        )));
    };
    Ok(Text::new(content).into_label())
}

/// The label of a control that may go without one.
///
/// `<Progress>` is the case: a bar filling a row inside a labelled section
/// names itself through that section, and Rust's `Progress::new` takes the
/// label separately for the same reason.
///
/// # Errors
///
/// Returns [`JsError`] when a child cannot be read as text.
pub fn label_or_none(bridge: &Bridge, children: &[JsValue]) -> Result<Option<Label>, JsError> {
    Ok(text(bridge, children)?.map(|content| Text::new(content).into_label()))
}

/// The children of an element whose children are each their own text.
///
/// A `<Column>`'s children are its rows: one text view per child, not one
/// value assembled from all of them, which is what [`text`] does.
///
/// # Errors
///
/// Returns [`JsError`] when a child cannot be read as text.
pub fn texts(bridge: &Bridge, children: &[JsValue]) -> Result<Vec<Text>, JsError> {
    children
        .iter()
        .map(|child| {
            let content: Computed<TextContent> = bridge.materialize_computed(child)?;
            Ok(Text::new(content))
        })
        .collect()
}

/// The text of an element whose children are text, lifted into one value.
///
/// A single string child is a translation key, like Rust's `text("…")`. Text
/// assembled from several parts is the assembled value, like `text!("…{}")`:
/// what is looked up is what an author wrote as one piece.
pub fn text(
    bridge: &Bridge,
    children: &[JsValue],
) -> Result<Option<Computed<TextContent>>, JsError> {
    let mut parts = children.iter();
    let Some(first) = parts.next() else {
        return Ok(None);
    };
    let first: Computed<TextContent> = bridge.materialize_computed(first)?;
    let Some(second) = parts.next() else {
        return Ok(Some(first));
    };

    let mut joined = strings(&first);
    for part in core::iter::once(second).chain(parts) {
        let part: Computed<TextContent> = bridge.materialize_computed(part)?;
        joined = joined
            .zip(&strings(&part))
            .map(|(left, right)| Str::from(format!("{left}{right}")))
            .computed();
    }
    Ok(Some(joined.map(TextContent::Verbatim).computed()))
}

/// The text one part carries, before any lookup.
fn strings(content: &Computed<TextContent>) -> Computed<Str> {
    content.map(|content| content.as_str().clone()).computed()
}

/// Refuses children for a component that takes none.
pub fn none(component: &str, children: &[JsValue]) -> Result<(), JsError> {
    if children.is_empty() {
        return Ok(());
    }
    Err(JsError::conversion(format!(
        "<{component}> takes no children, and was given {}",
        children.len()
    )))
}

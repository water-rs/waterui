//! What a JSX element's children become.
//!
//! The runtime normalizes children before the host sees them: `null`,
//! `undefined` and booleans are gone, nested arrays are flattened, and what
//! arrives is a list of handles, strings, numbers and accessors. Which slot
//! that list fills is the component's own — the catalog's
//! [`ChildrenSlot`](waterui_ts::schema::ChildrenSlot) — and this module turns
//! the list into the Rust value that slot takes: content views, a label, or
//! text.

use core::iter::once;

use nami::{Computed, Signal, SignalExt as _};
use suiteki::Str;
use waterui_controls::label::{ErasedLabel, IntoLabel, Label, TitleLabel};
use waterui_core::{AnyView, Dynamic, Metadata, Retain};
use waterui_text::StyledStr;
use waterui_text::text::{IntoText, Text, TextConfig};
use waterui_ts::engine::{JsError, JsValue};

use crate::ts::catalog::{ResolvedTextPart, TextContent};
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
        JsValue::Object(_) if TextContent::is_localized_object(child) => {
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
    handler.set(build(&source.snapshot())?);
    let guard = source.watch(move |context| {
        match build(&context.into_value()) {
            Ok(view) => handler.set(view),
            Err(error) => crate::contract_violation!(
                "a reactive child of a TypeScript view could not be realized, so it keeps its last view: {error}"
            ),
        }
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
) -> Result<Label<ErasedLabel>, JsError> {
    if let Some(view) = explicit {
        return view
            .downcast::<Label<ErasedLabel>>()
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
    Ok(content.into_label().erase())
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
pub fn label_or_none(
    bridge: &Bridge,
    children: &[JsValue],
) -> Result<Option<Label<TitleLabel>>, JsError> {
    Ok(text(bridge, children)?.map(IntoLabel::into_label))
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
            Ok(Text::new(TextContent::flatten_computed(&content)))
        })
        .collect()
}

/// Several resolved text parts joined into one `Text`, preserving per-part
/// catalog localization and interpolation alongside verbatim runtime strings.
#[derive(Debug, Clone)]
struct JoinedText(Vec<ResolvedTextPart>);

impl IntoText for JoinedText {
    fn into_text(self) -> Text {
        let parts = self.0;
        Text::localized_with(move |env, locale| {
            let mut combined = String::new();
            for part in &parts {
                combined.push_str(part.render(env, locale).as_str());
            }
            TextConfig::new(StyledStr::plain(Str::from(combined)))
        })
    }
}

/// The text of an element whose children are text, lifted into one [`Text`].
///
/// A single `{ key }` / `{ key, args }` child (as the JSX transform emits for
/// literal or interpolated text, or `t` constructs) is resolved through the
/// translation catalog with its named arguments; runtime strings and numbers
/// stay verbatim. Multiple children each keep their own verbatim or localized
/// semantics and concatenate after resolution.
pub fn text(bridge: &Bridge, children: &[JsValue]) -> Result<Option<Text>, JsError> {
    let mut parts = children.iter();
    let Some(first) = parts.next() else {
        return Ok(None);
    };
    let first: Computed<TextContent> = bridge.materialize_computed(first)?;
    let first = TextContent::flatten_computed(&first);
    let Some(second) = parts.next() else {
        return Ok(Some(Text::new(first)));
    };

    let mut joined: Computed<Vec<ResolvedTextPart>> = first.map(|part| vec![part]).computed();
    for part in once(second).chain(parts) {
        let part: Computed<TextContent> = bridge.materialize_computed(part)?;
        let part = TextContent::flatten_computed(&part);
        joined = joined
            .zip(&part)
            .map(|(mut acc, next)| {
                acc.push(next);
                acc
            })
            .computed();
    }
    Ok(Some(Text::new(joined.map(JoinedText).computed())))
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

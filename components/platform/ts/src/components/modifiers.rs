//! The modifier chain: one attribute at a time, in written order.
//!
//! `modify` is handed the attributes of an element one by one, left to right,
//! and each call returns the view the next one wraps — which is exactly a Rust
//! modifier chain, and why `padding → background` and `background → padding`
//! produce different trees here as they do there.
//!
//! Every value is a reactive input, materialized inside the call: a modifier
//! whose value is a signal takes the signal, never a value read out of one.

use nami::{Computed, SignalExt as _};
use suiteki::Str;
use waterui_core::accessibility::{
    AccessibilityHidden, AccessibilityIdentifier, AccessibilityLabel, AccessibilityValue,
};
use waterui_core::env::use_env;
use waterui_core::gesture::{GestureObserver, TapGesture};
use waterui_core::interaction::Disabled;
use waterui_core::{AnyView, Environment, IgnorableMetadata, Metadata};
use waterui_layout::background::BackgroundView;
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_ts_engine::{JsError, JsValue};

use crate::bridge::Bridge;
use crate::catalog::{BackgroundValue, JsAction, PaddingValue, TextContent};
use crate::convert::FromJs as _;

/// Applies one modifier and returns the view the next one wraps.
///
/// # Errors
///
/// Returns [`JsError`] for a name the catalog does not declare — which the
/// runtime cannot produce, because it classifies attributes against the very
/// table this host published — or for a value the modifier cannot take.
pub fn apply(
    bridge: &Bridge,
    view: AnyView,
    name: &str,
    value: &JsValue,
) -> Result<AnyView, JsError> {
    match name {
        "padding" => {
            let insets: Computed<PaddingValue> = bridge.materialize_computed(value)?;
            Ok(AnyView::new(Padding::new(
                insets.map(EdgeInsets::from).computed(),
                view,
            )))
        }
        "background" => match BackgroundValue::from_js(value, bridge)? {
            BackgroundValue::Color(color) => Ok(AnyView::new(BackgroundView::new(
                view,
                crate::catalog::color_of(&color),
            ))),
            BackgroundValue::View(background) => {
                Ok(AnyView::new(BackgroundView::new(view, background)))
            }
        },
        "width" => frame(bridge, view, value, Frame::width),
        "height" => frame(bridge, view, value, Frame::height),
        "minWidth" => frame(bridge, view, value, Frame::min_width),
        "maxWidth" => frame(bridge, view, value, Frame::max_width),
        "minHeight" => frame(bridge, view, value, Frame::min_height),
        "maxHeight" => frame(bridge, view, value, Frame::max_height),
        "disabled" => {
            let disabled: Computed<bool> = bridge.materialize_computed(value)?;
            Ok(AnyView::new(use_env(move |mut env: Environment| {
                Disabled::install(&mut env, disabled);
                Metadata::new(view, env)
            })))
        }
        "onTapGesture" => {
            let action = JsAction::from_js(value, bridge)?;
            Ok(AnyView::new(Metadata::new(
                view,
                GestureObserver::new(TapGesture::new(), move || action.call()),
            )))
        }
        "a11yLabel" => {
            let label = semantic(bridge, value)?;
            Ok(AnyView::new(IgnorableMetadata::new(
                view,
                AccessibilityLabel::new(label),
            )))
        }
        "a11yValue" => {
            let text = semantic(bridge, value)?;
            Ok(AnyView::new(IgnorableMetadata::new(
                view,
                AccessibilityValue::new(text),
            )))
        }
        "a11yId" => {
            let identifier = Str::from_js(value, bridge)?;
            Ok(AnyView::new(IgnorableMetadata::new(
                view,
                AccessibilityIdentifier::new(identifier),
            )))
        }
        "a11yHidden" => {
            let hidden = bool::from_js(value, bridge)?;
            Ok(AnyView::new(IgnorableMetadata::new(
                view,
                AccessibilityHidden::new(hidden),
            )))
        }
        other => Err(JsError::conversion(format!(
            "`{other}` is not a view modifier. The catalog's modifiers are: {}",
            crate::catalog::CATALOG
                .modifiers
                .iter()
                .map(|modifier| modifier.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// One frame dimension, extending the frame the view already has.
///
/// `<Text width={120} height={44}>` is one frame with two dimensions, the same
/// view `.width(120).height(44)` builds in Rust, rather than a frame wrapped
/// in a frame.
fn frame(
    bridge: &Bridge,
    view: AnyView,
    value: &JsValue,
    set: impl FnOnce(Frame, Computed<f32>) -> Frame,
) -> Result<AnyView, JsError> {
    let length: Computed<f32> = bridge.materialize_computed(value)?;
    let frame = match view.downcast::<Frame>() {
        Ok(frame) => *frame,
        Err(view) => Frame::new(view),
    };
    Ok(AnyView::new(set(frame, length)))
}

/// Semantic text for an accessibility attribute, localized like any other.
fn semantic(bridge: &Bridge, value: &JsValue) -> Result<Computed<Str>, JsError> {
    let content: Computed<TextContent> = bridge.materialize_computed(value)?;
    Ok(content.map(|content| content.as_str().clone()).computed())
}

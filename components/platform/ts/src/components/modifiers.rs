//! The modifier chain: one attribute at a time, in written order.
//!
//! `modify` is handed the attributes of an element one by one, left to right,
//! and each call returns the view the next one wraps — which is exactly a Rust
//! modifier chain, and why `padding → background` and `background → padding`
//! produce different trees here as they do there.
//!
//! Every arm constructs the exact same underlying view/metadata tree as the
//! corresponding `ViewExt` method in the facade. That is not a style
//! preference: `disabled` is three things at once — an environment scope, an
//! accessibility state and a hit-test switch — and an arm that installed only
//! the first would give a TypeScript `<VStack disabled>` a different
//! accessibility tree from the Rust one, which is the kind of divergence the
//! equivalence tests exist to catch.
//!
//! A value is a reactive input wherever the Rust modifier takes one, and a
//! plain value wherever it takes one of those: `foreground` follows a signal
//! because `Color` is reactive, and `shadow`'s blur radius in TypeScript is a
//! plain value matching [`ShadowValue`].

use nami::{Computed, SignalExt as _};
use suiteki::Str;
use waterui_accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityIdentifier, AccessibilityLabel,
    AccessibilityState, AccessibilityStateSignal, AccessibilityValue,
};
use waterui_controls::theme::ForegroundOverride;
use waterui_core::env::Installed;
use waterui_core::plugin::Plugin;
use waterui_core::{AnyView, Environment, Metadata, OneOf2, View};
use waterui_graphics::WorkingColor;
use waterui_graphics::color::{Color, Working};
use waterui_interaction::gesture::{GestureObserver, TapGesture};
use waterui_interaction::interaction::{Disabled, Hittable};
use waterui_layout::background::BackgroundView;
use waterui_layout::frame::Frame;
use waterui_layout::padding::Padding;
use waterui_locale::locale_binding;
use waterui_shape::border::Border;
use waterui_shape::filter::Opacity;
use waterui_shape::style::Shadow;
use waterui_shape::{Capsule, Circle, ClipShape, Ellipse, Rectangle, RoundedRectangle};
use waterui_ts::engine::{JsError, JsValue};
use waterui_ts::{Bridge, FromJs as _};

use crate::catalog::{
    A11yChildrenValue, BackgroundValue, BorderValue, JsAction, PaddingValue, PickerStyleValue,
    ShadowValue, ShapeValue, TextContent, color_of,
};

struct DisabledScope(Computed<bool>);

impl Plugin for DisabledScope {
    fn install(self, env: &mut Environment) {
        Disabled::install(env, self.0);
    }
}

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
            let padding = PaddingValue::from_js(value, bridge)?;
            Ok(AnyView::new(Padding::new(padding.insets(), view)))
        }
        "background" => match BackgroundValue::from_js(value, bridge)? {
            BackgroundValue::Color(color) => {
                Ok(AnyView::new(BackgroundView::new(view, color_of(&color))))
            }
            BackgroundValue::View(background) => {
                Ok(AnyView::new(BackgroundView::new(view, background)))
            }
        },
        "foreground" => {
            let color = bridge.materialize_computed(value)?;
            Ok(AnyView::new(Installed::new(
                view,
                ForegroundOverride::new(color_of(&color)),
            )))
        }
        "opacity" => {
            let amount: Computed<f32> = bridge.materialize_computed(value)?;
            Ok(AnyView::new(Metadata::new(view, Opacity::new(amount))))
        }
        "shadow" => {
            let shadow = ShadowValue::from_js(value, bridge)?;
            Ok(AnyView::new(Metadata::new(view, Shadow::from(shadow))))
        }
        "border" => {
            let border = BorderValue::from_js(value, bridge)?;
            Ok(AnyView::new(Metadata::new(view, Border::from(border))))
        }
        "clip" => Ok(clip(ShapeValue::from_js(value, bridge)?, view)),
        "width" => frame(bridge, view, value, Frame::width),
        "height" => frame(bridge, view, value, Frame::height),
        "minWidth" => frame(bridge, view, value, Frame::min_width),
        "maxWidth" => frame(bridge, view, value, Frame::max_width),
        "minHeight" => frame(bridge, view, value, Frame::min_height),
        "maxHeight" => frame(bridge, view, value, Frame::max_height),
        "disabled" => disabled(bridge, view, value),
        "onTapGesture" => {
            let action = JsAction::from_js(value, bridge)?;
            Ok(AnyView::new(Metadata::new(
                view,
                GestureObserver::without_event(TapGesture::new(), move || action.call(())),
            )))
        }
        "a11yLabel" => semantic(bridge, view, value, A11yTextKind::Label),
        "a11yValue" => semantic(bridge, view, value, A11yTextKind::Value),
        "a11yId" => {
            let identifier = Str::from_js(value, bridge)?;
            Ok(AnyView::new(Metadata::new(
                view,
                AccessibilityIdentifier::new(identifier),
            )))
        }
        "a11yHidden" => {
            let hidden = bool::from_js(value, bridge)?;
            Ok(AnyView::new(Metadata::new(
                view,
                AccessibilityHidden::new(hidden),
            )))
        }
        "a11yChildren" => {
            let behavior = A11yChildrenValue::from_js(value, bridge)?;
            Ok(AnyView::new(Metadata::new(
                view,
                AccessibilityChildren::from(behavior),
            )))
        }
        "pickerStyle" => {
            let style = PickerStyleValue::from_js(value, bridge)?;
            Ok(AnyView::new(Installed::new(
                view,
                waterui_form::picker::PickerStyle::from(style),
            )))
        }
        other => Err(JsError::conversion(format!(
            "`{other}` is not a view modifier. The catalog's modifiers are: {}",
            crate::catalog::CATALOG
                .modifiers()
                .iter()
                .map(waterui_ts::schema::ModifierSchema::name)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Applies the `disabled` modifier: environment scope, accessibility state, and
/// hit-test suppression matching `ViewExt::disabled`.
fn disabled(bridge: &Bridge, view: AnyView, value: &JsValue) -> Result<AnyView, JsError> {
    let is_disabled: Computed<bool> = bridge.materialize_computed(value)?;
    let accessibility_state = is_disabled.map(|disabled| {
        if disabled {
            AccessibilityState::new().disabled(true)
        } else {
            AccessibilityState::new()
        }
    });
    let hittable_value = is_disabled.map(|d| !d);
    let view = Metadata::new(view, AccessibilityStateSignal::new(accessibility_state));
    let view = Metadata::new(view, Hittable::new(hittable_value));
    Ok(AnyView::new(Installed::new(
        view,
        DisabledScope(is_disabled),
    )))
}

/// Clips the view to one of the shapes the catalog declares.
///
/// The match is on the shape rather than on an erased value because
/// `ClipShape::new` is generic over [`Shape`](waterui_shape::Shape), and the
/// clip carries the shape's kind so a backend can round the corners the same
/// way a fill of the same shape would.
fn clip(shape: ShapeValue, view: AnyView) -> AnyView {
    match shape {
        ShapeValue::Rectangle => AnyView::new(Metadata::new(view, ClipShape::new(Rectangle))),
        ShapeValue::Circle => AnyView::new(Metadata::new(view, ClipShape::new(Circle))),
        ShapeValue::Capsule => AnyView::new(Metadata::new(view, ClipShape::new(Capsule))),
        ShapeValue::Ellipse => AnyView::new(Metadata::new(view, ClipShape::new(Ellipse))),
        ShapeValue::Rounded(radius) => AnyView::new(Metadata::new(
            view,
            ClipShape::new(RoundedRectangle::new(radius)),
        )),
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
    set: impl FnOnce(Frame<AnyView>, Computed<f32>) -> Frame<AnyView>,
) -> Result<AnyView, JsError> {
    let length: Computed<f32> = bridge.materialize_computed(value)?;
    let frame = match view.downcast::<Frame<AnyView>>() {
        Ok(frame) => *frame,
        Err(view) => Frame::new(view),
    };
    Ok(AnyView::new(set(frame, length)))
}

#[derive(Clone, Copy)]
enum A11yTextKind {
    Label,
    Value,
}

/// Applies a verbatim or localized accessibility text modifier against the
/// view's rendering environment, matching [`Text`](waterui_text::Text)'s
/// catalog and locale resolution.
struct LocalizedA11y {
    view: AnyView,
    content: Computed<TextContent>,
    kind: A11yTextKind,
}

impl View for LocalizedA11y {
    fn body(self, env: &Environment) -> impl View {
        let env_for_render = env.clone();
        let locale = locale_binding(env);
        let text = TextContent::flatten_computed(&self.content)
            .zip(&locale)
            .map(move |(part, locale)| part.render(&env_for_render, &locale))
            .computed();
        match self.kind {
            A11yTextKind::Label => {
                OneOf2::A(Metadata::new(self.view, AccessibilityLabel::new(text)))
            }
            A11yTextKind::Value => {
                OneOf2::B(Metadata::new(self.view, AccessibilityValue::new(text)))
            }
        }
    }
}

/// Semantic text for an accessibility attribute, localized like any other.
fn semantic(
    bridge: &Bridge,
    view: AnyView,
    value: &JsValue,
    kind: A11yTextKind,
) -> Result<AnyView, JsError> {
    let content: Computed<TextContent> = bridge.materialize_computed(value)?;
    Ok(AnyView::new(LocalizedA11y {
        view,
        content,
        kind,
    }))
}

impl From<ShadowValue> for Shadow {
    fn from(value: ShadowValue) -> Self {
        let shadow = Self::splat(value.radius())
            .color(value.color().map_or_else(
                || Color::srgb(0, 0, 0),
                |color| Color::new(Working(WorkingColor::from(color))),
            ))
            .offset(value.x().unwrap_or_default(), value.y().unwrap_or_default());
        match value.corner_radius() {
            Some(radius) => shadow.silhouette(RoundedRectangle::new(radius)),
            None => shadow,
        }
    }
}

impl From<BorderValue> for Border {
    fn from(value: BorderValue) -> Self {
        Self::new(
            Color::new(Working(WorkingColor::from(value.color()))),
            value.width(),
        )
        .corner_radius(value.corner_radius().unwrap_or_default())
    }
}

//! The host table: JSX resolved against the catalog, into real `WaterUI`
//! views.
//!
//! [`Components`] is the [`HostTable`] a `TsRuntime` is built with. Every
//! method here is the Rust half of one entry in HOST.md, and every component
//! it can create is one entry in [`CATALOG`](crate::catalog::CATALOG) — the
//! same table `installHost` hands JavaScript its modifier names from, and the
//! same table `water components --json` prints. A tag with no entry is an
//! error naming it, because a component that does not exist cannot be made to
//! exist by guessing.
//!
//! Nothing here reads a value out of a signal. A configuration attribute is
//! materialized as the `Binding<T>` or `Computed<T>` the constructor takes,
//! inside the host call and never before, so an attribute the tree does not
//! use creates nothing on the Rust side.

mod children;
mod collection;
mod control_flow;
mod modifiers;

use nami::Computed;
use waterui_controls::{Button, Slider, Stepper, TextField, Toggle};
use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};
use waterui_core::{AnyView, Error};
use waterui_form::picker::{Picker, PickerItem};
use waterui_layout::ScrollView;
use waterui_layout::stack::{HStack, VStack, ZStack};
use waterui_layout::{Divider, Spacer};
use waterui_text::Text;
use waterui_ts_engine::{JsError, JsFunction, JsValue};

use crate::bridge::Bridge;
use crate::catalog::{
    ButtonAttributes, HStackAttributes, PickerAttributes, PickerOption, ScrollAxis,
    ScrollViewAttributes, SliderAttributes, SpacerAttributes, StepperAttributes,
    TextFieldAttributes, ToggleAttributes, VStackAttributes, ZStackAttributes, component_named,
    component_names, modifier_names,
};
use crate::convert::FromJs;
use crate::host::HostTable;
use collection::Spliced;

/// The default gap a stack leaves between children, which is what `vstack`
/// and `hstack` use.
const DEFAULT_SPACING: f32 = 10.0;

/// `WaterUI`'s component vocabulary, as the TypeScript runtime's host table.
///
/// One value, no state: what a JSX element becomes is decided entirely by the
/// catalog and the values the element carried.
#[derive(Debug, Clone, Copy, Default)]
pub struct Components;

impl HostTable for Components {
    fn modifiers(&self) -> Vec<suiteki::Str> {
        modifier_names()
    }

    fn create(
        &self,
        bridge: &Bridge,
        component: &str,
        config: &[(String, JsValue)],
        children: &[JsValue],
    ) -> Result<AnyView, Error> {
        if component_named(component).is_none() {
            return Err(Error::new(JsError::conversion(format!(
                "<{component}> is not a WaterUI component. The catalog declares: {}",
                component_names().join(", ")
            ))));
        }
        Ok(build(bridge, component, config, children)?)
    }

    fn modify(
        &self,
        bridge: &Bridge,
        view: AnyView,
        name: &str,
        value: &JsValue,
    ) -> Result<AnyView, Error> {
        Ok(modifiers::apply(bridge, view, name, value)?)
    }

    fn text(&self, bridge: &Bridge, content: &JsValue) -> Result<AnyView, Error> {
        let content: Computed<crate::catalog::TextContent> =
            bridge.materialize_computed(content)?;
        Ok(AnyView::new(Text::new(content)))
    }

    fn show(
        &self,
        bridge: &Bridge,
        when: &JsValue,
        render: &JsFunction,
        fallback: Option<&JsFunction>,
    ) -> Result<AnyView, Error> {
        Ok(control_flow::show(bridge, when, render, fallback)?)
    }

    fn each(
        &self,
        bridge: &Bridge,
        items: &JsValue,
        render: &JsFunction,
        by: Option<&JsFunction>,
    ) -> Result<AnyView, Error> {
        Ok(collection::each(bridge, items, render, by)?)
    }

    fn suspense(
        &self,
        bridge: &Bridge,
        children: &JsFunction,
        fallback: Option<&JsFunction>,
    ) -> Result<AnyView, Error> {
        Ok(control_flow::suspense(bridge, children, fallback)?)
    }
}

/// One element, built from its configuration and its children.
///
/// The dispatch is split by family only to keep each arm readable; the catalog
/// is what decides whether a name exists at all.
fn build(
    bridge: &Bridge,
    component: &str,
    config: &[(String, JsValue)],
    children: &[JsValue],
) -> Result<AnyView, JsError> {
    let config = JsValue::Object(config.to_vec());
    if let Some(view) = containers(bridge, component, &config, children)? {
        return Ok(view);
    }
    if let Some(view) = leaves(bridge, component, &config, children)? {
        return Ok(view);
    }
    controls(bridge, component, &config, children)
}

/// The components whose children are content views.
fn containers(
    bridge: &Bridge,
    component: &str,
    config: &JsValue,
    children: &[JsValue],
) -> Result<Option<AnyView>, JsError> {
    Ok(Some(match component {
        "VStack" => {
            let attributes = VStackAttributes::from_js(config, bridge)?;
            let alignment = HorizontalAlignment::from(attributes.alignment.unwrap_or_default());
            match collection::splice(children::content(bridge, children)?) {
                Spliced::Lazy(items) => {
                    collection::into_vstack(&items, alignment, attributes.spacing)
                }
                Spliced::Fixed(views) => {
                    let spacing = attributes.spacing;
                    let mut stack = VStack::new(alignment, DEFAULT_SPACING, views);
                    if let Some(spacing) = spacing {
                        stack = stack.spacing(spacing);
                    }
                    AnyView::new(stack)
                }
            }
        }
        "HStack" => {
            let attributes = HStackAttributes::from_js(config, bridge)?;
            let alignment = VerticalAlignment::from(attributes.alignment.unwrap_or_default());
            match collection::splice(children::content(bridge, children)?) {
                Spliced::Lazy(items) => {
                    collection::into_hstack(&items, alignment, attributes.spacing)
                }
                Spliced::Fixed(views) => {
                    let spacing = attributes.spacing;
                    let mut stack = HStack::new(alignment, DEFAULT_SPACING, views);
                    if let Some(spacing) = spacing {
                        stack = stack.spacing(spacing);
                    }
                    AnyView::new(stack)
                }
            }
        }
        "ZStack" => {
            let attributes = ZStackAttributes::from_js(config, bridge)?;
            let alignment = Alignment::from(attributes.alignment.unwrap_or_default());
            match collection::splice(children::content(bridge, children)?) {
                Spliced::Lazy(items) => collection::into_zstack(&items, alignment),
                Spliced::Fixed(views) => AnyView::new(ZStack::new(alignment, views)),
            }
        }
        "ScrollView" => {
            let attributes = ScrollViewAttributes::from_js(config, bridge)?;
            let content = VStack::new(
                HorizontalAlignment::Center,
                DEFAULT_SPACING,
                children::content(bridge, children)?,
            );
            AnyView::new(match attributes.axis.unwrap_or_default() {
                ScrollAxis::Vertical => ScrollView::vertical(content),
                ScrollAxis::Horizontal => ScrollView::horizontal(content),
                ScrollAxis::Both => ScrollView::both(content),
            })
        }
        _ => return Ok(None),
    }))
}

/// The components that carry no views: gaps, rules and text.
fn leaves(
    bridge: &Bridge,
    component: &str,
    config: &JsValue,
    children: &[JsValue],
) -> Result<Option<AnyView>, JsError> {
    Ok(Some(match component {
        "Spacer" => {
            let attributes = SpacerAttributes::from_js(config, bridge)?;
            children::none(component, children)?;
            AnyView::new(Spacer::new(attributes.min_length.unwrap_or_default()))
        }
        "Divider" => {
            children::none(component, children)?;
            AnyView::new(Divider)
        }
        "Text" => {
            let content = children::text(bridge, children)?;
            AnyView::new(content.map_or_else(|| Text::verbatim(""), Text::new))
        }
        "Label" => AnyView::new(children::label(bridge, component, None, children)?),
        _ => return Ok(None),
    }))
}

/// The controls, every one of which takes a label at construction.
fn controls(
    bridge: &Bridge,
    component: &str,
    config: &JsValue,
    children: &[JsValue],
) -> Result<AnyView, JsError> {
    match component {
        "Button" => {
            let attributes = ButtonAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, attributes.label, children)?;
            let button = Button::new(label);
            Ok(match attributes.on_tap {
                Some(action) => AnyView::new(button.action(move || action.call())),
                None => AnyView::new(button),
            })
        }
        "Toggle" => {
            let attributes = ToggleAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, attributes.label, children)?;
            Ok(AnyView::new(Toggle::new(label, &attributes.value)))
        }
        "Slider" => {
            let attributes = SliderAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, attributes.label, children)?;
            Ok(AnyView::new(Slider::new(label, &attributes.value)))
        }
        "Stepper" => {
            let attributes = StepperAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, attributes.label, children)?;
            let stepper = Stepper::new(label, &attributes.value);
            Ok(AnyView::new(match attributes.step {
                Some(step) => stepper.step(step),
                None => stepper,
            }))
        }
        "TextField" => {
            let attributes = TextFieldAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, attributes.label, children)?;
            let field = TextField::new(label, &attributes.value);
            Ok(AnyView::new(match attributes.prompt {
                Some(prompt) => field.prompt(Text::new(prompt)),
                None => field,
            }))
        }
        "Picker" => {
            let attributes = PickerAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, attributes.label, children)?;
            Ok(AnyView::new(Picker::new(
                label,
                options(&attributes.options),
                &attributes.value,
            )))
        }
        // `create` checked the catalog before dispatching, so a name reaching
        // here is one the catalog declares and this match does not build.
        other => Err(JsError::new(
            "Error",
            format!(
                "<{other}> is in the component catalog but the host table does not build it, \
                 which is a bug in waterui-ts rather than in the view that named it"
            ),
        )),
    }
}

/// The options of a `<Picker>`, as the tagged views it selects between.
///
/// The selection crosses as the option's own value, a string: the C ABI
/// carries `Id`s rather than an application's type, and `Picker::new` is
/// generic exactly so that erasure stays below the authoring layer. Here the
/// authored type *is* the string, and `Mapping` still does the erasure.
fn options(options: &Computed<Vec<PickerOption>>) -> Computed<Vec<PickerItem<suiteki::Str>>> {
    use nami::SignalExt as _;
    options
        .map(|options| {
            options
                .into_iter()
                .map(|option| PickerItem::new(option.value, Text::new(option.label)))
                .collect()
        })
        .computed()
}

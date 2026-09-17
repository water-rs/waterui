//! The host table: JSX resolved against the catalog, into real `WaterUI`
//! views.
//!
//! [`Components`] is the [`HostTable`] a `TsRuntime` is built with. Every
//! method here is the Rust half of one entry in HOST.md, and every component
//! it can create is one entry in [`CATALOG`](crate::ts::catalog::CATALOG) — the
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

use core::cell::RefCell;

use nami::{Computed, SignalExt as _};
use suiteki::Str;
use waterui_controls::label::Label;
use waterui_controls::{Button, Slider, Stepper, TextField, Toggle};
use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};
use waterui_core::{AnyView, Environment, Error, View};
use waterui_form::picker::{Picker, PickerItem};
use waterui_layout::ScrollView;
use waterui_layout::stack::{HStack, HStackLayout, VStack, VStackLayout, ZStack};
use waterui_layout::{Divider, Spacer};
use waterui_navigation::tab::{Tab, Tabs};
use waterui_navigation::{NavigationLink, NavigationStack, NavigationView};

use crate::component::badge::Badge;
use crate::component::link::Link;
use crate::component::progress::Progress;
use crate::component::table::{TableColumn, table};
use crate::widget::accordion::Accordion;
use crate::widget::avatar::Avatar;
use crate::widget::card::Card;
use waterui_core::handler::ViewBuilder as _;
#[cfg(feature = "media")]
use waterui_media::Photo;
use waterui_text::Text;
use waterui_ts::engine::{JsError, JsFunction, JsValue};
use waterui_ts::{Bridge, FromJs, HostTable, JsViewBuilder};

#[cfg(feature = "media")]
use crate::ts::catalog::ImageAttributes;
use crate::ts::catalog::{
    AccordionAttributes, AvatarAttributes, BadgeAttributes, ButtonAttributes, CardAttributes,
    ColumnAttributes, HStackAttributes, LinkAttributes, ListAttributes, NavigationLinkAttributes,
    NavigationStackAttributes, PickerAttributes, PickerOption, ProgressAttributes,
    ProgressStyleValue, ScrollAxis, ScrollViewAttributes, SliderAttributes, SpacerAttributes,
    StepperAttributes, TabAttributes, TableAttributes, TabsAttributes, TextFieldAttributes,
    ToggleAttributes, VStackAttributes, ZStackAttributes, attribute_names, component_named,
    component_names, modifier_names,
};
use collection::Spliced;

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
        let Some(entry) = component_named(component) else {
            return Err(Error::new(JsError::conversion(format!(
                "<{component}> is not a WaterUI component. The catalog declares: {}",
                component_names().join(", ")
            ))));
        };
        // Modifier attributes never reach here — the runtime routes them to
        // `modify` — so what is left is exactly the configuration, and a name
        // the entry does not declare is a typo that would otherwise render and
        // do nothing.
        waterui_ts::support::reject_unknown(config, component, &attribute_names(entry))?;
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
        let content: Computed<crate::ts::catalog::TextContent> =
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
    if let Some(view) = collections(bridge, component, &config, children)? {
        return Ok(view);
    }
    if let Some(view) = navigation(bridge, component, &config, children)? {
        return Ok(view);
    }
    controls(bridge, component, &config, children)
}

/// The gap a stack leaves when the element named none.
///
/// It is read from the layout's own `Default` rather than written here, so the
/// TypeScript stack and the Rust one cannot drift apart when the framework
/// changes its mind about the number.
fn default_vertical_spacing() -> Computed<f32> {
    VStackLayout::default().spacing
}

/// The same, for a horizontal stack.
fn default_horizontal_spacing() -> Computed<f32> {
    HStackLayout::default().spacing
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
            let spacing = attributes.spacing.unwrap_or_else(default_vertical_spacing);
            match collection::splice(children::content(bridge, children)?) {
                Spliced::Lazy(items) => collection::into_vstack(&items, alignment, Some(spacing)),
                Spliced::Fixed(views) => {
                    AnyView::new(VStack::new(alignment, 0.0, views).spacing(spacing))
                }
            }
        }
        "HStack" => {
            let attributes = HStackAttributes::from_js(config, bridge)?;
            let alignment = VerticalAlignment::from(attributes.alignment.unwrap_or_default());
            let spacing = attributes
                .spacing
                .unwrap_or_else(default_horizontal_spacing);
            match collection::splice(children::content(bridge, children)?) {
                Spliced::Lazy(items) => collection::into_hstack(&items, alignment, Some(spacing)),
                Spliced::Fixed(views) => {
                    AnyView::new(HStack::new(alignment, 0.0, views).spacing(spacing))
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
                0.0,
                children::content(bridge, children)?,
            )
            .spacing(default_vertical_spacing());
            AnyView::new(match attributes.axis.unwrap_or_default() {
                ScrollAxis::Vertical => ScrollView::vertical(content),
                ScrollAxis::Horizontal => ScrollView::horizontal(content),
                ScrollAxis::Both => ScrollView::both(content),
            })
        }
        "Card" => {
            let attributes = CardAttributes::from_js(config, bridge)?;
            let content = VStack::new(
                HorizontalAlignment::Leading,
                0.0,
                children::content(bridge, children)?,
            )
            .spacing(default_vertical_spacing());
            let mut card = Card::new(content);
            if let Some(title) = attributes.title {
                card = card.title(title);
            }
            if let Some(subtitle) = attributes.subtitle {
                card = card.subtitle(subtitle);
            }
            AnyView::new(card)
        }
        "Accordion" => {
            let attributes = AccordionAttributes::from_js(config, bridge)?;
            let header = VStack::new(
                HorizontalAlignment::Leading,
                0.0,
                children::content(bridge, children)?,
            )
            .spacing(default_vertical_spacing());
            let content = attributes.content;
            AnyView::new(match attributes.expanded {
                Some(expanded) => Accordion::with_toggle(&expanded, header, content),
                None => Accordion::new(header, content),
            })
        }
        _ => return Ok(None),
    }))
}

/// The components that carry no views: gaps, rules, text and pictures.
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
        "Badge" => {
            let attributes = BadgeAttributes::from_js(config, bridge)?;
            children::none(component, children)?;
            AnyView::new(Badge::new(attributes.value, Rebuilt(attributes.content)))
        }
        "Avatar" => {
            let attributes = AvatarAttributes::from_js(config, bridge)?;
            children::none(component, children)?;
            let avatar = Avatar::new(Text::new(attributes.name), || ());
            AnyView::new(match attributes.image {
                Some(source) => avatar.image(source.map(|url| url.0).computed()),
                None => avatar,
            })
        }
        #[cfg(feature = "media")]
        "Image" => {
            let attributes = ImageAttributes::from_js(config, bridge)?;
            children::none(component, children)?;
            let photo = Photo::new(attributes.src.map(|url| url.0).computed());
            AnyView::new(if attributes.resizable.unwrap_or(false) {
                photo.resizable()
            } else {
                photo
            })
        }
        _ => return Ok(None),
    }))
}

/// The components built out of their children rather than wrapping them:
/// lists, tables and trees.
fn collections(
    bridge: &Bridge,
    component: &str,
    config: &JsValue,
    children: &[JsValue],
) -> Result<Option<AnyView>, JsError> {
    Ok(Some(match component {
        "List" => {
            let attributes = ListAttributes::from_js(config, bridge)?;
            // A list rebuilds a row whenever it realizes it again — after a
            // scroll recycles it, after an edit — and a view that crossed from
            // JavaScript is realized once. `<For>` is what can answer that: its
            // rows are rendered by calling back into the module, as often as
            // the list asks. So a list's content is a collection, which is also
            // the fine-grained path the framework wants for a list.
            match collection::splice(children::content(bridge, children)?) {
                Spliced::Lazy(items) => collection::into_list(&items, attributes),
                Spliced::Fixed(_) => {
                    return Err(JsError::conversion(
                        "<List> takes a <For> as its content. A list realizes a row again \
                         whenever it needs it — after a scroll recycles it, after an edit — and a \
                         view written straight into the list can be realized only once, so the \
                         rows come from <For each={…}>, which renders each one on demand",
                    ));
                }
            }
        }
        "Table" => {
            TableAttributes::from_js(config, bridge)?;
            let columns = children::content(bridge, children)?
                .into_iter()
                .map(|child| Carried::<TableColumn>::take_from(child, "Column", "Table"))
                .collect::<Result<Vec<_>, _>>()?;
            AnyView::new(table(columns))
        }
        "Column" => {
            let attributes = ColumnAttributes::from_js(config, bridge)?;
            children::none(component, &[])?;
            AnyView::new(Carried::new(TableColumn::new(
                attributes.label,
                children::texts(bridge, children)?,
            )))
        }
        _ => return Ok(None),
    }))
}

/// The navigation containers, whose destinations are builders rather than
/// views.
fn navigation(
    bridge: &Bridge,
    component: &str,
    config: &JsValue,
    children: &[JsValue],
) -> Result<Option<AnyView>, JsError> {
    Ok(Some(match component {
        "NavigationStack" => {
            let attributes = NavigationStackAttributes::from_js(config, bridge)?;
            let root = VStack::new(
                HorizontalAlignment::Center,
                0.0,
                children::content(bridge, children)?,
            )
            .spacing(default_vertical_spacing());
            AnyView::new(NavigationStack::new(NavigationView::new(
                title_of(attributes.title),
                root,
            )))
        }
        "NavigationLink" => {
            let attributes = NavigationLinkAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, None, children)?;
            let destination = attributes.destination;
            let title = title_of(attributes.title);
            AnyView::new(NavigationLink::new(label, move || {
                NavigationView::new(title.clone(), destination.build())
            }))
        }
        "Tabs" => {
            let attributes = TabsAttributes::from_js(config, bridge)?;
            let tabs = children::content(bridge, children)?
                .into_iter()
                .map(|child| Carried::<Tab<Str>>::take_from(child, "Tab", "Tabs"))
                .collect::<Result<Vec<_>, _>>()?;
            AnyView::new(Tabs::new(&attributes.value, tabs))
        }
        "Tab" => {
            let attributes = TabAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, None, children)?;
            let content = attributes.content;
            let title = title_of(attributes.title);
            AnyView::new(Carried::new(Tab::new(attributes.value, label, move || {
                NavigationView::new(title.clone(), content.build())
            })))
        }
        _ => return Ok(None),
    }))
}

/// A navigation title, which every destination carries whether or not the
/// element named one.
fn title_of(title: Option<crate::ts::catalog::TextContent>) -> Text {
    title.map_or_else(|| Text::verbatim(""), Text::new)
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
                Some(action) => AnyView::new(button.action(move || action.call(()))),
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
        "Progress" => {
            let attributes = ProgressAttributes::from_js(config, bridge)?;
            let label = children::label_or_none(bridge, children)?;
            progress_of(attributes, label)
        }
        "Link" => {
            let attributes = LinkAttributes::from_js(config, bridge)?;
            let label = children::label(bridge, component, None, children)?;
            Ok(AnyView::new(Link::new(label, attributes.url)))
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

/// A `<Progress>`, determinate or not, with whatever style it asked for.
///
/// A total is a separate Rust type — `ProgressWithTotal` — because a progress
/// counted out of something is a different thing from one counted from zero to
/// one, and the two carry different styles: `loading` is indeterminate, so it
/// has no meaning beside a total and the pair is refused rather than silently
/// dropping one of them.
fn progress_of(attributes: ProgressAttributes, label: Option<Label>) -> Result<AnyView, JsError> {
    let style = attributes.style.unwrap_or_default();
    let Some(value) = attributes.value else {
        let progress = Progress::infinity();
        return Ok(AnyView::new(match label {
            Some(label) => progress.label(label),
            None => progress,
        }));
    };
    if let Some(total) = attributes.total {
        let progress = Progress::new(value).total(total);
        let progress = match style {
            ProgressStyleValue::Linear => progress.linear(),
            ProgressStyleValue::Circular => progress.circular(),
            ProgressStyleValue::Loading => {
                return Err(JsError::conversion(
                    "<Progress style=\"Loading\"> is indeterminate, so it cannot also carry a \
                     `total`: drop the total, or name the style the bar should take",
                ));
            }
        };
        return Ok(AnyView::new(match label {
            Some(label) => progress.label(label),
            None => progress,
        }));
    }
    let progress = Progress::new(value);
    let progress = match style {
        ProgressStyleValue::Linear => progress.linear(),
        ProgressStyleValue::Circular => progress.circular(),
        ProgressStyleValue::Loading => progress.loading(),
    };
    Ok(AnyView::new(match label {
        Some(label) => progress.label(label),
        None => progress,
    }))
}

/// A JavaScript builder used where the framework takes a view it may build
/// again.
///
/// `Badge` keeps an `AnyViewBuilder` and builds its content whenever it needs
/// it, so the content it is handed must be `Clone` and must produce a fresh
/// subtree per build. That is exactly a [`JsViewBuilder`], and this is the
/// view it presents as.
#[derive(Clone)]
struct Rebuilt(JsViewBuilder);

impl core::fmt::Debug for Rebuilt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Rebuilt").finish_non_exhaustive()
    }
}

impl View for Rebuilt {
    fn body(self, _env: &Environment) -> impl View {
        self.0.build()
    }
}

/// A value one component builds for its parent to take.
///
/// `<Column>` and `<Tab>` are not views: they are the pieces `Table` and
/// `Tabs` are assembled from. They still cross as handles, because every child
/// does, so each carries its value in a slot the parent empties.
struct Carried<T: 'static>(RefCell<Option<T>>);

impl<T: 'static> Carried<T> {
    /// Puts `value` in a fresh carrier.
    const fn new(value: T) -> Self {
        Self(RefCell::new(Some(value)))
    }

    /// Takes the value a child carried, or says which parent it belongs under.
    fn take_from(child: AnyView, element: &str, parent: &str) -> Result<T, JsError> {
        let carried = child.downcast::<Self>().map_err(|view| {
            JsError::conversion(format!(
                "<{parent}> takes <{element}> children, found {}",
                view.name()
            ))
        })?;
        carried.0.borrow_mut().take().ok_or_else(|| {
            JsError::conversion(format!("this <{element}> is already part of a <{parent}>"))
        })
    }
}

impl<T: 'static> View for Carried<T> {
    /// # Panics
    ///
    /// Always: a carrier is taken by its parent, never realized. Reaching here
    /// means the element was written somewhere its parent cannot see it.
    fn body(self, _env: &Environment) -> impl View {
        assert!(
            self.0.borrow().is_none(),
            "this element is a piece of the component that contains it, and was written outside \
             one: a <Column> belongs in a <Table> and a <Tab> in a <Tabs>"
        );
        // A carrier its parent already emptied renders nothing, which is what
        // it always was: the value went to the parent, and nothing is left to
        // draw at the position the child occupied.
    }
}

/// The options of a `<Picker>`, as the tagged views it selects between.
///
/// The selection crosses as the option's own value, a string: the C ABI
/// carries `Id`s rather than an application's type, and `Picker::new` is
/// generic exactly so that erasure stays below the authoring layer. Here the
/// authored type *is* the string, and `Mapping` still does the erasure.
fn options(options: &Computed<Vec<PickerOption>>) -> Computed<Vec<PickerItem<Str>>> {
    options
        .map(|options| {
            options
                .into_iter()
                .map(|option| PickerItem::new(option.value, Text::new(option.label)))
                .collect()
        })
        .computed()
}

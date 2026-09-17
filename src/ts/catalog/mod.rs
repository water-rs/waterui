//! The component catalog: what JSX may name, and what each name accepts.
//!
//! The table is Rust data, assembled from the attribute structs' own schemas,
//! and it is the only source there is. The host resolves a JSX tag against it;
//! `installHost` hands JavaScript its modifier names, so classifying an
//! attribute stays a JavaScript-local lookup; and the whole table is encoded
//! into [`waterui_meta_ts_catalog`], the `#[used] static` that `water
//! components --json` and the `waterui` module's `.d.ts` generator read back
//! out of this crate's rlib. A component with no entry cannot be named from
//! JSX: [`create`](waterui_ts::HostTable::create) answers with an error naming it.
//!
//! # What is in it, and what is not
//!
//! The catalog covers the components whose realization is a native primitive
//! and whose Rust constructor can be driven by the shapes JavaScript hands
//! over. Two boundaries decide the rest, and both are structural rather than a
//! matter of effort:
//!
//! * A component that lives in the `waterui` facade rather than in a component
//!   crate — `List`, `Progress`, `Link`, `Card` — is unreachable from here.
//!   The facade depends on this crate under its `ts` feature, so a dependency
//!   back would be a cycle. Those components join the catalog when their
//!   implementations move into component crates, which is where every other
//!   component already lives.
//! * A component whose content is a *builder* rather than a view — a
//!   navigation destination, a tab's root, which `ViewBuilder::build` may call
//!   again at any time — needs a JavaScript render function the props schema
//!   cannot yet type: there is no node for "a function returning a view", and a
//!   view that crossed once cannot be handed over twice.
//! * An image has no native counterpart a JavaScript value constructs. A
//!   bundled image is an `ImageAsset`, whose handle the asset macros produce at
//!   compile time, and the URL-backed `Photo` lives in `waterui-media`, whose
//!   graph is the video stack, an HTTP client and the GPU image decoder. A
//!   TypeScript application linking all of that to show a picture is the
//!   package-size boundary the framework draws, not a detail, so `<Image>`
//!   waits for the decision about which of the two it is.
//!
//! Neither gap is papered over with a half-working entry, because a container
//! whose destinations cannot be built is a promise the runtime cannot keep.

mod attributes;
mod values;

use suiteki::Str;
use waterui_ts::schema::{
    CatalogSchema, ChildrenSlot, ComponentSchema, ModifierSchema, TsType, TypeSchema,
    attributes_of, catalog_encoded_len, encode_catalog, payload,
};

#[cfg(feature = "media")]
pub use attributes::ImageAttributes;
pub use attributes::{
    AccordionAttributes, AvatarAttributes, BadgeAttributes, ButtonAttributes, CardAttributes,
    ColumnAttributes, DividerAttributes, HStackAttributes, LabelAttributes, LinkAttributes,
    ListAttributes, Modifiers, NavigationLinkAttributes, NavigationStackAttributes,
    PickerAttributes, ProgressAttributes, ScrollViewAttributes, SliderAttributes, SpacerAttributes,
    StepperAttributes, TabAttributes, TableAttributes, TabsAttributes, TextAttributes,
    TextFieldAttributes, ToggleAttributes, VStackAttributes, ZStackAttributes,
};
pub use values::color_of;
pub use values::{
    BackgroundValue, BorderValue, ColorValue, EdgeValues, HorizontalAlign, JsAction, PaddingValue,
    PickerOption, ProgressStyleValue, RoundedShape, ScrollAxis, ShadowValue, ShapeName, ShapeValue,
    StackAlign, TextContent, UrlValue, VerticalAlign,
};

/// The whole vocabulary.
pub const CATALOG: CatalogSchema = CatalogSchema {
    components: &COMPONENTS,
    modifiers: &MODIFIERS,
};

/// The components every build carries.
const CORE: [ComponentSchema; 27] = [
    component::<VStackAttributes>(
        "VStack",
        "Stacks its children top to bottom.",
        ChildrenSlot::Content,
    ),
    component::<HStackAttributes>(
        "HStack",
        "Stacks its children leading to trailing.",
        ChildrenSlot::Content,
    ),
    component::<ZStackAttributes>(
        "ZStack",
        "Stacks its children front to back, aligned in both axes.",
        ChildrenSlot::Content,
    ),
    component::<ScrollViewAttributes>(
        "ScrollView",
        "Scrolls its children when they do not fit.",
        ChildrenSlot::Content,
    ),
    component::<SpacerAttributes>(
        "Spacer",
        "A flexible gap that takes the space its siblings leave.",
        ChildrenSlot::None,
    ),
    component::<DividerAttributes>(
        "Divider",
        "A hairline across its stack's cross axis, in the theme's border colour.",
        ChildrenSlot::None,
    ),
    component::<TextAttributes>(
        "Text",
        "Semantic text, localized through the same catalog as Rust's `text`.",
        ChildrenSlot::Text,
    ),
    component::<LabelAttributes>(
        "Label",
        "Text that names a control: the explicit form of a control's label slot.",
        ChildrenSlot::Text,
    ),
    component::<ButtonAttributes>(
        "Button",
        "A control that performs an action when it is activated.",
        ChildrenSlot::Label,
    ),
    component::<ToggleAttributes>(
        "Toggle",
        "A two-state switch bound to a boolean signal.",
        ChildrenSlot::Label,
    ),
    component::<SliderAttributes>(
        "Slider",
        "A control that sets a number by dragging.",
        ChildrenSlot::Label,
    ),
    component::<StepperAttributes>(
        "Stepper",
        "A control that increments and decrements a whole number.",
        ChildrenSlot::Label,
    ),
    component::<TextFieldAttributes>(
        "TextField",
        "A single-line text entry field bound to a string signal.",
        ChildrenSlot::Label,
    ),
    component::<PickerAttributes>(
        "Picker",
        "A control that selects one of a set of options.",
        ChildrenSlot::Label,
    ),
    component::<ListAttributes>(
        "List",
        "A native list: each child is a row, and a section groups them.",
        ChildrenSlot::Content,
    ),
    component::<ProgressAttributes>(
        "Progress",
        "How far along a task is, or that one is running at all.",
        ChildrenSlot::Label,
    ),
    component::<LinkAttributes>("Link", "A label that opens a URL.", ChildrenSlot::Label),
    component::<CardAttributes>(
        "Card",
        "A titled surface around its content.",
        ChildrenSlot::Content,
    ),
    component::<BadgeAttributes>(
        "Badge",
        "A count shown over another view.",
        ChildrenSlot::None,
    ),
    component::<TableAttributes>(
        "Table",
        "Columns of text, one `<Column>` per child.",
        ChildrenSlot::Content,
    ),
    component::<ColumnAttributes>(
        "Column",
        "One column of a `<Table>`: a heading and its rows of text.",
        ChildrenSlot::Content,
    ),
    component::<AccordionAttributes>(
        "Accordion",
        "A header that expands to reveal its content.",
        ChildrenSlot::Content,
    ),
    component::<AvatarAttributes>(
        "Avatar",
        "A person's picture, or their monogram when there is none.",
        ChildrenSlot::None,
    ),
    component::<NavigationStackAttributes>(
        "NavigationStack",
        "A stack of destinations, rooted at its children.",
        ChildrenSlot::Content,
    ),
    component::<NavigationLinkAttributes>(
        "NavigationLink",
        "A label that pushes a destination onto the navigation stack.",
        ChildrenSlot::Label,
    ),
    component::<TabsAttributes>(
        "Tabs",
        "A native tab container, one `<Tab>` per child.",
        ChildrenSlot::Content,
    ),
    component::<TabAttributes>(
        "Tab",
        "One page of a `<Tabs>`, whose children are its label.",
        ChildrenSlot::Label,
    ),
];

/// The components a feature adds.
///
/// `<Image>` exists only with the facade's `media` feature, because the image
/// it shows is fetched and decoded by `waterui-media`: an HTTP client, the GPU
/// image decoder and the video stack behind it. An application that shows no
/// pictures pays for none of that, which is the proportional-size rule the
/// framework holds every native bridge to, and the catalog says so rather than
/// declaring a component the build cannot make.
#[cfg(feature = "media")]
const COMPONENTS: [ComponentSchema; CORE.len() + 1] = with(component::<ImageAttributes>(
    "Image",
    "A picture fetched from a URL.",
    ChildrenSlot::None,
));

/// The components, with nothing added.
#[cfg(not(feature = "media"))]
const COMPONENTS: [ComponentSchema; CORE.len()] = CORE;

/// [`CORE`] with one more entry after it.
#[cfg(feature = "media")]
const fn with(extra: ComponentSchema) -> [ComponentSchema; CORE.len() + 1] {
    // The array starts filled with `extra`, so the last slot is already what
    // it should be and the loop only copies the core entries over the rest.
    let mut all = [extra; CORE.len() + 1];
    let mut index = 0;
    while index < CORE.len() {
        all[index] = CORE[index];
        index += 1;
    }
    all
}

/// The modifiers, which apply to every component because `ViewExt` does.
const MODIFIERS: [ModifierSchema; 19] = [
    modifier(
        "padding",
        "Insets the view, in written order with the rest of the chain.",
    ),
    modifier("background", "Paints a colour, or a view, behind the view."),
    modifier("foreground", "Tints the view's text and symbols."),
    modifier("opacity", "Makes the view translucent."),
    modifier("shadow", "Casts a shadow behind the view."),
    modifier("border", "Strokes a border around the view."),
    modifier("clip", "Clips the view to a shape."),
    modifier("width", "Fixes the view's width, in points."),
    modifier("height", "Fixes the view's height, in points."),
    modifier("minWidth", "The width the view will not go below."),
    modifier("maxWidth", "The width the view will not exceed."),
    modifier("minHeight", "The height the view will not go below."),
    modifier("maxHeight", "The height the view will not exceed."),
    modifier("disabled", "Disables the view and everything below it."),
    modifier("onTapGesture", "Runs a handler when the view is tapped."),
    modifier("a11yLabel", "Names the view for assistive technology."),
    modifier("a11yValue", "The view's value, for assistive technology."),
    modifier("a11yId", "A stable identifier for tests and tooling."),
    modifier("a11yHidden", "Hides the view from assistive technology."),
];

/// One component entry, taking its attributes from the struct that reads them.
const fn component<A: TsType>(
    name: &'static str,
    summary: &'static str,
    children: ChildrenSlot,
) -> ComponentSchema {
    ComponentSchema {
        name,
        summary,
        attributes: attributes_of(&A::SCHEMA),
        children,
    }
}

/// One modifier entry, taking its value shape from the field of the modifier
/// table that declares it.
///
/// The name is looked up rather than positional, so a modifier listed here
/// that [`Modifiers`] does not declare fails const evaluation instead of
/// reaching JavaScript as a name nothing can apply.
const fn modifier(name: &'static str, summary: &'static str) -> ModifierSchema {
    ModifierSchema {
        name,
        summary,
        value: modifier_value(name),
    }
}

/// The value shape [`Modifiers`] declares for `name`.
const fn modifier_value(name: &str) -> &'static TypeSchema {
    let fields = match Modifiers::SCHEMA {
        TypeSchema::Struct(schema) => schema.fields,
        _ => panic!("the modifier table is an object type"),
    };
    let mut index = 0;
    while index < fields.len() {
        if str_eq(fields[index].name, name) {
            return &fields[index].ty;
        }
        index += 1;
    }
    panic!("the catalog lists a modifier the modifier table does not declare")
}

/// Byte-wise string equality, which `==` is not in a const context.
const fn str_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// The encoded catalog, NUL-terminated as the artifact static carries it.
const ENCODED: [u8; catalog_encoded_len(&CATALOG) + 1] = encode_catalog(&CATALOG);

/// The encoded catalog, read back by `water components --json` and by the
/// `.d.ts` generator.
///
/// `#[used]` keeps the item in the object file and the rlib, so the CLI
/// enumerates it by its `waterui_meta_` prefix and cuts the section data at
/// the first NUL — a Mach-O symbol carries no size. The debug gate is what
/// keeps a shipped application free of it.
#[cfg(debug_assertions)]
#[used]
#[expect(
    non_upper_case_globals,
    reason = "tooling enumerates the symbol by its `waterui_meta_` prefix, so the name is the \
              contract"
)]
pub static waterui_meta_ts_catalog: [u8; catalog_encoded_len(&CATALOG) + 1] = ENCODED;

/// The encoded catalog without its terminator: the exact bytes the CLI
/// recovers from the artifact.
pub const CATALOG_ENCODED: &[u8] = payload(&ENCODED);

/// The entry for `name`, or `None` when the catalog has none.
#[must_use]
pub fn component_named(name: &str) -> Option<&'static ComponentSchema> {
    CATALOG
        .components
        .iter()
        .find(|component| component.name == name)
}

/// Every component name, for the error an unknown tag raises.
#[must_use]
pub fn component_names() -> Vec<&'static str> {
    CATALOG
        .components
        .iter()
        .map(|component| component.name)
        .collect()
}

/// The attribute names a component's catalog entry declares.
///
/// The host checks what a JSX element carried against this, because an
/// attribute nobody reads is what an author expects to have an effect.
#[must_use]
pub fn attribute_names(component: &ComponentSchema) -> Vec<&'static str> {
    component
        .attributes
        .fields
        .iter()
        .map(|field| field.name)
        .collect()
}

/// Every modifier name, in catalog order — what `installHost` receives.
#[must_use]
pub fn modifier_names() -> Vec<Str> {
    CATALOG
        .modifiers
        .iter()
        .map(|modifier| Str::from_static(modifier.name))
        .collect()
}

//! Every catalog entry, mounted twice: once as JSX and once as the Rust view
//! it claims to be, with the two accessibility trees compared.
//!
//! That comparison is what makes the catalog a binding rather than a
//! lookalike. A TypeScript `<Toggle>` is not "close enough" to a Rust
//! `toggle(…)` — it *is* one, so anything a binding drops on the way (a label,
//! an action, a value, a bound, a disabled state) shows up here as a
//! difference in the tree rather than as a screen nobody can use.
//!
//! Every default the Rust form needs is read from the framework, never written
//! here: a stack's spacing comes from `VStackLayout::default()` and a bare
//! padding from `DEFAULT_PADDING`, so a test cannot agree with the host about
//! a number the framework has since changed.
//!
//! An attribute the tree cannot see is compared where it can be seen. A
//! purely visual one — a colour, an opacity, a shadow, a border, a clip, a
//! field's prompt — moves no node, label, action or bound, so its two forms
//! are compared as the frames they render (`assert_same_pixels`). One that
//! only shows when the control is used — a stepper's step, a handler on the
//! other side of the seam — is driven through `waterui-testing` and observed
//! in the JavaScript it reaches. A case whose two trees would be the same
//! tree whether the attribute was read or dropped is not written as a tree
//! comparison, because it could never fail.

mod support;

use nami::Binding;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem};
use waterui::component::table::{col, table};
use waterui::component::{Link, button, field, label, progress, slider, stepper, toggle};
use waterui::form::picker::{Picker, PickerItem};
use waterui::layout::padding::EdgeInsets;
use waterui::layout::stack::{HStack, HStackLayout, VStack, VStackLayout, ZStack};
use waterui::layout::{Divider, ScrollView, Spacer};
use waterui::navigation::tab::{Tab, Tabs};
use waterui::navigation::{NavigationLink, NavigationStack, NavigationView};
use waterui::text::text;
use waterui::widget::{Accordion, Avatar, Card};
use waterui::{AnyView, Str, ViewExt as _};
use waterui_core::id::SelfId;
use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};
use waterui_core::views::ForEach;
use waterui_testing::{OffscreenApp, Role};
use waterui_ts::engine::JsValue;

use support::{
    Module, assert_same_pixels, mount_error, mount_view_offscreen, rust_pixels, rust_tree,
};

/// Mounts the JSX form and the Rust form and asserts they are one tree.
fn assert_same(case: &str, source: &str, rust: impl waterui_core::View + 'static) {
    let jsx = Module::mount(source).tree();
    let rust = rust_tree(rust);
    assert_eq!(jsx, rust, "{case}: the JSX form and the Rust form differ");
}

/// A module whose only export builds one element.
fn module(element: &str) -> String {
    format!("waterui.installRuntimeGlobal({{ main: () => {element} }});")
}

/// A module that sets up state on `globalThis` before building its element.
fn stateful(setup: &str, element: &str) -> String {
    format!("{setup}\nwaterui.installRuntimeGlobal({{ main: () => {element} }});")
}

/// A vertical stack with the framework's own default spacing, which is what a
/// `<VStack>` with no `spacing` builds.
fn vstack(alignment: HorizontalAlignment, children: Vec<AnyView>) -> VStack<(Vec<AnyView>,)> {
    VStack::new(alignment, 0.0, children).spacing(VStackLayout::default().spacing)
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

#[test]
fn ts_vstack_is_a_vstack() {
    assert_same(
        "VStack",
        &module(
            r#"waterui.jsx("VStack", {
                spacing: 8,
                alignment: "Leading",
                children: [
                    waterui.jsx("Text", { children: "First" }),
                    waterui.jsx("Text", { children: "Second" }),
                ],
            })"#,
        ),
        VStack::new(
            HorizontalAlignment::Leading,
            8.0,
            vec![AnyView::new(text("First")), AnyView::new(text("Second"))],
        ),
    );
}

#[test]
fn ts_a_stack_with_no_spacing_uses_the_frameworks_own_default() {
    assert_same(
        "VStack default spacing",
        &module(
            r#"waterui.jsx("VStack", { children: [
                waterui.jsx("Text", { children: "First" }),
                waterui.jsx("Text", { children: "Second" }),
            ] })"#,
        ),
        vstack(
            HorizontalAlignment::Center,
            vec![AnyView::new(text("First")), AnyView::new(text("Second"))],
        ),
    );
}

#[test]
fn ts_hstack_is_an_hstack() {
    // Two children of different heights: the spacing is the gap between
    // them, and the alignment is where the shorter one sits beside the
    // taller. With one child neither would move anything.
    assert_same(
        "HStack",
        &module(
            r#"waterui.jsx("HStack", {
                spacing: 8,
                alignment: "Top",
                children: [
                    waterui.jsx("Text", { children: "Short" }),
                    waterui.jsx("Text", { height: 60, children: "Tall" }),
                ],
            })"#,
        ),
        HStack::new(
            VerticalAlignment::Top,
            8.0,
            vec![
                AnyView::new(text("Short")),
                AnyView::new(text("Tall").height(60.0)),
            ],
        ),
    );
    assert_same(
        "HStack default spacing",
        &module(
            r#"waterui.jsx("HStack", { children: [
                waterui.jsx("Text", { children: "First" }),
                waterui.jsx("Text", { children: "Second" }),
            ] })"#,
        ),
        HStack::new(
            VerticalAlignment::Center,
            0.0,
            vec![AnyView::new(text("First")), AnyView::new(text("Second"))],
        )
        .spacing(HStackLayout::default().spacing),
    );
}

#[test]
fn ts_zstack_is_a_zstack() {
    assert_same(
        "ZStack",
        &module(
            r#"waterui.jsx("ZStack", {
                alignment: "BottomTrailing",
                children: [
                    waterui.jsx("Text", { children: "Back" }),
                    waterui.jsx("Text", { children: "Front" }),
                ],
            })"#,
        ),
        ZStack::new(
            Alignment::BottomTrailing,
            vec![AnyView::new(text("Back")), AnyView::new(text("Front"))],
        ),
    );
}

#[test]
fn ts_scrollview_is_a_scroll_view() {
    assert_same(
        "ScrollView",
        &module(
            r#"waterui.jsx("ScrollView", {
                children: waterui.jsx("Text", { children: "Scrolled" }),
            })"#,
        ),
        ScrollView::vertical(vstack(
            HorizontalAlignment::Center,
            vec![AnyView::new(text("Scrolled"))],
        )),
    );
    assert_same(
        "ScrollView, horizontal",
        &module(
            r#"waterui.jsx("ScrollView", {
                axis: "Horizontal",
                children: waterui.jsx("Text", { children: "Scrolled" }),
            })"#,
        ),
        ScrollView::horizontal(vstack(
            HorizontalAlignment::Center,
            vec![AnyView::new(text("Scrolled"))],
        )),
    );
    assert_same(
        "ScrollView, both axes",
        &module(
            r#"waterui.jsx("ScrollView", {
                axis: "Both",
                children: waterui.jsx("Text", { children: "Scrolled" }),
            })"#,
        ),
        ScrollView::both(vstack(
            HorizontalAlignment::Center,
            vec![AnyView::new(text("Scrolled"))],
        )),
    );
}

#[test]
fn ts_card_is_a_card() {
    assert_same(
        "Card",
        &module(
            r#"waterui.jsx("Card", {
                title: "Weekly report",
                subtitle: "Last seven days",
                children: waterui.jsx("Text", { children: "Everything is fine" }),
            })"#,
        ),
        Card::new(vstack(
            HorizontalAlignment::Leading,
            vec![AnyView::new(text("Everything is fine"))],
        ))
        .title("Weekly report")
        .subtitle("Last seven days"),
    );
}

#[test]
fn ts_accordion_is_an_accordion() {
    assert_same(
        "Accordion",
        &module(
            r#"waterui.jsx("Accordion", {
                content: () => waterui.jsx("Text", { children: "Body" }),
                children: waterui.jsx("Text", { children: "Header" }),
            })"#,
        ),
        Accordion::new(
            vstack(
                HorizontalAlignment::Leading,
                vec![AnyView::new(text("Header"))],
            ),
            || text("Body"),
        ),
    );
    let expanded = Binding::container(true);
    assert_same(
        "Accordion, expanded by a binding",
        &stateful(
            "globalThis.open = waterui.createSignal(true);",
            r#"waterui.jsx("Accordion", {
                expanded: globalThis.open,
                content: () => waterui.jsx("Text", { children: "Body" }),
                children: waterui.jsx("Text", { children: "Header" }),
            })"#,
        ),
        Accordion::with_toggle(
            &expanded,
            vstack(
                HorizontalAlignment::Leading,
                vec![AnyView::new(text("Header"))],
            ),
            || text("Body"),
        ),
    );
}

#[test]
fn ts_an_accordion_header_toggles_the_javascript_signal() {
    let module = Module::mount(&stateful(
        "globalThis.open = waterui.createSignal(false);",
        r#"waterui.jsx("Accordion", {
            expanded: globalThis.open,
            content: () => waterui.jsx("Text", { children: "Body" }),
            children: waterui.jsx("Text", { children: "Header" }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Body").assert_not_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Header")
        .expanded(false)
        .assert_exists();

    app.query().role(Role::BUTTON).label("Header").tap();
    app.settle();
    assert_eq!(
        module.eval("globalThis.open()"),
        JsValue::Bool(true),
        "expanding the accordion wrote the JavaScript signal"
    );
    app.query().label("Body").assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Header")
        .expanded(true)
        .assert_exists();
}

// ---------------------------------------------------------------------------
// Leaves
// ---------------------------------------------------------------------------

#[test]
fn ts_text_is_text() {
    assert_same(
        "Text",
        &module(r#"waterui.jsx("Text", { children: "Hello" })"#),
        text("Hello"),
    );
}

#[test]
fn ts_spacer_and_divider_are_themselves() {
    assert_same(
        "Spacer and Divider",
        &module(
            r#"waterui.jsx("VStack", { children: [
                waterui.jsx("Text", { children: "Above" }),
                waterui.jsx("Spacer", {}),
                waterui.jsx("Divider", {}),
            ] })"#,
        ),
        vstack(
            HorizontalAlignment::Center,
            vec![
                AnyView::new(text("Above")),
                AnyView::new(Spacer::new(0.0)),
                AnyView::new(Divider),
            ],
        ),
    );
}

// A spacer's minimum length has no equivalence case. A spacer with room to
// spare takes the room whatever its minimum, and under compression the stack
// gives it nothing — `Spacer::new(40.0)` in a 50-point `VStack` of two texts
// lays out exactly as `Spacer::new(0.0)` does — so no tree and no frame can
// tell an arm that reads `minLength` from one that drops it. What holds that
// arm is the refusal in `ts_an_attribute_value_of_the_wrong_shape_is_refused`.

#[test]
fn ts_label_is_a_label() {
    assert_same(
        "Label",
        &module(r#"waterui.jsx("Label", { children: "Named" })"#),
        label("Named"),
    );
}

#[test]
fn ts_avatar_is_an_avatar() {
    assert_same(
        "Avatar",
        &module(r#"waterui.jsx("Avatar", { name: "Ada Lovelace" })"#),
        Avatar::new(text("Ada Lovelace"), || ()),
    );
    // An avatar's picture has no equivalence case. `Avatar::body` ends in
    // `.a11y_children(ExcludeDescendants)`, so the picture never reaches the
    // accessibility tree and the two trees are identical whether the `image`
    // attribute was read or dropped; and the picture is fetched from its URL,
    // which a test session cannot reach, so both frames show the monogram as
    // well. What holds that arm honest is the refusal in
    // `ts_an_attribute_value_of_the_wrong_shape_is_refused`, which an arm that
    // never looks at the value cannot produce.
}

#[test]
fn ts_badge_is_a_badge() {
    assert_same(
        "Badge",
        &module(
            r#"waterui.jsx("Badge", {
                value: 3,
                content: () => waterui.jsx("Text", { children: "Inbox" }),
            })"#,
        ),
        text("Inbox").badge(3),
    );
}

#[test]
fn ts_image_is_a_photo() {
    assert_same(
        "Image",
        &module(r#"waterui.jsx("Image", { src: "https://waterui.dev/logo.png" })"#),
        waterui::media::Photo::new(
            waterui::Url::parse("https://waterui.dev/logo.png").expect("a valid URL"),
        ),
    );
    assert_same(
        "Image, resizable",
        &module(
            r#"waterui.jsx("Image", {
                src: "https://waterui.dev/logo.png",
                resizable: true,
            })"#,
        ),
        waterui::media::Photo::new(
            waterui::Url::parse("https://waterui.dev/logo.png").expect("a valid URL"),
        )
        .resizable(),
    );
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

#[test]
fn ts_button_is_a_button() {
    assert_same(
        "Button",
        &module(r#"waterui.jsx("Button", { onTap: () => {}, children: "Save" })"#),
        button("Save").action(|| {}),
    );
}

#[test]
fn ts_toggle_is_a_toggle() {
    let on = Binding::container(true);
    assert_same(
        "Toggle",
        &stateful(
            "globalThis.on = waterui.createSignal(true);",
            r#"waterui.jsx("Toggle", { value: globalThis.on, children: "Dark mode" })"#,
        ),
        toggle("Dark mode", &on),
    );
}

#[test]
fn ts_slider_is_a_slider() {
    let amount = Binding::container(0.25_f64);
    assert_same(
        "Slider",
        &stateful(
            "globalThis.amount = waterui.createSignal(0.25);",
            r#"waterui.jsx("Slider", { value: globalThis.amount, children: "Volume" })"#,
        ),
        slider("Volume", &amount),
    );
}

#[test]
fn ts_stepper_is_a_stepper() {
    // The step is not in the tree — a stepper reports its value, not how far
    // one press moves it — so it is observed by pressing, in
    // `ts_a_stepper_steps_the_javascript_signal_by_its_step`.
    let count = Binding::container(2_i32);
    assert_same(
        "Stepper",
        &stateful(
            "globalThis.count = waterui.createSignal(2);",
            r#"waterui.jsx("Stepper", { value: globalThis.count, children: "Guests" })"#,
        ),
        stepper("Guests", &count),
    );
}

#[test]
fn ts_textfield_is_a_text_field() {
    let name = Binding::container(Str::from("Ada"));
    assert_same(
        "TextField",
        &stateful(
            r#"globalThis.name = waterui.createSignal("Ada");"#,
            r#"waterui.jsx("TextField", { value: globalThis.name, children: "Name" })"#,
        ),
        field("Name", &name),
    );
}

#[test]
fn ts_a_text_fields_prompt_is_the_rust_prompt() {
    // The prompt is what an empty, focused field shows, and it is not in the
    // tree — the node reports the field's value, which is nothing — so the
    // two forms are focused and compared as the frames they render, where the
    // prompt is drawn.
    let focused = |mut app: OffscreenApp| {
        app.semantic_mut().query().label("Name").focus();
        app.semantic_mut().settle();
        app.snapshot()
    };
    let prompted = Binding::container(Str::from(""));
    assert_same_pixels(
        "TextField with a prompt",
        &focused(
            Module::mount(&stateful(
                r#"globalThis.name = waterui.createSignal("");"#,
                r#"waterui.jsx("TextField", {
                    value: globalThis.name,
                    prompt: "Your name",
                    children: "Name",
                })"#,
            ))
            .offscreen(),
        ),
        &focused(mount_view_offscreen(AnyView::new(
            field("Name", &prompted).prompt(text("Your name")),
        ))),
    );
}

#[test]
fn ts_picker_is_a_picker() {
    let choice = Binding::container(Str::from("alpha"));
    assert_same(
        "Picker",
        &stateful(
            r#"globalThis.choice = waterui.createSignal("alpha");"#,
            r#"waterui.jsx("Picker", {
                value: globalThis.choice,
                options: [
                    { value: "alpha", label: "Alpha" },
                    { value: "beta", label: "Beta" },
                ],
                children: "Variant",
            })"#,
        ),
        Picker::new(
            "Variant",
            vec![
                PickerItem::new(Str::from("alpha"), text("Alpha")),
                PickerItem::new(Str::from("beta"), text("Beta")),
            ],
            &choice,
        ),
    );
}

#[test]
fn ts_progress_is_progress() {
    assert_same(
        "Progress",
        &module(r#"waterui.jsx("Progress", { value: 0.4, children: "Uploading" })"#),
        progress(0.4).linear().label("Uploading"),
    );
    assert_same(
        "Progress, circular",
        &module(
            r#"waterui.jsx("Progress", {
                value: 0.4,
                style: "Circular",
                children: "Uploading",
            })"#,
        ),
        progress(0.4).circular().label("Uploading"),
    );
    assert_same(
        "Progress, loading",
        &module(
            r#"waterui.jsx("Progress", {
                value: 0.4,
                style: "Loading",
                children: "Uploading",
            })"#,
        ),
        progress(0.4).loading().label("Uploading"),
    );
    assert_same(
        "Progress out of a total",
        &module(r#"waterui.jsx("Progress", { value: 3, total: 10, children: "Files" })"#),
        progress(3.0).total(10.0).linear().label("Files"),
    );
}

#[test]
fn ts_progress_without_a_value_is_indeterminate() {
    assert_same(
        "Progress, indeterminate",
        &module(r#"waterui.jsx("Progress", { children: "Working" })"#),
        waterui::component::progress::Progress::infinity().label("Working"),
    );
}

#[test]
fn ts_link_is_a_link() {
    // The target is not in the tree — a link node carries its label — and
    // following it hands the URL to the platform, which a test session must
    // not do. What holds the `url` arm is the refusal in
    // `ts_an_attribute_value_of_the_wrong_shape_is_refused`.
    assert_same(
        "Link",
        &module(r#"waterui.jsx("Link", { url: "https://waterui.dev", children: "WaterUI" })"#),
        Link::new(label("WaterUI"), Str::from("https://waterui.dev")),
    );
}

#[test]
fn ts_a_controls_label_attribute_is_its_label() {
    // `label` is the explicit form of the children slot. The Rust twin is the
    // control with that label, so the tree is the same one the children form
    // produces — and an arm that dropped the attribute would leave the
    // control with no label at all, which the host refuses.
    let label = |name: &str| format!(r#"label: waterui.jsx("Label", {{ children: "{name}" }})"#);
    assert_same(
        "Button by label attribute",
        &module(&format!(
            r#"waterui.jsx("Button", {{ onTap: () => {{}}, {} }})"#,
            label("Save")
        )),
        button("Save").action(|| {}),
    );
    let on = Binding::container(true);
    assert_same(
        "Toggle by label attribute",
        &stateful(
            "globalThis.on = waterui.createSignal(true);",
            &format!(
                r#"waterui.jsx("Toggle", {{ value: globalThis.on, {} }})"#,
                label("Dark mode")
            ),
        ),
        toggle("Dark mode", &on),
    );
    let amount = Binding::container(0.25_f64);
    assert_same(
        "Slider by label attribute",
        &stateful(
            "globalThis.amount = waterui.createSignal(0.25);",
            &format!(
                r#"waterui.jsx("Slider", {{ value: globalThis.amount, {} }})"#,
                label("Volume")
            ),
        ),
        slider("Volume", &amount),
    );
    let count = Binding::container(2_i32);
    assert_same(
        "Stepper by label attribute",
        &stateful(
            "globalThis.count = waterui.createSignal(2);",
            &format!(
                r#"waterui.jsx("Stepper", {{ value: globalThis.count, {} }})"#,
                label("Guests")
            ),
        ),
        stepper("Guests", &count),
    );
    let name = Binding::container(Str::from("Ada"));
    assert_same(
        "TextField by label attribute",
        &stateful(
            r#"globalThis.name = waterui.createSignal("Ada");"#,
            &format!(
                r#"waterui.jsx("TextField", {{ value: globalThis.name, {} }})"#,
                label("Name")
            ),
        ),
        field("Name", &name),
    );
    let choice = Binding::container(Str::from("alpha"));
    assert_same(
        "Picker by label attribute",
        &stateful(
            r#"globalThis.choice = waterui.createSignal("alpha");"#,
            &format!(
                r#"waterui.jsx("Picker", {{
                    value: globalThis.choice,
                    options: [{{ value: "alpha", label: "Alpha" }}],
                    {}
                }})"#,
                label("Variant")
            ),
        ),
        Picker::new(
            "Variant",
            vec![PickerItem::new(Str::from("alpha"), text("Alpha"))],
            &choice,
        ),
    );
}

#[test]
fn ts_a_button_activation_reaches_the_javascript_handler() {
    // `onTap` is the button's own activation, distinct from the tap gesture
    // modifier: it is what a native control runs when it is activated, by
    // whatever means. Activating the button through the accessibility action
    // is how a test tells the two apart.
    let module = Module::mount(&stateful(
        "globalThis.saves = 0;",
        r#"waterui.jsx("Button", {
            onTap: () => { globalThis.saves += 1; },
            children: "Save",
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Save").tap();
    app.settle();
    assert_eq!(
        module.eval("globalThis.saves").as_f64(),
        Some(1.0),
        "activating the button ran the JavaScript handler once"
    );
}

#[test]
fn ts_a_slider_writes_through_to_the_javascript_signal() {
    let module = Module::mount(&stateful(
        "globalThis.amount = waterui.createSignal(0.25);",
        r#"waterui.jsx("Slider", { value: globalThis.amount, children: "Volume" })"#,
    ));
    let mut app = module.app();
    app.query().label("Volume").increment();
    app.settle();
    let amount = module
        .eval("globalThis.amount()")
        .as_f64()
        .expect("the signal holds a number");
    assert!(
        amount > 0.25,
        "incrementing the slider moved the JavaScript signal up from 0.25, found {amount}"
    );
}

#[test]
fn ts_a_stepper_steps_the_javascript_signal_by_its_step() {
    // The step is invisible in the tree, and visible the moment the stepper
    // is pressed: one increment moves the signal by the step, not by one.
    let module = Module::mount(&stateful(
        "globalThis.count = waterui.createSignal(2);",
        r#"waterui.jsx("Stepper", { value: globalThis.count, step: 2, children: "Guests" })"#,
    ));
    let mut app = module.app();
    app.query().label("Guests").increment();
    app.settle();
    assert_eq!(
        module.eval("globalThis.count()").as_f64(),
        Some(4.0),
        "one increment moved the JavaScript signal by the step"
    );
    app.query().label("Guests").decrement();
    app.settle();
    assert_eq!(
        module.eval("globalThis.count()").as_f64(),
        Some(2.0),
        "and one decrement moved it back by the step"
    );
}

#[test]
fn ts_a_text_field_writes_through_to_the_javascript_signal() {
    let module = Module::mount(&stateful(
        r#"globalThis.name = waterui.createSignal("Ada");"#,
        r#"waterui.jsx("TextField", { value: globalThis.name, children: "Name" })"#,
    ));
    let mut app = module.app();
    app.query().label("Name").set_text("Grace");
    app.settle();
    assert_eq!(
        module.eval("globalThis.name()"),
        JsValue::String(String::from("Grace")),
        "setting the field's text wrote the JavaScript signal"
    );
}

#[test]
fn ts_a_picker_choice_writes_through_to_the_javascript_signal() {
    let module = Module::mount(&stateful(
        r#"globalThis.choice = waterui.createSignal("alpha");"#,
        r#"waterui.jsx("Picker", {
            value: globalThis.choice,
            options: [
                { value: "alpha", label: "Alpha" },
                { value: "beta", label: "Beta" },
            ],
            children: "Variant",
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Beta").tap();
    app.settle();
    assert_eq!(
        module.eval("globalThis.choice()"),
        JsValue::String(String::from("beta")),
        "choosing an option wrote its value into the JavaScript signal"
    );
    app.query().label("Beta").selected(true).assert_exists();
}

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

#[test]
fn ts_list_is_a_list() {
    let module = Module::mount(&stateful(
        r#"globalThis.rows = waterui.createSignal(["one", "two"]);"#,
        r#"waterui.jsx("List", {
            children: waterui.jsx(waterui.For, {
                each: globalThis.rows,
                children: (row) => waterui.jsx("Text", { children: row }),
            }),
        })"#,
    ));
    let rows = Binding::container(vec![
        SelfId::new(Str::from("one")),
        SelfId::new(Str::from("two")),
    ]);
    assert_eq!(
        module.tree(),
        rust_tree(List::new(ForEach::new(
            SignalCollection::new(rows),
            |row: SelfId<Str>| ListItem::new(text(row.into_inner())),
        ))),
        "the JSX list and the Rust list differ"
    );
}

/// How far inside a row's trailing edge its edit controls are.
///
/// A list insets them by a few points and makes them far wider than that, so a
/// point this far in is on the trailing-most control whatever width the theme
/// gives it. The controls are chrome the list draws rather than nodes it
/// publishes, so pressing one is how a test reaches them.
const TRAILING_CONTROL_PROBE: f32 = 12.0;

/// A list in edit mode with the handler named by `handler`, over two rows.
fn editing_list(setup: &str, handler: &str) -> String {
    stateful(
        setup,
        &format!(
            r#"waterui.jsx("List", {{
                editing: true,
                {handler},
                children: waterui.jsx(waterui.For, {{
                    each: ["one", "two"],
                    children: (row) => waterui.jsx("Text", {{ children: row }}),
                }}),
            }})"#
        ),
    )
}

#[test]
fn ts_a_list_delete_control_reaches_the_javascript_handler() {
    // `editing` leaves no mark on the accessibility tree — the controls it
    // adds are chrome the list paints — so an equivalence test cannot tell an
    // arm that reads it from one that drops it, and neither can it tell
    // whether `onDelete` was wired to anything. Using the control can: there
    // is nothing at the row's trailing edge to press unless edit mode is on
    // *and* the list can delete, and what the press produces is the row.
    let module = Module::mount(&editing_list(
        "globalThis.deleted = null;",
        "onDelete: (index) => { globalThis.deleted = index; }",
    ));
    let mut app = module.app();
    let row = app.query().label("one").single().bounds();
    app.tap_at(
        row.x() + row.width() - TRAILING_CONTROL_PROBE,
        row.y() + row.height() / 2.0,
    );
    app.settle();
    assert_eq!(
        module.eval("globalThis.deleted").as_f64(),
        Some(0.0),
        "the delete control ran the JavaScript handler with the row's own index"
    );
}

#[test]
fn ts_a_list_move_control_reaches_the_javascript_handler() {
    // The reorder handle's lower half moves a row down one place, so the first
    // row's handle answers with the pair (0, 1) — which is also what says both
    // arguments arrive, and in the order the attribute declares them.
    let module = Module::mount(&editing_list(
        "globalThis.movedFrom = null; globalThis.movedTo = null;",
        "onMove: (from, to) => { globalThis.movedFrom = from; globalThis.movedTo = to; }",
    ));
    let mut app = module.app();
    let row = app.query().label("one").single().bounds();
    app.tap_at(
        row.x() + row.width() - TRAILING_CONTROL_PROBE,
        row.height().mul_add(0.75, row.y()),
    );
    app.settle();
    assert_eq!(
        (
            module.eval("globalThis.movedFrom").as_f64(),
            module.eval("globalThis.movedTo").as_f64()
        ),
        (Some(0.0), Some(1.0)),
        "the reorder handle ran the JavaScript handler with the row's old and new index"
    );
}

#[test]
fn ts_a_list_of_written_out_rows_is_refused() {
    let error = mount_error(&module(
        r#"waterui.jsx("List", { children: waterui.jsx("Text", { children: "one" }) })"#,
    ));
    assert!(
        error.contains("<For"),
        "the error points at the collection that can rebuild a row: {error}"
    );
}

#[test]
fn ts_table_is_a_table() {
    assert_same(
        "Table",
        &module(
            r#"waterui.jsx("Table", { children: [
                waterui.jsx("Column", { label: "Name", children: ["Ada", "Alan"] }),
                waterui.jsx("Column", { label: "Year", children: ["1815", "1912"] }),
            ] })"#,
        ),
        table(vec![
            col("Name", vec![text("Ada"), text("Alan")]),
            col("Year", vec![text("1815"), text("1912")]),
        ]),
    );
}

#[test]
fn ts_a_column_outside_a_table_is_refused() {
    let error = mount_error(&module(
        r#"waterui.jsx("VStack", { children: waterui.jsx("Table", {
            children: waterui.jsx("Text", { children: "not a column" }),
        }) })"#,
    ));
    assert!(
        error.contains("Column"),
        "the error names the child a table takes: {error}"
    );
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

#[test]
fn ts_navigation_stack_is_a_navigation_stack() {
    assert_same(
        "NavigationStack",
        &module(
            r#"waterui.jsx("NavigationStack", {
                title: "Inbox",
                children: waterui.jsx("Text", { children: "Nothing here" }),
            })"#,
        ),
        NavigationStack::new(NavigationView::new(
            "Inbox",
            vstack(
                HorizontalAlignment::Center,
                vec![AnyView::new(text("Nothing here"))],
            ),
        )),
    );
}

#[test]
fn ts_navigation_link_is_a_navigation_link() {
    assert_same(
        "NavigationLink",
        &module(
            r#"waterui.jsx("NavigationStack", {
                title: "Inbox",
                children: waterui.jsx("NavigationLink", {
                    title: "Message",
                    destination: () => waterui.jsx("Text", { children: "Body" }),
                    children: "Open",
                }),
            })"#,
        ),
        NavigationStack::new(NavigationView::new(
            "Inbox",
            vstack(
                HorizontalAlignment::Center,
                vec![AnyView::new(NavigationLink::new(label("Open"), || {
                    NavigationView::new("Message", text("Body"))
                }))],
            ),
        )),
    );
}

#[test]
fn ts_a_destination_is_built_again_every_time_it_is_entered() {
    // A view crosses the seam once; a destination does not. The builder is
    // called per build, so a second build produces a second view rather than
    // failing on a slot that was already emptied.
    let module = Module::mount(&stateful(
        "globalThis.builds = 0;",
        r#"waterui.jsx("NavigationStack", {
            children: waterui.jsx("NavigationLink", {
                destination: () => {
                    globalThis.builds += 1;
                    return waterui.jsx("Text", { children: "Body " + globalThis.builds });
                },
                children: "Open",
            }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Open").assert_exists();

    app.query().label("Open").tap();
    app.settle();
    app.query().label("Body 1").assert_exists();
}

#[test]
fn ts_a_navigation_links_title_heads_the_destination() {
    // The title is the destination's, so it is nowhere in the tree until the
    // link is followed — which is why `ts_navigation_link_is_a_navigation_link`
    // cannot see it and this one enters the destination.
    let module = Module::mount(&module(
        r#"waterui.jsx("NavigationStack", {
            title: "Inbox",
            children: waterui.jsx("NavigationLink", {
                title: "Message",
                destination: () => waterui.jsx("Text", { children: "Body" }),
                children: "Open",
            }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Message").assert_not_exists();

    app.query().label("Open").tap();
    app.settle();
    app.query().label("Body").assert_exists();
    app.query().label("Message").assert_exists();
}

#[test]
fn ts_tabs_is_a_tabs() {
    let selection = Binding::container(Str::from("inbox"));
    assert_same(
        "Tabs",
        &stateful(
            r#"globalThis.tab = waterui.createSignal("inbox");"#,
            r#"waterui.jsx("Tabs", { value: globalThis.tab, children: [
                waterui.jsx("Tab", {
                    value: "inbox",
                    title: "All messages",
                    content: () => waterui.jsx("Text", { children: "Messages" }),
                    children: "Inbox",
                }),
                waterui.jsx("Tab", {
                    value: "sent",
                    content: () => waterui.jsx("Text", { children: "Sent mail" }),
                    children: "Sent",
                }),
            ] })"#,
        ),
        Tabs::new(
            &selection,
            vec![
                Tab::new(Str::from("inbox"), label("Inbox"), || {
                    NavigationView::new("All messages", text("Messages"))
                }),
                Tab::new(Str::from("sent"), label("Sent"), || {
                    NavigationView::new("", text("Sent mail"))
                }),
            ],
        ),
    );
}

#[test]
fn ts_choosing_a_tab_writes_through_to_the_javascript_signal() {
    let module = Module::mount(&stateful(
        r#"globalThis.tab = waterui.createSignal("inbox");"#,
        r#"waterui.jsx("Tabs", { value: globalThis.tab, children: [
            waterui.jsx("Tab", {
                value: "inbox",
                content: () => waterui.jsx("Text", { children: "Messages" }),
                children: "Inbox",
            }),
            waterui.jsx("Tab", {
                value: "sent",
                content: () => waterui.jsx("Text", { children: "Sent mail" }),
                children: "Sent",
            }),
        ] })"#,
    ));
    let mut app = module.app();
    app.query().label("Messages").assert_exists();

    app.query().label("Sent").tap();
    app.settle();
    assert_eq!(
        module.eval("globalThis.tab()"),
        JsValue::String(String::from("sent")),
        "choosing a tab wrote its value into the JavaScript signal"
    );
    app.query().label("Sent mail").assert_exists();
    app.query().label("Messages").assert_not_exists();
}

// ---------------------------------------------------------------------------
// Modifiers
// ---------------------------------------------------------------------------

#[test]
fn ts_modifier_attributes_are_the_rust_chain() {
    assert_same(
        "padding then a width",
        &module(r#"waterui.jsx("Text", { padding: 12, width: 200, children: "Boxed" })"#),
        text("Boxed").padding_with(12.0).width(200.0),
    );
}

#[test]
fn ts_padding_takes_every_shape_the_rust_chain_takes() {
    assert_same(
        "padding by edges",
        &module(
            r#"waterui.jsx("Text", {
                padding: { horizontal: 16, top: 4 },
                children: "Inset",
            })"#,
        ),
        text("Inset").padding_with(EdgeInsets::new(4.0, 0.0, 16.0, 16.0)),
    );
    assert_same(
        "a bare padding",
        &module(r#"waterui.jsx("Text", { padding: true, children: "Inset" })"#),
        text("Inset").padding(),
    );
    assert_same(
        "a padding switched off",
        &module(r#"waterui.jsx("Text", { padding: false, children: "Flush" })"#),
        text("Flush").padding_with(EdgeInsets::all(0.0)),
    );
    // A zero inset and the framework's default inset are both `padding`, so the
    // equivalence above would also hold if `false` were read as `true`. This is
    // what tells the two apart.
    assert_ne!(
        Module::mount(&module(
            r#"waterui.jsx("Text", { padding: false, children: "Flush" })"#
        ))
        .tree(),
        Module::mount(&module(
            r#"waterui.jsx("Text", { padding: true, children: "Flush" })"#
        ))
        .tree(),
        "a padding of false insets nothing, and a bare padding insets by the default"
    );
}

#[test]
fn ts_one_padded_edge_follows_its_own_signal() {
    let module = Module::mount(&stateful(
        "globalThis.gap = waterui.createSignal(4);",
        r#"waterui.jsx("Text", {
            padding: { top: globalThis.gap, horizontal: 16 },
            a11yId: "inset",
            children: "Inset",
        })"#,
    ));
    let mut app = module.app();
    let before = app.query().identifier("inset").single().bounds();

    module.eval("globalThis.gap.set(40)");
    app.settle();
    let after = app.query().identifier("inset").single().bounds();
    assert_ne!(
        before, after,
        "a signal on one edge moves the view it insets"
    );
}

#[test]
fn ts_modifier_order_is_the_chain_order() {
    // A background is a view, so where it sits in the chain is visible: in the
    // first form it covers the padded box, in the second only the text.
    let padding_then_background = Module::mount(&module(
        r#"waterui.jsx("Text", {
            padding: 12,
            background: waterui.jsx("Text", { children: "behind" }),
            children: "Order",
        })"#,
    ))
    .tree();
    let background_then_padding = Module::mount(&module(
        r#"waterui.jsx("Text", {
            background: waterui.jsx("Text", { children: "behind" }),
            padding: 12,
            children: "Order",
        })"#,
    ))
    .tree();
    assert_ne!(
        padding_then_background, background_then_padding,
        "`padding → background` and `background → padding` are different views, as they are in Rust"
    );
    assert_eq!(
        padding_then_background,
        rust_tree(text("Order").padding_with(12.0).background(text("behind"))),
        "the written order is the chain order"
    );
    assert_eq!(
        background_then_padding,
        rust_tree(text("Order").background(text("behind")).padding_with(12.0)),
        "and so is the other one"
    );
}

/// A colour as the catalog's `{ red, green, blue }` object resolves it.
fn rgb(red: f32, green: f32, blue: f32) -> waterui::graphics::color::Color {
    waterui::graphics::color::Color::new(waterui::graphics::ResolvedColor {
        red,
        green,
        blue,
        headroom: 1.0,
        opacity: 1.0,
    })
}

// The purely visual modifiers — opacity, shadow, border, clip, foreground, a
// colour background — leave the accessibility tree exactly as it was, so a
// tree comparison could never tell an arm that reads one from an arm that
// drops it. Each is compared as the frame it renders instead, against the
// frame the Rust chain renders, one modifier at a time so a difference names
// the arm.

#[test]
fn ts_opacity_is_the_rust_opacity() {
    assert_same_pixels(
        "opacity",
        &Module::mount(&module(
            r#"waterui.jsx("Text", { opacity: 0.5, children: "Faded" })"#,
        ))
        .pixels(),
        &rust_pixels(text("Faded").opacity(0.5)),
    );
}

#[test]
fn ts_shadow_is_the_rust_shadow() {
    use waterui::graphics::color::Color;
    use waterui::style::{Shadow, Vector};
    assert_same_pixels(
        "shadow",
        &Module::mount(&module(
            r#"waterui.jsx("Text", { shadow: { radius: 4, y: 2 }, children: "Raised" })"#,
        ))
        .pixels(),
        &rust_pixels(text("Raised").shadow(Shadow::new(
            Color::srgb(0, 0, 0),
            Vector { x: 0.0, y: 2.0 },
            4.0,
            0.0,
        ))),
    );
}

#[test]
fn ts_border_is_the_rust_border() {
    use waterui::border::Border;
    assert_same_pixels(
        "border",
        &Module::mount(&module(
            r#"waterui.jsx("Text", {
                border: { color: { red: 0, green: 0, blue: 1 }, width: 2 },
                children: "Framed",
            })"#,
        ))
        .pixels(),
        &rust_pixels(text("Framed").border_with(Border::new(rgb(0.0, 0.0, 1.0), 2.0))),
    );
}

#[test]
fn ts_clip_is_the_rust_clip() {
    // A clip shows only where there is something to cut: a colour background
    // fills the text's box, and the shape takes its corners off.
    assert_same_pixels(
        "clip to a capsule",
        &Module::mount(&module(
            r#"waterui.jsx("Text", {
                padding: 12,
                background: { red: 0, green: 0, blue: 1 },
                clip: "Capsule",
                children: "Rounded",
            })"#,
        ))
        .pixels(),
        &rust_pixels(
            text("Rounded")
                .padding_with(12.0)
                .background(rgb(0.0, 0.0, 1.0))
                .clip(waterui::shape::Capsule),
        ),
    );
    assert_same_pixels(
        "clip by corner radius",
        &Module::mount(&module(
            r#"waterui.jsx("Text", {
                padding: 12,
                background: { red: 0, green: 0, blue: 1 },
                clip: { cornerRadius: 8 },
                children: "Rounded",
            })"#,
        ))
        .pixels(),
        &rust_pixels(
            text("Rounded")
                .padding_with(12.0)
                .background(rgb(0.0, 0.0, 1.0))
                .clip(waterui::shape::RoundedRectangle::new(8.0)),
        ),
    );
}

#[test]
fn ts_foreground_is_the_rust_foreground() {
    assert_same_pixels(
        "foreground",
        &Module::mount(&module(
            r#"waterui.jsx("Text", {
                foreground: { red: 1, green: 0, blue: 0 },
                children: "Tinted",
            })"#,
        ))
        .pixels(),
        &rust_pixels(text("Tinted").foreground(rgb(1.0, 0.0, 0.0))),
    );
}

#[test]
fn ts_background_takes_a_colour() {
    assert_same_pixels(
        "a colour background",
        &Module::mount(&module(
            r#"waterui.jsx("Text", {
                background: { red: 1, green: 0, blue: 0 },
                children: "Tinted",
            })"#,
        ))
        .pixels(),
        &rust_pixels(text("Tinted").background(rgb(1.0, 0.0, 0.0))),
    );
}

#[test]
fn ts_frame_bounds_are_the_rust_frame() {
    assert_same(
        "a height",
        &module(r#"waterui.jsx("Text", { height: 44, children: "Tall" })"#),
        text("Tall").height(44.0),
    );
    assert_same(
        "four bounds on one frame",
        &module(
            r#"waterui.jsx("Text", {
                minWidth: 80,
                maxWidth: 200,
                minHeight: 20,
                maxHeight: 60,
                children: "Bounded",
            })"#,
        ),
        text("Bounded")
            .min_width(80.0)
            .max_width(200.0)
            .min_height(20.0)
            .max_height(60.0),
    );
}

#[test]
fn ts_a_hidden_view_is_hidden_the_way_rust_hides_it() {
    assert_same(
        "a11yHidden",
        &module(r#"waterui.jsx("Text", { a11yHidden: true, children: "Decorative" })"#),
        text("Decorative").a11y_hidden(true),
    );
}

#[test]
fn ts_on_tap_gesture_is_the_rust_gesture() {
    assert_same(
        "onTapGesture",
        &module(r#"waterui.jsx("Text", { onTapGesture: () => {}, children: "Tap me" })"#),
        text("Tap me").on_tap_gesture(|| {}),
    );
}

#[test]
fn ts_a_tap_reaches_the_javascript_handler() {
    // The equivalence above says the tree is the same; this says the handler on
    // the other side of the seam is the one that runs.
    let module = Module::mount(&stateful(
        "globalThis.taps = 0;",
        r#"waterui.jsx("Button", {
            onTap: () => {},
            onTapGesture: () => { globalThis.taps += 1; },
            children: "Tap me",
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Tap me").tap();
    app.settle();
    assert_eq!(
        module.eval("globalThis.taps").as_f64(),
        Some(1.0),
        "the gesture ran the JavaScript handler once"
    );
}

#[test]
fn ts_accessibility_attributes_reach_the_tree() {
    assert_same(
        "a11y attributes",
        &module(
            r#"waterui.jsx("Text", {
                a11yLabel: "Progress",
                a11yValue: "Halfway",
                a11yId: "progress",
                children: "50%",
            })"#,
        ),
        text("50%")
            .a11y_label(Str::from("Progress"))
            .a11y_value(Str::from("Halfway"))
            .a11y_id("progress"),
    );
}

#[test]
fn ts_disabled_disables_a_subtree_the_way_rust_does() {
    // `disabled` is three things at once — an environment scope, an
    // accessibility state and a hit-test switch — so a subtree that is not a
    // control is where a partial implementation shows.
    assert_same(
        "a disabled stack",
        &module(
            r#"waterui.jsx("VStack", { disabled: true, children: [
                waterui.jsx("Text", { children: "Read only" }),
                waterui.jsx("Button", { onTap: () => {}, children: "Send" }),
            ] })"#,
        ),
        vstack(
            HorizontalAlignment::Center,
            vec![
                AnyView::new(text("Read only")),
                AnyView::new(button("Send").action(|| {})),
            ],
        )
        .disabled(true),
    );
}

// ---------------------------------------------------------------------------
// Reactivity across the seam
// ---------------------------------------------------------------------------

#[test]
fn ts_a_javascript_signal_drives_the_native_control() {
    let module = Module::mount(&stateful(
        "globalThis.on = waterui.createSignal(false);",
        r#"waterui.jsx("Toggle", { value: globalThis.on, children: "Wi-Fi" })"#,
    ));
    let mut app = module.app();
    app.query().label("Wi-Fi").checked(false).assert_exists();

    // The control writes through the materialized binding, and JavaScript sees
    // it: the signal is the state, not a copy of it.
    app.query().label("Wi-Fi").tap();
    app.settle();
    assert_eq!(module.eval("globalThis.on()"), JsValue::Bool(true));
}

#[test]
fn ts_a_reactive_child_updates_in_place() {
    let module = Module::mount(&stateful(
        "globalThis.count = waterui.createSignal(1);",
        r#"waterui.jsx("VStack", {
            children: () => waterui.jsx("Text", { children: "Count: " + globalThis.count() }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Count: 1").assert_exists();

    module.eval("globalThis.count.set(7)");
    app.settle();
    app.query().label("Count: 7").assert_exists();
    app.query().label("Count: 1").assert_not_exists();
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn ts_show_presents_one_branch_at_a_time() {
    let module = Module::mount(&stateful(
        "globalThis.signedIn = waterui.createSignal(false);",
        r#"waterui.jsx(waterui.Show, {
            when: globalThis.signedIn,
            fallback: () => waterui.jsx("Text", { children: "Signed out" }),
            children: () => waterui.jsx("Text", { children: "Signed in" }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Signed out").assert_exists();

    module.eval("globalThis.signedIn.set(true)");
    app.settle();
    app.query().label("Signed in").assert_exists();
    app.query().label("Signed out").assert_not_exists();
}

#[test]
fn ts_for_reconciles_by_key() {
    let module = Module::mount(&stateful(
        r#"globalThis.items = waterui.createSignal(["one", "two"]);"#,
        r#"waterui.jsx("VStack", {
            children: waterui.jsx(waterui.For, {
                each: globalThis.items,
                children: (item) => waterui.jsx("Text", { children: item }),
            }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("one").assert_exists();
    app.query().label("two").assert_exists();

    module.eval(r#"globalThis.items.set(["two", "three"])"#);
    app.settle();
    app.query().label("two").assert_exists();
    app.query().label("three").assert_exists();
    app.query().label("one").assert_not_exists();
}

#[test]
fn ts_two_items_with_one_key_are_refused() {
    let error = mount_error(&module(
        r#"waterui.jsx("VStack", {
            children: waterui.jsx(waterui.For, {
                each: ["a", "a"],
                children: (item) => waterui.jsx("Text", { children: item }),
            }),
        })"#,
    ));
    assert!(
        error.contains("key"),
        "the error says two rows cannot share one key: {error}"
    );
}

#[test]
fn ts_two_whole_numbers_past_the_integer_range_are_two_rows() {
    // Both are whole and both are far past `i64::MAX`, so a key taken by
    // casting saturates them to one integer and the list above refuses them as
    // two rows sharing a key — a list JavaScript tells apart, reported as an
    // authoring mistake nobody made. Keyed by their bits they are the two rows
    // they are.
    let module = Module::mount(&module(
        r#"waterui.jsx("VStack", {
            children: waterui.jsx(waterui.For, {
                each: [1e21, 2e21],
                children: (item) => waterui.jsx("Text", { children: `row ${item}` }),
            }),
        })"#,
    ));
    let tree = module.tree();
    assert!(
        tree.contains("row 1e+21") && tree.contains("row 2e+21"),
        "both rows are in the tree: {tree}"
    );
}

#[test]
fn ts_a_departed_row_releases_what_it_exported() {
    // Every `<For>` row exports an index accessor. Owned by the mount, those
    // would pile up one per departed row for as long as the module lived; owned
    // by the row, they go with it.
    let module = Module::mount(&stateful(
        r#"globalThis.items = waterui.createSignal(["a"]);"#,
        r#"waterui.jsx("VStack", {
            children: waterui.jsx(waterui.For, {
                each: globalThis.items,
                children: (item) => waterui.jsx("Text", { children: item }),
            }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("a").assert_exists();
    let baseline = module.bridge().exported_count();

    for round in 0..8 {
        module.eval(&format!(r#"globalThis.items.set(["row {round}"])"#));
        app.settle();
    }
    app.query().label("row 7").assert_exists();
    assert_eq!(
        module.bridge().exported_count(),
        baseline,
        "a list that churns leaves nothing behind in the mount's scope"
    );
}

#[test]
fn ts_a_departed_row_releases_what_its_branch_exported() {
    // A row's render callback exports too: a `<Show>` inside it materializes
    // `when` and exports the accessor its branch is handed. Those belong to the
    // branch the callback built, not to the mount — otherwise every row that
    // leaves strands a cell, a JavaScript memo and a dedup entry that live as
    // long as the module does.
    let module = Module::mount(&stateful(
        r#"globalThis.items = waterui.createSignal(["a"]);
           globalThis.live = waterui.createSignal(true);"#,
        r#"waterui.jsx("VStack", {
            children: waterui.jsx(waterui.For, {
                each: globalThis.items,
                children: (item) => waterui.jsx(waterui.Show, {
                    when: globalThis.live,
                    children: () => waterui.jsx("Text", { children: item }),
                }),
            }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("a").assert_exists();
    let baseline = module.bridge().exported_count();

    for round in 0..8 {
        module.eval(&format!(r#"globalThis.items.set(["row {round}"])"#));
        app.settle();
    }
    app.query().label("row 7").assert_exists();
    assert_eq!(
        module.bridge().exported_count(),
        baseline,
        "a row whose content exports releases those exports when the row leaves"
    );
}

#[test]
fn ts_a_replaced_branch_releases_what_it_built() {
    // The inner `<Show>` is built afresh every time the outer one activates,
    // and exports its own `when` accessor each time. The outgoing branch owns
    // the previous set, so replacing it releases them.
    let module = Module::mount(&stateful(
        r"globalThis.outer = waterui.createSignal(true);
           globalThis.inner = waterui.createSignal(true);",
        r#"waterui.jsx(waterui.Show, {
            when: globalThis.outer,
            fallback: () => waterui.jsx("Text", { children: "away" }),
            children: () => waterui.jsx(waterui.Show, {
                when: globalThis.inner,
                children: () => waterui.jsx("Text", { children: "here" }),
            }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("here").assert_exists();
    let baseline = module.bridge().exported_count();

    for _ in 0..8 {
        module.eval("globalThis.outer.set(false)");
        app.settle();
        module.eval("globalThis.outer.set(true)");
        app.settle();
    }
    app.query().label("here").assert_exists();
    assert_eq!(
        module.bridge().exported_count(),
        baseline,
        "a branch replaced eight times leaves eight sets of exports behind, or none"
    );
}

#[test]
fn ts_a_rebuilt_destination_releases_what_the_last_build_exported() {
    // `ViewBuilder::build` runs again every time the destination is entered,
    // and each build's render function exports afresh — here a `<Show>`'s `when`
    // accessor. A build's exports belong to the subtree it produced, so a
    // destination entered and left eight times does not leave eight sets in the
    // mount's scope.
    let module = Module::mount(&stateful(
        r"globalThis.builds = 0;
           globalThis.live = waterui.createSignal(true);",
        r#"waterui.jsx("NavigationStack", {
            children: waterui.jsx("NavigationLink", {
                destination: () => {
                    globalThis.builds += 1;
                    return waterui.jsx(waterui.Show, {
                        when: globalThis.live,
                        children: () => waterui.jsx("Text", { children: "Body" }),
                    });
                },
                children: "Open",
            }),
        })"#,
    ));
    let mut app = module.app();
    app.query().label("Open").assert_exists();
    let baseline = module.bridge().exported_count();

    for _ in 0..8 {
        app.query().label("Open").tap();
        app.settle();
        app.query().label("Body").assert_exists();
        app.query().label("Back").tap();
        app.settle();
    }
    app.query().label("Open").assert_exists();
    assert_eq!(
        module.eval("globalThis.builds").as_f64(),
        Some(8.0),
        "the destination is built once per entry, which is what leaves exports to release"
    );
    assert_eq!(
        module.bridge().exported_count(),
        baseline,
        "a destination rebuilt on every entry releases the previous build's exports"
    );
}

// ---------------------------------------------------------------------------
// What the catalog refuses
// ---------------------------------------------------------------------------

#[test]
fn ts_a_component_the_catalog_does_not_declare_is_refused() {
    let error = mount_error(&module(r#"waterui.jsx("Carousel", {})"#));
    assert!(
        error.contains("Carousel") && error.contains("VStack"),
        "the error names the component and what the catalog does declare: {error}"
    );
}

#[test]
fn ts_an_attribute_the_component_does_not_declare_is_refused() {
    // A dropped attribute is worse than a refused one: `onClick` on a button
    // renders a button that does nothing, and nothing says why.
    let error = mount_error(&module(
        r#"waterui.jsx("Button", { onClick: () => {}, children: "Send" })"#,
    ));
    assert!(
        error.contains("onClick") && error.contains("onTap"),
        "the error names the attribute and what the component accepts: {error}"
    );
}

#[test]
fn ts_an_attribute_value_of_the_wrong_shape_is_refused() {
    // Several attributes leave no mark on the accessibility tree — an
    // avatar's picture, a spacer's minimum, a link's target, a list's editing
    // mode — and some no mark on the frame either, so an equivalence test
    // cannot tell an arm that reads them from one that drops them. A refusal
    // can: an arm that never looks at the value cannot reject it.
    for (case, source, expected) in [
        (
            "a shape the catalog does not name",
            r#"waterui.jsx("Text", { clip: "Squircle", children: "x" })"#,
            "Squircle",
        ),
        (
            "a rounded clip with no corner radius",
            r#"waterui.jsx("Text", { clip: { radius: 8 }, children: "x" })"#,
            "cornerRadius",
        ),
        (
            "an editing flag that is not a flag",
            r#"waterui.jsx("List", {
                editing: "yes",
                children: waterui.jsx(waterui.For, {
                    each: ["a"],
                    children: (row) => waterui.jsx("Text", { children: row }),
                }),
            })"#,
            "editing",
        ),
        (
            "a delete handler that is not a handler",
            r#"waterui.jsx("List", {
                onDelete: 3,
                children: waterui.jsx(waterui.For, {
                    each: ["a"],
                    children: (row) => waterui.jsx("Text", { children: row }),
                }),
            })"#,
            "onDelete",
        ),
        (
            "a move handler that is not a handler",
            r#"waterui.jsx("List", {
                onMove: "soon",
                children: waterui.jsx(waterui.For, {
                    each: ["a"],
                    children: (row) => waterui.jsx("Text", { children: row }),
                }),
            })"#,
            "onMove",
        ),
        (
            "an avatar picture that is not a URL",
            r#"waterui.jsx("Avatar", { name: "Ada", image: "not a url" })"#,
            "image",
        ),
        (
            "a spacer minimum that is not a length",
            r#"waterui.jsx("Spacer", { minLength: "wide" })"#,
            "minLength",
        ),
        (
            "a link target that is not a URL string",
            r#"waterui.jsx("Link", { url: 3, children: "WaterUI" })"#,
            "url",
        ),
        (
            "a prompt that is not text",
            r#"waterui.jsx("TextField", {
                value: waterui.createSignal("a"),
                prompt: {},
                children: "Name",
            })"#,
            "prompt",
        ),
    ] {
        let error = mount_error(&module(source));
        assert!(
            error.contains(expected),
            "{case}: the error names what was wrong, found: {error}"
        );
    }
}

#[test]
fn ts_a_loading_progress_cannot_also_count() {
    let error = mount_error(&module(
        r#"waterui.jsx("Progress", { value: 1, total: 5, style: "Loading", children: "Files" })"#,
    ));
    assert!(
        error.contains("Loading") && error.contains("total"),
        "the error says an indeterminate bar has nothing to count out of: {error}"
    );
}

#[test]
fn ts_a_control_without_a_label_is_refused() {
    let error = mount_error(&module(r#"waterui.jsx("Button", { onTap: () => {} })"#));
    assert!(
        error.contains("Button") && error.contains("label"),
        "the error says a control names itself: {error}"
    );
}

//! Every catalog entry, mounted twice: once as JSX and once as the Rust view
//! it claims to be, with the two accessibility trees compared.
//!
//! That comparison is what makes the catalog a binding rather than a
//! lookalike. A TypeScript `<Toggle>` is not "close enough" to a Rust
//! `toggle(…)` — it *is* one, so anything a binding drops on the way (a label,
//! an action, a value, a bound) shows up here as a difference in the tree
//! rather than as a screen nobody can use.

mod support;

use nami::Binding;
use suiteki::Str;
use waterui::ViewExt as _;
use waterui_controls::{button, field, label, slider::slider, stepper::stepper, toggle};
use waterui_core::AnyView;
use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};
use waterui_form::picker::{Picker, PickerItem};
use waterui_graphics::ResolvedColor;
use waterui_graphics::color::Color;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::{HStack, VStack, ZStack};
use waterui_layout::{Divider, ScrollView, Spacer};
use waterui_text::text;
use waterui_ts::engine::JsValue;

use support::{Module, mount_error, rust_tree};

/// The gap a stack with no `spacing` leaves, which is what `VStack::new`
/// documents and what the host passes.
const DEFAULT_SPACING: f32 = 10.0;

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
fn ts_hstack_is_an_hstack() {
    assert_same(
        "HStack",
        &module(
            r#"waterui.jsx("HStack", {
                spacing: 4,
                alignment: "Top",
                children: waterui.jsx("Text", { children: "Only" }),
            })"#,
        ),
        HStack::new(
            VerticalAlignment::Top,
            4.0,
            vec![AnyView::new(text("Only"))],
        ),
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
        ScrollView::vertical(VStack::new(
            HorizontalAlignment::Center,
            DEFAULT_SPACING,
            vec![AnyView::new(text("Scrolled"))],
        )),
    );
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
        VStack::new(
            HorizontalAlignment::Center,
            DEFAULT_SPACING,
            vec![
                AnyView::new(text("Above")),
                AnyView::new(Spacer::new(0.0)),
                AnyView::new(Divider),
            ],
        ),
    );
}

#[test]
fn ts_label_is_a_label() {
    assert_same(
        "Label",
        &module(r#"waterui.jsx("Label", { children: "Named" })"#),
        label("Named"),
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
    let count = Binding::container(2_i32);
    assert_same(
        "Stepper",
        &stateful(
            "globalThis.count = waterui.createSignal(2);",
            r#"waterui.jsx("Stepper", { value: globalThis.count, step: 2, children: "Guests" })"#,
        ),
        stepper("Guests", &count).step(2),
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

#[test]
fn ts_background_takes_a_colour() {
    assert_same(
        "a colour background",
        &module(
            r#"waterui.jsx("Text", {
                background: { red: 1, green: 0, blue: 0 },
                children: "Tinted",
            })"#,
        ),
        text("Tinted").background(Color::new(ResolvedColor {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            headroom: 1.0,
            opacity: 1.0,
        })),
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
fn ts_disabled_disables_the_subtree() {
    assert_same(
        "disabled",
        &module(r#"waterui.jsx("Button", { disabled: true, onTap: () => {}, children: "Send" })"#),
        button("Send").action(|| {}).disabled(true),
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
fn ts_a_control_without_a_label_is_refused() {
    let error = mount_error(&module(r#"waterui.jsx("Button", { onTap: () => {} })"#));
    assert!(
        error.contains("Button") && error.contains("label"),
        "the error says a control names itself: {error}"
    );
}

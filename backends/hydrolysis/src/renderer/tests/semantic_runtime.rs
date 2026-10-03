//! Comprehensive [`SemanticRuntime`] coverage: every widget family mounts on a
//! bare [`Environment`] — no style package, no GPU — and the emitted AccessKit
//! tree is asserted for roles, labels, values, supported actions and
//! parent–child structure, while `perform_accessibility_action` is asserted to
//! mutate the bound state each action targets.
//!
//! These cannot run until `waterui-testing` (a dev-dependency every test
//! target links) carries the `waterui#1130` driver side; they are written
//! against the pinned waterui API so they compile and run unchanged after the
//! repin.

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, Toggled, TreeId, TreeUpdate,
};
use nami::Binding;
use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityState;
use waterui::component::list::{List, ListItem};
use waterui::component::progress::progress;
use waterui::component::table::{col, table};
use waterui::theme::color::Error;
use waterui_controls::button::button;
use waterui_controls::menu::{CommandExt as _, Menu};
use waterui_controls::slider::slider;
use waterui_controls::stepper::stepper;
use waterui_controls::text_field::field;
use waterui_controls::toggle::toggle;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment, Str};
use waterui_form::picker::color::ColorPicker;
use waterui_form::picker::date::DatePicker;
use waterui_form::picker::{PickerStyle, picker};
use waterui_form::secure::secure;
use waterui_graphics::Color;
use waterui_graphics::cherenkov::Recorder;
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::{GpuContent, GpuContentView, RecordingResources, SceneContent, SceneView};
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{VStackLayout, hstack, vstack};
use waterui_layout::{Divider, LazyContainer, scroll};
use waterui_navigation::NavigationView;
use waterui_navigation::tab::{Tab, Tabs};
use waterui_navigation::{NavigationLink, NavigationSplitView, NavigationStack};
use waterui_text::text;

use crate::runner::SemanticRuntime;
use crate::{InputEvent, KeyCode, KeyState, Modifiers, keyboard_types};

/// Mounts `content` as the single window of a semantic runtime over a bare
/// environment — `Environment::new()`, nothing installed. The runtime itself
/// seeds the framework tokens, fonts, native hooks and window managers a
/// semantic tree needs; a style is never involved.
fn mount(builder: AnyViewBuilder<AnyView>) -> SemanticRuntime {
    SemanticRuntime::new_for_tests(Environment::new(), builder, 800, 600)
}

/// Pumps until the runtime settles and returns the merged tree as of that
/// settle — `None` when no window has ever produced one.
fn pump_until_settled(runtime: &mut SemanticRuntime) -> Option<TreeUpdate> {
    for _ in 0..64 {
        let _ = runtime.pump();
        if runtime.is_settled() {
            break;
        }
    }
    assert!(
        !runtime.has_pending_semantic_update(),
        "semantic runtime never settled"
    );
    runtime.accessibility_tree()
}

fn pumped(runtime: &mut SemanticRuntime) -> TreeUpdate {
    pump_until_settled(runtime).expect("the pump emitted no tree update")
}

/// The node labelled `label` with `role`, if present.
fn find_by_label<'a>(
    update: &'a TreeUpdate,
    role: Role,
    label: &str,
) -> Option<(NodeId, &'a Node)> {
    update.nodes.iter().find_map(|(id, node)| {
        (node.role() == role && node.label() == Some(label)).then_some((*id, node))
    })
}

/// The single node with `role`, if exactly one exists.
fn find_only(update: &TreeUpdate, role: Role) -> Option<(NodeId, &Node)> {
    let mut matches = update.nodes.iter().filter(|(_, node)| node.role() == role);
    let only = matches.next().map(|(id, node)| (*id, node));
    assert!(
        matches.next().is_none(),
        "more than one {role:?} node in the semantic tree"
    );
    only
}

fn lookup(update: &TreeUpdate, id: NodeId) -> &Node {
    update
        .nodes
        .iter()
        .find_map(|(node_id, node)| (*node_id == id).then_some(node))
        .unwrap_or_else(|| panic!("node {id:?} is not in the semantic tree"))
}

fn act(runtime: &mut SemanticRuntime, action: Action, target: NodeId) -> bool {
    runtime.perform_accessibility_action(ActionRequest {
        action,
        target_node: target,
        target_tree: TreeId::ROOT,
        data: None,
    })
}

fn act_with_data(
    runtime: &mut SemanticRuntime,
    action: Action,
    target: NodeId,
    data: ActionData,
) -> bool {
    runtime.perform_accessibility_action(ActionRequest {
        action,
        target_node: target,
        target_tree: TreeId::ROOT,
        data: Some(data),
    })
}

/// Asserts the update carries a real tree: a root node whose children hold
/// every other node.
fn assert_rooted(update: &TreeUpdate) {
    let tree = update
        .tree
        .as_ref()
        .expect("the semantic update must carry its tree");
    let root = lookup(update, tree.root);
    assert_eq!(root.role(), Role::Window);
    let mut reachable: Vec<NodeId> = root.children().to_vec();
    for (id, node) in &update.nodes {
        if *id != tree.root {
            reachable.extend_from_slice(node.children());
        }
    }
    for (id, _) in &update.nodes {
        assert!(
            *id == tree.root || reachable.contains(id),
            "node {id:?} is not reachable from the tree root"
        );
    }
}

#[test]
fn text_and_button_emit_and_click_fires() {
    let fired = Binding::container(false);
    let fired_for_button = fired.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let fired = fired_for_button.clone();
        AnyView::new(vstack((
            text("hello semantic"),
            button("Tap").action(move || fired.set(true)),
        )))
    }));

    let update = pumped(&mut runtime);
    assert_rooted(&update);
    let (_, text_node) =
        find_only(&update, Role::Label).expect("the text view must emit a Label node");
    assert_eq!(text_node.label(), Some("hello semantic"));
    let (tap, tap_node) =
        find_by_label(&update, Role::Button, "Tap").expect("the Tap button is missing");
    assert!(tap_node.supports_action(Action::Focus));
    assert!(tap_node.supports_action(Action::Click));

    assert!(
        act(&mut runtime, Action::Click, tap),
        "Click changed nothing"
    );
    assert!(fired.snapshot(), "the button action did not fire");
    let update = pumped(&mut runtime);
    assert_rooted(&update);
}

/// A view coloured with the `Error` token mounts on the bare semantic runtime:
/// the framework defaults install the token, so resolving it needs no `Style`
/// backfill.
#[test]
fn error_token_foreground_resolves_against_framework_defaults() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((text("something failed").foreground(Error),)))
    }));

    let update = pumped(&mut runtime);
    assert_rooted(&update);
    let (_, text_node) = find_by_label(&update, Role::Label, "something failed")
        .expect("the Error-foreground text must emit");
    assert_eq!(text_node.label(), Some("something failed"));
}

#[test]
fn toggle_emits_and_click_flips() {
    let on = Binding::container(false);
    let on_for_view = on.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((toggle("Airplane mode", &on_for_view),)))
    }));

    let update = pumped(&mut runtime);
    let (switch, switch_node) =
        find_by_label(&update, Role::Switch, "Airplane mode").expect("the toggle is missing");
    assert_eq!(switch_node.toggled(), Some(Toggled::False));
    assert!(switch_node.supports_action(Action::Click));
    assert!(switch_node.supports_action(Action::Focus));

    assert!(
        act(&mut runtime, Action::Click, switch),
        "Click changed nothing"
    );
    assert!(on.snapshot(), "the toggle binding did not flip");

    let update = pumped(&mut runtime);
    let (_, switch_node) = find_by_label(&update, Role::Switch, "Airplane mode")
        .expect("the toggle vanished after its state changed");
    assert_eq!(switch_node.toggled(), Some(Toggled::True));
}

#[test]
fn slider_emits_and_value_actions_step() {
    let value = Binding::container(0.5f64);
    let value_for_view = value.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((slider("Volume", &value_for_view),)))
    }));

    let update = pumped(&mut runtime);
    let (slider_id, slider_node) =
        find_by_label(&update, Role::Slider, "Volume").expect("the slider is missing");
    assert_eq!(slider_node.numeric_value(), Some(0.5));
    assert_eq!(slider_node.min_numeric_value(), Some(0.0));
    assert_eq!(slider_node.max_numeric_value(), Some(1.0));
    for action in [
        Action::Focus,
        Action::Increment,
        Action::Decrement,
        Action::SetValue,
    ] {
        assert!(
            slider_node.supports_action(action),
            "the slider must advertise {action:?}"
        );
    }

    assert!(
        act(&mut runtime, Action::Increment, slider_id),
        "Increment changed nothing"
    );
    assert_eq!(value.snapshot(), 0.51, "Increment did not step the binding");
    assert!(
        act(&mut runtime, Action::Decrement, slider_id),
        "Decrement changed nothing"
    );
    assert_eq!(value.snapshot(), 0.5, "Decrement did not step the binding");
    assert!(
        act_with_data(
            &mut runtime,
            Action::SetValue,
            slider_id,
            ActionData::NumericValue(0.25),
        ),
        "SetValue changed nothing"
    );
    assert_eq!(value.snapshot(), 0.25, "SetValue did not write the binding");

    let update = pumped(&mut runtime);
    let (_, slider_node) = find_by_label(&update, Role::Slider, "Volume")
        .expect("the slider vanished after its value changed");
    assert_eq!(slider_node.numeric_value(), Some(0.25));
}

#[test]
fn stepper_emits_and_value_actions_step() {
    let value = Binding::container(3i32);
    let value_for_view = value.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((stepper("Quantity", &value_for_view),)))
    }));

    let update = pumped(&mut runtime);
    let (stepper_id, stepper_node) =
        find_by_label(&update, Role::SpinButton, "Quantity").expect("the stepper is missing");
    assert_eq!(stepper_node.numeric_value(), Some(3.0));
    assert_eq!(stepper_node.numeric_value_step(), Some(1.0));
    for action in [
        Action::Focus,
        Action::Increment,
        Action::Decrement,
        Action::SetValue,
    ] {
        assert!(
            stepper_node.supports_action(action),
            "the stepper must advertise {action:?}"
        );
    }

    assert!(
        act(&mut runtime, Action::Increment, stepper_id),
        "Increment changed nothing"
    );
    assert_eq!(value.snapshot(), 4, "Increment did not step the binding");
    assert!(
        act(&mut runtime, Action::Decrement, stepper_id),
        "Decrement changed nothing"
    );
    assert_eq!(value.snapshot(), 3, "Decrement did not step the binding");
    assert!(
        act_with_data(
            &mut runtime,
            Action::SetValue,
            stepper_id,
            ActionData::NumericValue(7.0),
        ),
        "SetValue changed nothing"
    );
    assert_eq!(value.snapshot(), 7, "SetValue did not write the binding");
}

#[test]
fn progress_emits_its_value() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((progress(0.4),)))
    }));

    let update = pumped(&mut runtime);
    let (_, progress_node) =
        find_only(&update, Role::ProgressIndicator).expect("the progress view is missing");
    assert_eq!(progress_node.min_numeric_value(), Some(0.0));
    assert_eq!(progress_node.max_numeric_value(), Some(1.0));
    assert_eq!(progress_node.numeric_value(), Some(0.4));
}

#[test]
fn text_field_emits_and_set_value_edits() {
    let value = Binding::container(Str::default());
    let value_for_view = value.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((field("Name", &value_for_view),)))
    }));

    let update = pumped(&mut runtime);
    let (field_id, field_node) =
        find_only(&update, Role::TextInput).expect("the text field is missing");
    assert_eq!(field_node.label(), Some("Name"));
    assert!(field_node.supports_action(Action::Focus));
    assert!(field_node.supports_action(Action::SetValue));

    assert!(
        act(&mut runtime, Action::Focus, field_id),
        "Focus changed nothing"
    );
    let _ = pumped(&mut runtime);
    assert_eq!(
        runtime.focused_ui_node(),
        Some(field_id),
        "the text field did not take UI focus"
    );

    assert!(
        act_with_data(
            &mut runtime,
            Action::SetValue,
            field_id,
            ActionData::Value("Ada".into()),
        ),
        "SetValue changed nothing"
    );
    assert_eq!(
        value.snapshot().to_string().as_str(),
        "Ada",
        "SetValue did not write the binding"
    );
    let update = pumped(&mut runtime);
    let (_, field_node) = find_only(&update, Role::TextInput).expect("the text field vanished");
    assert_eq!(field_node.value(), Some("Ada"));
}

#[test]
fn secure_field_emits_and_set_value_edits() {
    let secret = Binding::container(waterui_form::secure::Secure::new(String::new()));
    let secret_for_view = secret.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((secure("Password", &secret_for_view),)))
    }));

    let update = pumped(&mut runtime);
    let (field_id, field_node) =
        find_only(&update, Role::PasswordInput).expect("the secure field is missing");
    assert_eq!(field_node.label(), Some("Password"));
    assert!(field_node.supports_action(Action::SetValue));

    assert!(
        act_with_data(
            &mut runtime,
            Action::SetValue,
            field_id,
            ActionData::Value("hunter2".into()),
        ),
        "SetValue changed nothing"
    );
    assert_eq!(
        secret.snapshot().expose(),
        "hunter2",
        "SetValue did not write the secure binding"
    );
}

#[test]
fn menu_picker_emits_options_and_selects() {
    let selection = Binding::container(0i32);
    let selection_for_view = selection.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((picker(
            "Size",
            vec![text("Small").tag(0i32), text("Large").tag(1i32)],
            &selection_for_view,
        )
        .style(PickerStyle::Menu),)))
    }));

    let update = pumped(&mut runtime);
    let (_, combo_node) =
        find_by_label(&update, Role::ComboBox, "Size").expect("the picker is missing");
    assert_eq!(combo_node.value(), Some("Small"));
    assert!(combo_node.supports_action(Action::Click));
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Size"))
            .count(),
        1,
        "the picker's label must name its single accessibility node exactly once"
    );
    let children = combo_node.children();
    assert_eq!(
        children.len(),
        2,
        "a menu picker's options are its semantic children"
    );
    let small = lookup(&update, children[0]);
    let large = lookup(&update, children[1]);
    assert_eq!(small.role(), Role::ListBoxOption);
    assert_eq!(small.label(), Some("Small"));
    assert_eq!(small.is_selected(), Some(true));
    assert_eq!(large.role(), Role::ListBoxOption);
    assert_eq!(large.label(), Some("Large"));
    assert_eq!(large.is_selected(), Some(false));
    assert!(large.supports_action(Action::Click));

    assert!(
        act(&mut runtime, Action::Click, children[1]),
        "the option Click changed nothing"
    );
    assert_eq!(selection.snapshot(), 1, "the option did not select");
    let update = pumped(&mut runtime);
    let (_, combo_node) =
        find_by_label(&update, Role::ComboBox, "Size").expect("the picker vanished");
    assert_eq!(combo_node.value(), Some("Large"));
    let children = combo_node.children();
    assert_eq!(lookup(&update, children[0]).is_selected(), Some(false));
    assert_eq!(lookup(&update, children[1]).is_selected(), Some(true));
}

/// A visually hidden menu-picker label draws nothing and takes no space, but
/// the picker's accessibility node still carries the label — exactly once.
#[test]
fn menu_picker_hidden_label_still_names_the_combo() {
    let selection = Binding::container(0i32);
    let selection_for_view = selection;
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((picker(
            "Size",
            vec![text("Small").tag(0i32), text("Large").tag(1i32)],
            &selection_for_view,
        )
        .style(PickerStyle::Menu)
        .hide_label(),)))
    }));

    let update = pumped(&mut runtime);
    let (_, combo_node) = find_by_label(&update, Role::ComboBox, "Size")
        .expect("a hidden label still names the picker");
    assert_eq!(combo_node.value(), Some("Small"));
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Size"))
            .count(),
        1,
        "a hidden label must not add a second node carrying the label"
    );
}

#[test]
fn radio_picker_emits_group_and_selects() {
    let selection = Binding::container(0i32);
    let selection_for_view = selection.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((picker(
            "Mode",
            vec![text("Auto").tag(0i32), text("Manual").tag(1i32)],
            &selection_for_view,
        )
        .style(PickerStyle::Radio),)))
    }));

    let update = pumped(&mut runtime);
    let (_, group_node) =
        find_by_label(&update, Role::Group, "Mode").expect("the radio group is missing");
    let children = group_node.children();
    assert_eq!(children.len(), 2, "the group must hold one node per option");
    let auto = lookup(&update, children[0]);
    let manual = lookup(&update, children[1]);
    assert_eq!(auto.role(), Role::RadioButton);
    assert_eq!(auto.label(), Some("Auto"));
    assert_eq!(auto.is_selected(), Some(true));
    assert_eq!(manual.role(), Role::RadioButton);
    assert_eq!(manual.label(), Some("Manual"));
    assert_eq!(manual.is_selected(), Some(false));
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Mode"))
            .count(),
        1,
        "the picker's label must name its single group node exactly once"
    );

    assert!(
        act(&mut runtime, Action::Click, children[1]),
        "the radio Click changed nothing"
    );
    assert_eq!(selection.snapshot(), 1, "the radio option did not select");
}

/// A visually hidden radio-picker label draws nothing and takes no space, but
/// the group's accessibility node still carries the label — exactly once.
#[test]
fn radio_picker_hidden_label_still_names_the_group() {
    let selection = Binding::container(0i32);
    let selection_for_view = selection;
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((picker(
            "Mode",
            vec![text("Auto").tag(0i32), text("Manual").tag(1i32)],
            &selection_for_view,
        )
        .style(PickerStyle::Radio)
        .hide_label(),)))
    }));

    let update = pumped(&mut runtime);
    find_by_label(&update, Role::Group, "Mode")
        .expect("a hidden label still names the radio group");
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Mode"))
            .count(),
        1,
        "a hidden label must not add a second node carrying the label"
    );
}

#[test]
fn date_picker_emits_and_set_value_edits() {
    let day = Binding::container(jiff::civil::date(2025, 1, 15));
    let day_for_view = day.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((DatePicker::new("When", &day_for_view),)))
    }));

    let update = pumped(&mut runtime);
    let (picker_id, picker_node) =
        find_only(&update, Role::ComboBox).expect("the date picker is missing");
    // The picker's node names itself with its formatted value; the field's own
    // label is a sibling view's node.
    assert_eq!(picker_node.label(), Some("2025-01-15"));
    assert_eq!(picker_node.value(), Some("2025-01-15"));
    assert!(picker_node.supports_action(Action::SetValue));

    assert!(
        act_with_data(
            &mut runtime,
            Action::SetValue,
            picker_id,
            ActionData::Value("2030-05-06".into()),
        ),
        "SetValue changed nothing"
    );
    assert_eq!(
        day.snapshot(),
        jiff::civil::date(2030, 5, 6),
        "SetValue did not write the date binding"
    );
}

#[test]
fn color_picker_emits_and_popup_swatches_select() {
    let tint = Binding::container(Color::srgb(0, 0, 0));
    let tint_for_view = tint.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((ColorPicker::new("Tint", &tint_for_view),)))
    }));

    let update = pumped(&mut runtime);
    let (trigger, _) =
        find_by_label(&update, Role::Button, "Tint").expect("the color picker is missing");
    let button_count = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Button)
        .count();
    assert_eq!(
        button_count, 1,
        "swatches must not appear before the picker opens"
    );

    // Activation mounts the swatch panel as a semantic popup window whose
    // nodes merge into the root tree. Swatch labels are localized, so the
    // first non-trigger `Button` node is the first palette swatch — `Red`.
    assert!(
        act(&mut runtime, Action::Click, trigger),
        "the picker Click changed nothing"
    );
    let update = pumped(&mut runtime);
    let swatch = update
        .nodes
        .iter()
        .find_map(|(id, node)| {
            (node.role() == Role::Button && node.label() != Some("Tint")).then_some(*id)
        })
        .expect("no swatch appeared after the picker opened");
    assert!(
        act(&mut runtime, Action::Click, swatch),
        "the swatch Click changed nothing"
    );

    // The swatch wrote the binding and its `close_all` dismissed the panel.
    let env = Environment::new();
    let picked = tint.snapshot().resolve(&env).snapshot();
    let expected = Color::srgb(0xba, 0x1a, 0x1a).resolve(&env).snapshot();
    for (picked, expected, channel) in [
        (picked.components[0], expected.components[0], "red"),
        (picked.components[1], expected.components[1], "green"),
        (picked.components[2], expected.components[2], "blue"),
        (picked.components[3], expected.components[3], "opacity"),
    ] {
        assert_eq!(picked, expected, "the swatch wrote a different {channel}");
    }
    let update = pumped(&mut runtime);
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| !(node.role() == Role::Button && node.label() != Some("Tint"))),
        "a dismissed picker must leave the merged tree"
    );
}

#[test]
fn menu_opens_a_semantic_popup_window_and_commands_fire() {
    let fired = Binding::container(false);
    let fired_for_view = fired.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let fired = fired_for_view.clone();
        AnyView::new(vstack((
            text("host"),
            Menu::new("File", "Export".action(move || fired.set(true))),
        )))
    }));

    let update = pumped(&mut runtime);
    let (file, file_node) =
        find_by_label(&update, Role::Button, "File").expect("the menu trigger is missing");
    assert!(file_node.supports_action(Action::Click));
    assert!(
        find_by_label(&update, Role::Button, "Export").is_none(),
        "menu items must not emit before the menu opens"
    );

    // Activation mounts the item rows as a semantic popup window whose nodes
    // merge into the root tree.
    assert!(
        act(&mut runtime, Action::Click, file),
        "the menu Click changed nothing"
    );
    let update = pumped(&mut runtime);
    let (export, _) = find_by_label(&update, Role::Button, "Export")
        .expect("no command emitted after the menu opened");

    // The command's Click fires its action and closes the popup group, so the
    // merged tree drops the popup window's nodes on the next emit.
    assert!(
        act(&mut runtime, Action::Click, export),
        "the command Click changed nothing"
    );
    assert!(fired.snapshot(), "the menu command did not fire");
    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Button, "Export").is_none(),
        "a closed popup must leave the merged tree"
    );
}

/// `tree_update` is the "the tree changed" signal: a pump where no window —
/// main or popup — emitted must publish `None`, or a test host invalidates
/// its snapshot on every pump.
#[test]
fn a_clean_pump_publishes_no_tree_update() {
    let tint = Binding::container(Color::srgb(0, 0, 0));
    let tint_for_view = tint;
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((ColorPicker::new("Tint", &tint_for_view),)))
    }));

    let update = pumped(&mut runtime);
    assert!(
        runtime.pump().tree_update.is_none(),
        "a settled pump with no popup must publish nothing"
    );

    // With a popup open the same rule holds: the swatch panel's window is
    // mounted and merged, but once every core is clean the next pump
    // publishes nothing.
    let (trigger, _) =
        find_by_label(&update, Role::Button, "Tint").expect("the color picker is missing");
    assert!(act(&mut runtime, Action::Click, trigger));
    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Button, "Red").is_some(),
        "the swatch panel must be merged before the clean-pump check"
    );
    assert!(
        runtime.pump().tree_update.is_none(),
        "a settled pump with an open popup must publish nothing"
    );
}

#[test]
fn list_emits_all_rows_and_scrolls() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(List::content((
            || ListItem::new(text("Row 1")),
            || ListItem::new(text("Row 2")),
            || ListItem::new(text("Row 3")),
            || ListItem::new(text("Row 4")),
        )))
    }));

    let update = pumped(&mut runtime);
    let (list_id, list_node) = find_only(&update, Role::List).expect("the list is missing");
    assert_eq!(list_node.scroll_y(), Some(0.0));
    assert!(list_node.supports_action(Action::ScrollDown));
    assert!(list_node.supports_action(Action::ScrollUp));
    let children = list_node.children();
    assert_eq!(
        children.len(),
        4,
        "the semantic tree emits every row — there is no viewport to virtualize"
    );
    for (index, child) in children.iter().enumerate() {
        let row = lookup(&update, *child);
        assert_eq!(row.role(), Role::ListItem);
        let expected = format!("Row {}", index + 1);
        assert_eq!(row.label(), Some(expected.as_str()));
        assert!(row.supports_action(Action::Focus));
    }

    // The semantic scroll domain is measured in rows; ScrollDown moves the
    // bound offset directly.
    assert!(
        act(&mut runtime, Action::ScrollDown, list_id),
        "ScrollDown changed nothing"
    );
    let update = pumped(&mut runtime);
    let (_, list_node) = find_only(&update, Role::List).expect("the list vanished");
    let offset = list_node
        .scroll_y()
        .expect("the list must keep reporting its scroll offset");
    assert!(offset > 0.0, "ScrollDown did not move the scroll offset");
    assert!(
        act(&mut runtime, Action::ScrollUp, list_id),
        "ScrollUp changed nothing"
    );
    let update = pumped(&mut runtime);
    let (_, list_node) = find_only(&update, Role::List).expect("the list vanished");
    assert_eq!(
        list_node.scroll_y(),
        Some(0.0),
        "ScrollUp did not return the offset to the top"
    );

    // A vertical list does not serve horizontal directions: the action is
    // declined, not handled — only an *unknown* axis variant panics.
    assert!(
        !act(&mut runtime, Action::ScrollLeft, list_id),
        "a direction the axis does not serve must report unhandled"
    );
}

/// A row's content is the row's own subtree: every text inside it lands under
/// the `ListItem` node, and the row's label derives from that content the way
/// a composite button's does.
#[test]
fn list_rows_emit_content_subtree_and_derived_label() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(List::content((
            || ListItem::new(hstack((text("Name"), spacer(), text("Value")))),
            || ListItem::new(text("Plain")),
        )))
    }));

    let update = pumped(&mut runtime);
    assert_rooted(&update);
    let (_, list_node) = find_only(&update, Role::List).expect("the list is missing");
    let children = list_node.children();
    assert_eq!(
        children.len(),
        2,
        "the two rows must be the list's children"
    );

    let row = lookup(&update, children[0]);
    assert_eq!(row.role(), Role::ListItem);
    assert_eq!(
        row.label(),
        Some("Name Value"),
        "the row's label derives from its content like a composite button's"
    );
    assert!(row.supports_action(Action::Focus));
    // Both texts are nodes under the row — the spacer emits nothing.
    let row_children: Vec<&Node> = row
        .children()
        .iter()
        .map(|id| lookup(&update, *id))
        .collect();
    assert_eq!(
        row_children.len(),
        2,
        "only the two texts emit under the row"
    );
    assert_eq!(row_children[0].role(), Role::Label);
    assert_eq!(row_children[0].label(), Some("Name"));
    assert_eq!(row_children[1].role(), Role::Label);
    assert_eq!(row_children[1].label(), Some("Value"));

    let plain = lookup(&update, children[1]);
    assert_eq!(plain.role(), Role::ListItem);
    assert_eq!(plain.label(), Some("Plain"));
    let plain_child = lookup(&update, plain.children()[0]);
    assert_eq!(plain_child.role(), Role::Label);
    assert_eq!(plain_child.label(), Some("Plain"));
}

/// An interactive control inside a row stays its own actionable node — a
/// child of the `ListItem`, never folded into it — and the row derives the
/// control's own name.
#[test]
fn list_toggle_row_emits_actionable_switch() {
    let on = Binding::container(false);
    let on_for_view = on.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let on_for_row = on_for_view.clone();
        AnyView::new(List::content((move || {
            ListItem::new(toggle("Wi-Fi", &on_for_row))
        },)))
    }));

    let update = pumped(&mut runtime);
    assert_rooted(&update);
    let (switch, switch_node) =
        find_by_label(&update, Role::Switch, "Wi-Fi").expect("the toggle is missing");
    assert_eq!(switch_node.toggled(), Some(Toggled::False));
    assert!(switch_node.supports_action(Action::Click));
    assert!(switch_node.supports_action(Action::Focus));

    let (_, list_node) = find_only(&update, Role::List).expect("the list is missing");
    let row = lookup(&update, list_node.children()[0]);
    assert_eq!(row.role(), Role::ListItem);
    assert_eq!(
        row.label(),
        Some("Wi-Fi"),
        "the row derives the control's own name"
    );
    assert!(
        row.children().contains(&switch),
        "the toggle must sit under its row's ListItem node"
    );

    assert!(
        act(&mut runtime, Action::Click, switch),
        "Click changed nothing"
    );
    assert!(on.snapshot(), "the toggle binding did not flip");
}

/// An explicit `a11y_label` on the row's content names the `ListItem` node —
/// the row claims the naming scope, not a leaf inside it — while the content
/// subtree still emits under it.
#[test]
fn list_row_explicit_label_names_the_row() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(List::content((|| {
            ListItem::new(text("content text").a11y_label("Row name"))
        },)))
    }));

    let update = pumped(&mut runtime);
    assert_rooted(&update);
    let (_, list_node) = find_only(&update, Role::List).expect("the list is missing");
    let row = lookup(&update, list_node.children()[0]);
    assert_eq!(row.role(), Role::ListItem);
    assert_eq!(row.label(), Some("Row name"));
    let child = lookup(&update, row.children()[0]);
    assert_eq!(child.role(), Role::Label);
    assert_eq!(
        child.label(),
        Some("content text"),
        "the leaf keeps its own name — the row's label is not re-emitted"
    );
}

/// A row whose reactive accessibility state turns hidden leaves the tree on
/// the next frame — the hidden check must subscribe to the state signal, or
/// no rebuild is scheduled and the stale row outlives the flip.
#[test]
fn list_row_state_signal_hidden_removes_row_next_frame() {
    let state = Binding::container(AccessibilityState::new());
    let state_for_view = state.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let state_for_row = state_for_view.clone();
        AnyView::new(List::content((move || {
            ListItem::new(text("Hide me").a11y_state_signal(state_for_row.clone()))
        },)))
    }));

    let update = pumped(&mut runtime);
    assert_rooted(&update);
    let (_, list_node) = find_only(&update, Role::List).expect("the list is missing");
    assert_eq!(list_node.children().len(), 1, "the row starts visible");

    state.set(AccessibilityState::new().hidden(true));
    let update = pumped(&mut runtime);
    let (_, list_node) = find_only(&update, Role::List).expect("the list is missing");
    assert!(
        list_node.children().is_empty(),
        "a row whose state signal turns hidden must leave the tree on the next frame"
    );
}

#[test]
fn table_emits_headers_cells_and_scrolls() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(table([
            col("Name", vec![text("Ada"), text("Grace")]),
            col("Lang", vec![text("Rust"), text("COBOL")]),
        ]))
    }));

    let update = pumped(&mut runtime);
    let (table_id, table_node) = find_only(&update, Role::Table).expect("the table is missing");
    assert!(table_node.supports_action(Action::ScrollDown));
    assert!(table_node.supports_action(Action::ScrollUp));
    assert!(table_node.supports_action(Action::ScrollLeft));
    assert!(table_node.supports_action(Action::ScrollRight));
    assert_eq!(table_node.scroll_x(), Some(0.0));
    let children = table_node.children();
    assert_eq!(
        children.len(),
        6,
        "two column headers plus four cells must be the table's semantic children"
    );
    let headers: Vec<&Node> = children
        .iter()
        .map(|id| lookup(&update, *id))
        .filter(|node| node.role() == Role::ColumnHeader)
        .collect();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0].label(), Some("Name"));
    assert_eq!(headers[1].label(), Some("Lang"));
    let cells: Vec<&Node> = children
        .iter()
        .map(|id| lookup(&update, *id))
        .filter(|node| node.role() == Role::Cell)
        .collect();
    assert_eq!(
        cells.len(),
        4,
        "every cell exists in the semantic tree — there is no viewport to virtualize"
    );
    let labels: Vec<Option<&str>> = cells.iter().map(|cell| cell.label()).collect();
    for expected in ["Ada", "Grace", "Rust", "COBOL"] {
        assert!(
            labels.contains(&Some(expected)),
            "cell {expected:?} is missing from the semantic tree"
        );
    }

    assert!(
        act(&mut runtime, Action::ScrollDown, table_id),
        "ScrollDown changed nothing"
    );
    let update = pumped(&mut runtime);
    let (_, table_node) = find_only(&update, Role::Table).expect("the table vanished");
    assert!(
        table_node
            .scroll_y()
            .expect("the table must keep reporting its scroll offset")
            > 0.0,
        "ScrollDown did not move the scroll offset"
    );

    // The semantic scroll domain is measured in cells on both axes;
    // ScrollRight moves the bound horizontal offset directly.
    assert!(
        act(&mut runtime, Action::ScrollRight, table_id),
        "ScrollRight changed nothing"
    );
    let update = pumped(&mut runtime);
    let (_, table_node) = find_only(&update, Role::Table).expect("the table vanished");
    assert!(
        table_node
            .scroll_x()
            .expect("the table must keep reporting its horizontal offset")
            > 0.0,
        "ScrollRight did not move the horizontal offset"
    );
    assert!(
        act(&mut runtime, Action::ScrollLeft, table_id),
        "ScrollLeft changed nothing"
    );
    let update = pumped(&mut runtime);
    let (_, table_node) = find_only(&update, Role::Table).expect("the table vanished");
    assert_eq!(
        table_node.scroll_x(),
        Some(0.0),
        "ScrollLeft did not return the horizontal offset to the start"
    );
}

#[test]
fn tabs_emit_tab_list_and_click_selects() {
    let selection = Binding::container(0i32);
    let selection_for_view = selection.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(Tabs::new(
            &selection_for_view,
            vec![
                Tab::new(0, "First", || NavigationView::new("One", text("one"))),
                Tab::new(1, "Second", || NavigationView::new("Two", text("two"))),
            ],
        ))
    }));

    let update = pumped(&mut runtime);
    let (_, tab_list) = find_only(&update, Role::TabList).expect("the tab list is missing");
    let children = tab_list.children();
    assert_eq!(children.len(), 2, "the tab list must hold one node per tab");
    let first = lookup(&update, children[0]);
    let second = lookup(&update, children[1]);
    assert_eq!(first.role(), Role::Tab);
    assert_eq!(first.label(), Some("First"));
    assert_eq!(first.is_selected(), Some(true));
    assert_eq!(second.role(), Role::Tab);
    assert_eq!(second.label(), Some("Second"));
    assert_eq!(second.is_selected(), Some(false));
    assert!(second.supports_action(Action::Click));
    assert!(
        find_by_label(&update, Role::Label, "one").is_some(),
        "the selected tab's content must emit"
    );

    assert!(
        act(&mut runtime, Action::Click, children[1]),
        "the tab Click changed nothing"
    );
    assert_eq!(selection.snapshot(), 1, "the tab did not select");
    let update = pumped(&mut runtime);
    let (_, tab_list) = find_only(&update, Role::TabList).expect("the tab list vanished");
    let children = tab_list.children();
    assert_eq!(lookup(&update, children[0]).is_selected(), Some(false));
    assert_eq!(lookup(&update, children[1]).is_selected(), Some(true));
    assert!(
        find_by_label(&update, Role::Label, "two").is_some(),
        "the newly selected tab's content must emit"
    );
}

#[test]
fn navigation_view_emits_container_title_and_content() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(NavigationView::new("Settings", text("body content")))
    }));

    let update = pumped(&mut runtime);
    let (_, nav_node) =
        find_only(&update, Role::Navigation).expect("the navigation container is missing");
    assert!(
        !nav_node.children().is_empty(),
        "the navigation container must hold its chrome and content"
    );
    assert!(
        find_by_label(&update, Role::Header, "Settings").is_some(),
        "the navigation title must emit as a Header"
    );
    assert!(
        find_by_label(&update, Role::Label, "body content").is_some(),
        "the navigation content must emit"
    );
}

#[test]
fn scroll_view_emits_and_scroll_actions_move_offset() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(scroll(vstack((text("scrolled content"),))))
    }));

    let update = pumped(&mut runtime);
    let (scroller, scroll_node) =
        find_only(&update, Role::ScrollView).expect("the scroll view is missing");
    assert_eq!(scroll_node.scroll_y(), Some(0.0));
    assert_eq!(
        scroll_node.scroll_y_max(),
        Some(f64::INFINITY),
        "the semantic scroll domain is unbounded — no layout measures it"
    );
    for action in [Action::ScrollDown, Action::ScrollUp] {
        assert!(
            scroll_node.supports_action(action),
            "the scroll view must advertise {action:?}"
        );
    }
    assert!(
        scroll_node.children().iter().any(|id| {
            let node = lookup(&update, *id);
            node.role() == Role::Label && node.label() == Some("scrolled content")
        }),
        "the scroll view's content must emit inside it"
    );

    assert!(
        act(&mut runtime, Action::ScrollDown, scroller),
        "ScrollDown changed nothing"
    );
    let update = pumped(&mut runtime);
    let (_, scroll_node) = find_only(&update, Role::ScrollView).expect("the scroll view vanished");
    assert!(
        scroll_node.scroll_y().expect("scroll offset must be set") > 0.0,
        "ScrollDown did not move the bound offset"
    );
    assert!(
        act(&mut runtime, Action::ScrollUp, scroller),
        "ScrollUp changed nothing"
    );
    let update = pumped(&mut runtime);
    let (_, scroll_node) = find_only(&update, Role::ScrollView).expect("the scroll view vanished");
    assert_eq!(
        scroll_node.scroll_y(),
        Some(0.0),
        "ScrollUp did not return the offset to the top"
    );
}

#[test]
fn badge_emits_count_beside_content() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(waterui::component::badge::Badge::new(3, text("Inbox")))
    }));

    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Label, "Inbox").is_some(),
        "the badged content must emit"
    );
    assert!(
        find_by_label(&update, Role::Label, "3").is_some(),
        "the badge count must emit as a Label node"
    );
}

#[test]
fn layout_only_views_emit_nothing() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((Divider, text("only semantic content"))))
    }));

    let update = pumped(&mut runtime);
    // A divider is presentation chrome: the tree holds the window root and the
    // text — nothing else.
    assert_eq!(update.nodes.len(), 2);
    assert!(
        find_by_label(&update, Role::Label, "only semantic content").is_some(),
        "the text must still emit"
    );
}

/// Queues one key press for the next pump — the semantic input path's only
/// windowed input besides IME. The pump drains the event, moves focus or
/// activates through the semantic target, and re-emits in the same pass.
fn press(runtime: &mut SemanticRuntime, key: KeyCode, modifiers: Modifiers) {
    runtime.push_input_event(InputEvent::Key {
        logical_key: key.to_w3c_key(),
        physical_code: keyboard_types::Code::Unidentified,
        repeat: false,
        key,
        state: KeyState::Pressed,
        modifiers,
    });
}

#[test]
fn tab_traverses_the_semantic_tree_and_activation_dispatches_click() {
    let tapped = Binding::container(false);
    let on = Binding::container(false);
    let value = Binding::container(Str::default());
    let tapped_for_view = tapped.clone();
    let on_for_view = on.clone();
    let value_for_view = value;
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let tapped = tapped_for_view.clone();
        AnyView::new(vstack((
            button("First").action(move || tapped.set(true)),
            toggle("Mode", &on_for_view),
            field("Name", &value_for_view),
            button("Last"),
        )))
    }));

    let update = pumped(&mut runtime);
    let (first, _) = find_by_label(&update, Role::Button, "First").expect("First is missing");
    let (switch, _) = find_only(&update, Role::Switch).expect("the toggle is missing");
    let (name, _) = find_only(&update, Role::TextInput).expect("the field is missing");
    let (last, _) = find_by_label(&update, Role::Button, "Last").expect("Last is missing");

    // Traversal order is the semantic order of the tree — the order the nodes
    // emit in — not any pointer-target order.
    for expected in [first, switch, name, last] {
        press(
            &mut runtime,
            KeyCode::Named("Tab".into()),
            Modifiers::default(),
        );
        let update = pumped(&mut runtime);
        assert_eq!(
            update.focus, expected,
            "Tab must land on {expected:?} — the focused node is reported in the tree"
        );
    }
    // Traversal wraps; Shift-Tab walks the same order in reverse.
    press(
        &mut runtime,
        KeyCode::Named("Tab".into()),
        Modifiers::default(),
    );
    let update = pumped(&mut runtime);
    assert_eq!(update.focus, first, "Tab must wrap to the first candidate");
    press(
        &mut runtime,
        KeyCode::Named("Tab".into()),
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    let update = pumped(&mut runtime);
    assert_eq!(
        update.focus, last,
        "Shift-Tab must move to the previous candidate"
    );

    // Return to First: one more Shift-Tab from Last walks the order backwards.
    press(
        &mut runtime,
        KeyCode::Named("Tab".into()),
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    press(
        &mut runtime,
        KeyCode::Named("Tab".into()),
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    press(
        &mut runtime,
        KeyCode::Named("Tab".into()),
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    let update = pumped(&mut runtime);
    assert_eq!(update.focus, first);

    // Enter activates the focused node through the same semantic target a
    // `Click` action request uses.
    press(
        &mut runtime,
        KeyCode::Named("Enter".into()),
        Modifiers::default(),
    );
    let _ = pumped(&mut runtime);
    assert!(
        tapped.snapshot(),
        "Enter did not activate the focused button"
    );

    // Space on the focused toggle flips its binding through the same path.
    press(
        &mut runtime,
        KeyCode::Named("Tab".into()),
        Modifiers::default(),
    );
    let update = pumped(&mut runtime);
    assert_eq!(update.focus, switch);
    press(
        &mut runtime,
        KeyCode::Named("Space".into()),
        Modifiers::default(),
    );
    let _ = pumped(&mut runtime);
    assert!(on.snapshot(), "Space did not activate the focused toggle");
}

#[test]
fn navigation_stack_pushes_and_back_click_pops() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(NavigationStack::new(NavigationView::new(
            "Root",
            vstack((NavigationLink::new("Open Detail", || {
                NavigationView::new("Detail", text("detail content"))
            }),)),
        )))
    }));

    let update = pumped(&mut runtime);
    let (open, _) =
        find_by_label(&update, Role::Button, "Open Detail").expect("the link is missing");
    assert!(
        act(&mut runtime, Action::Click, open),
        "the link Click changed nothing"
    );

    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Header, "Detail").is_some(),
        "the pushed destination's title must emit"
    );
    assert!(
        find_by_label(&update, Role::Label, "detail content").is_some(),
        "the pushed destination's content must emit"
    );
    // The pushed page's only Button is the back affordance — its label is
    // localized, so it is found positionally.
    let (back, back_node) = find_only(&update, Role::Button).expect("the back button is missing");
    assert!(back_node.supports_action(Action::Click));
    assert!(
        find_by_label(&update, Role::Button, "Open Detail").is_none(),
        "the root page must leave the tree while a destination is pushed"
    );

    assert!(
        act(&mut runtime, Action::Click, back),
        "the back Click changed nothing"
    );
    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Header, "Root").is_some(),
        "the previous title must be restored after the pop"
    );
    assert!(
        find_by_label(&update, Role::Button, "Open Detail").is_some(),
        "the root page's content must be restored after the pop"
    );
    assert!(
        find_by_label(&update, Role::Label, "detail content").is_none(),
        "the popped destination must leave the tree"
    );
}

#[test]
fn navigation_stack_fires_lifecycle_hooks_on_push_and_pop() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let log_for_view = log.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let log = log_for_view.clone();
        let detail_log = log.clone();
        AnyView::new(NavigationStack::new(
            NavigationView::new(
                "Root",
                vstack((NavigationLink::new("Open Detail", move || {
                    let log = detail_log.clone();
                    NavigationView::new("Detail", text("detail content"))
                        .on_navigation_appear({
                            let log = log.clone();
                            move || log.borrow_mut().push("detail-appear")
                        })
                        .on_navigation_disappear({
                            let log = log.clone();
                            move || log.borrow_mut().push("detail-disappear")
                        })
                        .on_navigation_pop(move || log.borrow_mut().push("detail-pop"))
                }),)),
            )
            .on_navigation_appear({
                let log = log.clone();
                move || log.borrow_mut().push("root-appear")
            })
            .on_navigation_disappear(move || log.borrow_mut().push("root-disappear")),
        ))
    }));

    let update = pumped(&mut runtime);
    assert_eq!(
        log.borrow().as_slice(),
        ["root-appear"],
        "mounting the stack activates the root exactly once"
    );

    let (open, _) =
        find_by_label(&update, Role::Button, "Open Detail").expect("the link is missing");
    assert!(act(&mut runtime, Action::Click, open));
    let update = pumped(&mut runtime);
    assert_eq!(
        log.borrow().as_slice(),
        ["root-appear", "root-disappear", "detail-appear"],
        "a push must disappear the root and appear the destination"
    );

    let (back, _) = find_only(&update, Role::Button).expect("the back button is missing");
    assert!(act(&mut runtime, Action::Click, back));
    pumped(&mut runtime);
    assert_eq!(
        log.borrow().as_slice(),
        [
            "root-appear",
            "root-disappear",
            "detail-appear",
            "detail-disappear",
            "detail-pop",
            "root-appear",
        ],
        "a pop must disappear and pop the destination, then reappear the root"
    );
}

/// A destination with `.navigation_pop_enabled(false)` still *handles* the
/// back button's `Click`: the activation fires `pop_attempted` and keeps the
/// destination active. Reporting the pop's denial as "unhandled" would tell
/// a test host the button did nothing.
#[test]
fn navigation_stack_denied_pop_reports_attempt_and_keeps_destination() {
    use std::cell::Cell;
    use std::rc::Rc;

    let attempts = Rc::new(Cell::new(0u32));
    let mounted_attempts = Rc::clone(&attempts);
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let attempts = Rc::clone(&mounted_attempts);
        AnyView::new(NavigationStack::new(NavigationView::new(
            "Root",
            vstack((NavigationLink::new("Open Locked", move || {
                let attempts = Rc::clone(&attempts);
                NavigationView::new("Locked", text("locked content"))
                    .navigation_pop_enabled(false)
                    .on_navigation_pop_attempted(move || attempts.set(attempts.get() + 1))
            }),)),
        )))
    }));

    let update = pumped(&mut runtime);
    let (open, _) =
        find_by_label(&update, Role::Button, "Open Locked").expect("the link is missing");
    assert!(act(&mut runtime, Action::Click, open));

    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Label, "locked content").is_some(),
        "the locked destination's content must emit"
    );
    let (back, _) = find_only(&update, Role::Button).expect("the back button is missing");

    assert!(
        act(&mut runtime, Action::Click, back),
        "a denied pop is still a handled activation"
    );
    assert_eq!(attempts.get(), 1, "one denied pop reports one attempt");
    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Label, "locked content").is_some(),
        "the denied pop must keep the destination active"
    );
    assert!(
        find_by_label(&update, Role::Button, "Open Locked").is_none(),
        "the root page must not return on a denied pop"
    );
}

#[test]
fn navigation_split_emits_sidebar_and_selected_detail() {
    let selection = Binding::container(None::<i32>);
    let selection_for_view = selection.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let selection = selection_for_view.clone();
        let sidebar_selection = selection.clone();
        AnyView::new(
            NavigationSplitView::new(
                &selection,
                move || {
                    let selection = sidebar_selection.clone();
                    vstack((button("Select Detail").action(move || selection.set(Some(7))),))
                },
                |value| NavigationView::new("Detail", text(format!("detail:{value}"))),
            )
            .placeholder(|| text("placeholder content")),
        )
    }));

    let update = pumped(&mut runtime);
    let (select, _) = find_by_label(&update, Role::Button, "Select Detail")
        .expect("the sidebar control is missing");
    assert!(
        find_by_label(&update, Role::Label, "placeholder content").is_some(),
        "an empty selection must emit the placeholder"
    );

    assert!(
        act(&mut runtime, Action::Click, select),
        "the sidebar Click changed nothing"
    );
    assert_eq!(
        selection.snapshot(),
        Some(7),
        "the sidebar did not write the selection"
    );
    let update = pumped(&mut runtime);
    assert!(
        find_by_label(&update, Role::Header, "Detail").is_some(),
        "the selected detail's title must emit"
    );
    assert!(
        find_by_label(&update, Role::Label, "detail:7").is_some(),
        "the selected detail's content must emit"
    );
    assert!(
        find_by_label(&update, Role::Label, "placeholder content").is_none(),
        "the placeholder must leave the tree once a detail is selected"
    );
}

#[test]
fn segmented_picker_emits_group_and_click_selects() {
    let selection = Binding::container(0i32);
    let selection_for_view = selection.clone();
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((picker(
            "Mode",
            vec![text("Auto").tag(0i32), text("Manual").tag(1i32)],
            &selection_for_view,
        )
        .style(PickerStyle::Segmented),)))
    }));

    let update = pumped(&mut runtime);
    let (_, group_node) =
        find_by_label(&update, Role::Group, "Mode").expect("the segmented group is missing");
    let children = group_node.children();
    assert_eq!(
        children.len(),
        2,
        "the group must hold one node per segment"
    );
    let auto = lookup(&update, children[0]);
    let manual = lookup(&update, children[1]);
    assert_eq!(auto.role(), Role::RadioButton);
    assert_eq!(auto.label(), Some("Auto"));
    assert_eq!(auto.is_selected(), Some(true));
    assert_eq!(manual.role(), Role::RadioButton);
    assert_eq!(manual.label(), Some("Manual"));
    assert_eq!(manual.is_selected(), Some(false));
    assert!(manual.supports_action(Action::Click));
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Mode"))
            .count(),
        1,
        "the picker's label must name its single group node exactly once"
    );

    assert!(
        act(&mut runtime, Action::Click, children[1]),
        "the segment Click changed nothing"
    );
    assert_eq!(selection.snapshot(), 1, "the segment did not select");
    let update = pumped(&mut runtime);
    let (_, group_node) =
        find_by_label(&update, Role::Group, "Mode").expect("the segmented group vanished");
    let children = group_node.children();
    assert_eq!(lookup(&update, children[0]).is_selected(), Some(false));
    assert_eq!(lookup(&update, children[1]).is_selected(), Some(true));
}

/// A visually hidden segmented-picker label draws nothing and takes no space,
/// but the group's accessibility node still carries the label — exactly once.
#[test]
fn segmented_picker_hidden_label_still_names_the_group() {
    let selection = Binding::container(0i32);
    let selection_for_view = selection;
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((picker(
            "Mode",
            vec![text("Auto").tag(0i32), text("Manual").tag(1i32)],
            &selection_for_view,
        )
        .style(PickerStyle::Segmented)
        .hide_label(),)))
    }));

    let update = pumped(&mut runtime);
    find_by_label(&update, Role::Group, "Mode")
        .expect("a hidden label still names the segmented group");
    assert_eq!(
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Mode"))
            .count(),
        1,
        "a hidden label must not add a second node carrying the label"
    );
}

/// A scene that names itself — the label the semantic tree has to offer its
/// `Image` node, since a scene reaches the tree as anonymous fills.
struct Chart;

impl SceneContent for Chart {
    fn build_scene(
        &mut self,
        _recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        false
    }

    fn accessibility_label(&self) -> Option<String> {
        Some("weekly chart".to_string())
    }

    fn rebuild_for_engine(&mut self) {}
}

#[test]
fn scene_view_emits_an_image_leaf_with_its_content_label() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(SceneView::new(Chart))
    }));

    let update = pumped(&mut runtime);
    let (_, image_node) =
        find_only(&update, Role::Image).expect("the scene view must emit an Image leaf");
    assert_eq!(image_node.label(), Some("weekly chart"));
}

/// A GPU surface whose view names itself and reports its own semantic content —
/// the pair of answers its `Image` node has to publish, since a surface reaches
/// the tree as bare pixels.
struct GpuChart;

impl GpuContent for GpuChart {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, _frame: &mut GpuFrame<'_>) {}
}

#[test]
fn gpu_surface_emits_an_image_leaf_with_its_label_and_value() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(
            GpuContentView::new(GpuChart)
                .labeled("weekly chart")
                .described("up 12% week over week"),
        )
    }));

    let update = pumped(&mut runtime);
    let (_, image_node) =
        find_only(&update, Role::Image).expect("the gpu surface must emit an Image leaf");
    assert_eq!(image_node.label(), Some("weekly chart"));
    assert_eq!(image_node.value(), Some("up 12% week over week"));
}

#[test]
fn lazy_stack_emits_every_item_without_layout() {
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(LazyContainer::new(
            VStackLayout::default(),
            vec![
                text("Item 1"),
                text("Item 2"),
                text("Item 3"),
                text("Item 4"),
                text("Item 5"),
            ],
        ))
    }));

    let update = pumped(&mut runtime);
    for index in 1..=5 {
        let expected = format!("Item {index}");
        assert!(
            find_by_label(&update, Role::Label, &expected).is_some(),
            "lazy item {expected:?} is missing — there is no viewport to virtualize against"
        );
    }
}

/// The `.focused(binding)` direction of focus: a runtime write to the focus
/// binding lands UI focus on the field's node in the emitted tree, clearing
/// the binding clears it, and a non-text node can hold the tree's focus
/// while UI focus stays on the field — the two are deliberately separate.
/// (Keyboard traversal is the other contract: it carries the caret onto a
/// non-text target and ends editing — #95.)
#[test]
fn focused_binding_moves_ui_focus_and_tree_focus() {
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Name,
        Email,
    }

    let focus = Binding::container(None::<Field>);
    let name = Binding::container(Str::default());
    let email = Binding::container(Str::default());
    let focus_for_view = focus.clone();
    let name_for_view = name;
    let email_for_view = email;
    let mut runtime = mount(AnyViewBuilder::<AnyView>::new(move || {
        let focus = focus_for_view.clone();
        AnyView::new(vstack((
            field("Name", &name_for_view).focused(&focus, Field::Name),
            field("Email", &email_for_view).focused(&focus, Field::Email),
            button("Done"),
        )))
    }));

    let update = pumped(&mut runtime);
    let (name_node, _) =
        find_by_label(&update, Role::TextInput, "Name").expect("the Name field is missing");
    let (email_node, _) =
        find_by_label(&update, Role::TextInput, "Email").expect("the Email field is missing");
    let (done, _) = find_by_label(&update, Role::Button, "Done").expect("the button is missing");
    assert_eq!(runtime.focused_ui_node(), None);

    // A runtime write moves UI focus onto the field — and the tree reports
    // focus on its node too.
    focus.set(Some(Field::Name));
    let update = pumped(&mut runtime);
    assert_eq!(
        update.focus, name_node,
        "the tree must report focus on Name"
    );
    assert_eq!(runtime.focused_ui_node(), Some(name_node));

    // A later write moves both to the second field.
    focus.set(Some(Field::Email));
    let update = pumped(&mut runtime);
    assert_eq!(update.focus, email_node);
    assert_eq!(runtime.focused_ui_node(), Some(email_node));

    // A non-text node taking the tree's focus leaves UI focus on the field —
    // the text caret is independent of where accessibility focus sits.
    assert!(act(&mut runtime, Action::Focus, done));
    let update = pumped(&mut runtime);
    assert_eq!(update.focus, done);
    assert_eq!(
        runtime.focused_ui_node(),
        Some(email_node),
        "UI focus is the text caret — the button holds only the tree's focus"
    );
    assert_eq!(focus.snapshot(), Some(Field::Email));

    // Clearing the binding clears UI focus and writes None back; the tree's
    // focus stays on the button it moved to.
    focus.set(None);
    let update = pumped(&mut runtime);
    assert_eq!(runtime.focused_ui_node(), None);
    assert_eq!(focus.snapshot(), None);
    assert_eq!(update.focus, done);

    // A cleared UI focus accepts a new target: an accessibility Focus on the
    // field moves UI focus and the tree's focus, and writes the tag back.
    assert!(act(&mut runtime, Action::Focus, name_node));
    let update = pumped(&mut runtime);
    assert_eq!(update.focus, name_node);
    assert_eq!(runtime.focused_ui_node(), Some(name_node));
    assert_eq!(focus.snapshot(), Some(Field::Name));
}

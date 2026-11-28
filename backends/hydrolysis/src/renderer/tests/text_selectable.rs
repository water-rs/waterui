//! Selectable static text (`Text::selectable` / `ViewExt::text_selectable`,
//! water-rs/waterui#4329): read-only selection + copy with zero layout change,
//! exact non-selectable hit-test pass-through, subtree inheritance + leaf
//! override, reactive toggling, mutation/IME refusal, and accessibility.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Action, ActionData, ActionRequest, NodeId, Role, TextPosition, TextSelection};
use nami::Binding;
use waterui::ViewExt as _;
use waterui::component::text;
use waterui_controls::button::button;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Str};
use waterui_layout::stack::vstack;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, KeyCode, KeyState, Modifiers, PointerButton, PointerKind};

const WINDOW: (u32, u32) = (360, 260);

fn pump_until_settled(runtime: &mut HeadlessRuntime) -> Option<accesskit::TreeUpdate> {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
    runtime.accessibility_tree()
}

fn find_node_by_label(runtime: &mut HeadlessRuntime, label: &str) -> (NodeId, accesskit::Node) {
    let update = pump_until_settled(runtime).expect("runtime must publish an accessibility tree");
    let (id, node) = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .unwrap_or_else(|| {
            panic!("node with label {label:?} must be present in accessibility tree")
        });
    (*id, node.clone())
}

fn bounds_of(runtime: &mut HeadlessRuntime, role: Role, label: &str) -> accesskit::Rect {
    let update = pump_until_settled(runtime).expect("runtime must publish an accessibility tree");
    find_by_label(&update, role, label)
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("{role:?} {label:?} must carry bounds"))
}

fn click_at(runtime: &mut HeadlessRuntime, x: f32, y: f32, button: PointerButton) {
    for event in [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button,
        },
    ] {
        runtime.push_input_event(event);
    }
    let _ = pump_until_settled(runtime);
}

fn drag_across(runtime: &mut HeadlessRuntime, start: (f32, f32), end: (f32, f32)) {
    runtime.push_input_event(InputEvent::PointerDown {
        id: 1,
        kind: PointerKind::Mouse,
        x: start.0,
        y: start.1,
        button: PointerButton::Primary,
    });
    runtime.push_input_event(InputEvent::PointerMove {
        id: 1,
        kind: PointerKind::Mouse,
        x: end.0,
        y: end.1,
    });
    runtime.push_input_event(InputEvent::PointerUp {
        id: 1,
        kind: PointerKind::Mouse,
        x: end.0,
        y: end.1,
        button: PointerButton::Primary,
    });
    let _ = pump_until_settled(runtime);
}

fn primary_shortcut_modifiers() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers {
            super_key: true,
            ..Modifiers::default()
        }
    } else {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    }
}

fn key_chord(runtime: &mut HeadlessRuntime, ch: &str, modifiers: Modifiers) {
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(InputEvent::Key {
            logical_key: keyboard_types::Key::Character(ch.to_owned()),
            physical_code: keyboard_types::Code::Unidentified,
            repeat: false,
            key: KeyCode::Character(ch.to_owned()),
            state,
            modifiers,
        });
    }
    let _ = pump_until_settled(runtime);
}

fn named_key(runtime: &mut HeadlessRuntime, key: keyboard_types::NamedKey, name: &str) {
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(InputEvent::Key {
            logical_key: keyboard_types::Key::Named(key),
            physical_code: keyboard_types::Code::Unidentified,
            repeat: false,
            key: KeyCode::Named(name.to_owned()),
            state,
            modifiers: Modifiers::default(),
        });
    }
    let _ = pump_until_settled(runtime);
}

fn attach_clipboard_spy(runtime: &mut HeadlessRuntime) -> Rc<RefCell<Vec<String>>> {
    let copied = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&copied);
    runtime.set_clipboard_write(Rc::new(move |value: &str| {
        sink.borrow_mut().push(value.to_owned());
        true
    }));
    copied
}

/// Non-selectable `Text` (default) must preserve exact layout bounds, stay
/// non-focusable in accessibility, and pass clicks through to parent handlers.
#[test]
fn non_selectable_text_matches_layout_and_passes_clicks_through() {
    let mut non_selectable_runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(|| AnyView::new(text("ERR_42_TIMEOUT"))),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );

    let mut selectable_runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(|| AnyView::new(text("ERR_42_TIMEOUT").selectable(true))),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );

    let (_, non_sel_node) = find_node_by_label(&mut non_selectable_runtime, "ERR_42_TIMEOUT");
    let (_, sel_node) = find_node_by_label(&mut selectable_runtime, "ERR_42_TIMEOUT");

    assert_eq!(
        non_sel_node.bounds(),
        sel_node.bounds(),
        "selectable text must measure and place identically to non-selectable text"
    );
    assert!(
        !non_sel_node.supports_action(Action::SetTextSelection),
        "non-selectable text must not advertise SetTextSelection"
    );
    assert!(
        !non_sel_node.supports_action(Action::Focus),
        "non-selectable text must not become focusable"
    );

    let taps = Rc::new(Cell::new(0usize));
    let taps_for_view = Rc::clone(&taps);
    let mut tappable_runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let taps = Rc::clone(&taps_for_view);
            AnyView::new(vstack((text("ERR_42_TIMEOUT"),)).on_tap(move || {
                taps.set(taps.get() + 1);
            }))
        }),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    let (_, tappable_node) = find_node_by_label(&mut tappable_runtime, "ERR_42_TIMEOUT");
    let rect = tappable_node.bounds().expect("label has bounds");
    click_at(
        &mut tappable_runtime,
        crate::num_cast::f64_as_f32(f64::midpoint(rect.x0, rect.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(rect.y0, rect.y1)),
        PointerButton::Primary,
    );
    assert_eq!(
        taps.get(),
        1,
        "non-selectable text must pass clicks through to parent tap handlers"
    );
}

/// Selectable text supports drag selection, Select All + Copy shortcuts,
/// read-only context menu (Copy + Select All only), and refuses mutation/IME.
#[test]
fn selectable_text_drag_copy_context_menu_and_refuses_mutation() {
    let mut env = test_environment();
    crate::localization::install(&mut env);
    let mut runtime = HeadlessRuntime::new_for_tests(
        env,
        AnyViewBuilder::<AnyView>::new(|| AnyView::new(text("alpha beta gamma").selectable(true))),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    let copied = attach_clipboard_spy(&mut runtime);

    let (_label_id, node) = find_node_by_label(&mut runtime, "alpha beta gamma");
    assert!(node.is_read_only(), "selectable text must expose read-only");
    assert!(
        node.supports_action(Action::Focus),
        "selectable text must expose Focus"
    );
    assert!(
        node.supports_action(Action::SetTextSelection),
        "selectable text must expose SetTextSelection"
    );
    assert!(
        !node.supports_action(Action::SetValue),
        "selectable static text must not advertise SetValue"
    );
    assert!(
        !node.supports_action(Action::ReplaceSelectedText),
        "selectable static text must not advertise ReplaceSelectedText"
    );

    let rect = node.bounds().expect("label has bounds");
    let mid_y = crate::num_cast::f64_as_f32(f64::midpoint(rect.y0, rect.y1));
    let start_x = crate::num_cast::f64_as_f32(rect.x0 + 1.0);
    let mid_x = crate::num_cast::f64_as_f32(f64::midpoint(rect.x0, rect.x1));

    // Drag across the first half of the text and press Cmd/Ctrl+C.
    drag_across(&mut runtime, (start_x, mid_y), (mid_x, mid_y));
    assert!(
        runtime.focused_text_input_state().is_none(),
        "selectable static text must never activate platform IME / soft keyboard"
    );
    key_chord(&mut runtime, "c", primary_shortcut_modifiers());
    let first_copy = copied
        .borrow()
        .last()
        .cloned()
        .expect("Cmd/Ctrl+C must copy the dragged selection");
    assert!(
        !first_copy.is_empty() && "alpha beta gamma".starts_with(&first_copy),
        "dragged selection copy {first_copy:?} must be a non-empty prefix of the text"
    );
    assert!(
        first_copy.len() < "alpha beta gamma".len(),
        "dragging to the midpoint must select a strict substring, got {first_copy:?}"
    );

    // Mutation attempts (TextInput, IME preedit/commit, Backspace, Delete, Cut, Paste)
    // must be ignored and leave the label text unchanged.
    runtime.push_input_event(InputEvent::TextInput {
        text: "MUTATED".to_owned(),
    });
    runtime.push_input_event(InputEvent::ImePreedit {
        text: "preedit".to_owned(),
        caret: Some(7),
    });
    runtime.push_input_event(InputEvent::ImeCommit {
        text: "committed".to_owned(),
    });
    named_key(
        &mut runtime,
        keyboard_types::NamedKey::Backspace,
        "Backspace",
    );
    named_key(&mut runtime, keyboard_types::NamedKey::Delete, "Delete");
    key_chord(&mut runtime, "x", primary_shortcut_modifiers());
    key_chord(&mut runtime, "v", primary_shortcut_modifiers());

    // Cmd/Ctrl+A selects the entire string and Cmd/Ctrl+C copies it intact.
    key_chord(&mut runtime, "a", primary_shortcut_modifiers());
    key_chord(&mut runtime, "c", primary_shortcut_modifiers());
    assert_eq!(
        copied.borrow().last().map(String::as_str),
        Some("alpha beta gamma"),
        "Select All + Copy must copy the intact original text"
    );

    // Right-click opens read-only context menu overlay whose first row is Copy.
    #[cfg(feature = "menu")]
    {
        click_at(&mut runtime, mid_x, mid_y, PointerButton::Secondary);
        let before_count = copied.borrow().len();
        click_at(
            &mut runtime,
            mid_x + 12.0,
            mid_y + 12.0,
            PointerButton::Primary,
        );
        assert_eq!(
            copied.borrow().len(),
            before_count + 1,
            "clicking Copy in the selectable-text context menu overlay must write to the clipboard"
        );
        assert_eq!(
            copied.borrow().last().map(String::as_str),
            Some("alpha beta gamma")
        );
    }
}

/// Selectable static text context menu offers only Copy (when a selection is
/// non-empty) and Select All — never Cut or Paste — and executing Copy writes
/// the selected substring to the clipboard.
#[cfg(feature = "menu")]
#[test]
fn selectable_text_context_menu_offers_only_copy_and_select_all() {
    use crate::renderer::{PopupMenuNode, SemanticCore, TextInputModel, TextSelectionSlot};

    let mut env = test_environment();
    crate::localization::install(&mut env);

    let copied = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = Rc::clone(&copied);
    let clipboard: crate::renderer::text_editing::ClipboardWrite = Rc::new(move |value: &str| {
        sink.borrow_mut().push(value.to_owned());
        true
    });

    // Collapsed selection: only Select All.
    let selection = Rc::new(RefCell::new(TextSelectionSlot {
        anchor: 0,
        focus: 0,
        initialized: true,
    }));
    let target = super::text_input_target(
        TextInputModel::SelectableText {
            value: Str::from_static("err_503"),
        },
        Rc::clone(&selection),
    );
    let collapsed_nodes =
        SemanticCore::build_text_context_menu_nodes(&target, &env, true, &clipboard);
    let collapsed_labels: Vec<&str> = collapsed_nodes
        .iter()
        .filter_map(|node| match node {
            PopupMenuNode::Command { plain_label, .. } => Some(plain_label.as_str()),
            PopupMenuNode::Menu { .. } | PopupMenuNode::Divider => None,
        })
        .collect();
    assert_eq!(collapsed_labels, vec!["Select All"]);

    // Non-empty selection: Copy and Select All only (no Cut, no Paste).
    selection.borrow_mut().focus = 3;
    let selected_nodes =
        SemanticCore::build_text_context_menu_nodes(&target, &env, true, &clipboard);
    let selected_labels: Vec<&str> = selected_nodes
        .iter()
        .filter_map(|node| match node {
            PopupMenuNode::Command { plain_label, .. } => Some(plain_label.as_str()),
            PopupMenuNode::Menu { .. } | PopupMenuNode::Divider => None,
        })
        .collect();
    assert_eq!(selected_labels, vec!["Copy", "Select All"]);

    if let PopupMenuNode::Command { action, .. } = &selected_nodes[0] {
        action.call(&env).expect("Copy command must succeed");
    } else {
        panic!("expected Copy command at index 0");
    }
    assert_eq!(copied.borrow().as_slice(), ["err"]);
}

/// Subtree `.text_selectable(true)` makes descendant text selectable, leaf
/// `.selectable(false)` opts out, and buttons inside the scope stay clickable.
#[test]
fn subtree_text_selectable_inherits_supports_leaf_override_and_preserves_buttons() {
    let button_clicks = Rc::new(Cell::new(0usize));
    let skipped_taps = Rc::new(Cell::new(0usize));
    let clicks_for_view = Rc::clone(&button_clicks);
    let skipped_for_view = Rc::clone(&skipped_taps);

    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let clicks = Rc::clone(&clicks_for_view);
            let skipped = Rc::clone(&skipped_for_view);
            AnyView::new(
                vstack((
                    text("inherited_id_99"),
                    Str::from_static("plain_inherited"),
                    text("opt_out_label").selectable(false).on_tap(move || {
                        skipped.set(skipped.get() + 1);
                    }),
                    button(text("Action")).action(move || {
                        clicks.set(clicks.get() + 1);
                    }),
                ))
                .text_selectable(true),
            )
        }),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    let copied = attach_clipboard_spy(&mut runtime);

    // 1. Inherited `Text` and bare `Str` both become selectable.
    let (_, inherited_node) = find_node_by_label(&mut runtime, "inherited_id_99");
    let (_, plain_node) = find_node_by_label(&mut runtime, "plain_inherited");
    let (_, opt_out_node) = find_node_by_label(&mut runtime, "opt_out_label");
    assert!(inherited_node.supports_action(Action::SetTextSelection));
    assert!(plain_node.supports_action(Action::SetTextSelection));
    assert!(
        !opt_out_node.supports_action(Action::SetTextSelection),
        "leaf .selectable(false) must override enclosing .text_selectable(true)"
    );

    // Focus & copy the inherited text.
    let inh_rect = inherited_node.bounds().expect("inherited label bounds");
    click_at(
        &mut runtime,
        crate::num_cast::f64_as_f32(f64::midpoint(inh_rect.x0, inh_rect.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(inh_rect.y0, inh_rect.y1)),
        PointerButton::Primary,
    );
    key_chord(&mut runtime, "a", primary_shortcut_modifiers());
    key_chord(&mut runtime, "c", primary_shortcut_modifiers());
    assert_eq!(
        copied.borrow().last().map(String::as_str),
        Some("inherited_id_99")
    );

    // 2. Clicking the opt-out leaf passes through to its tap handler.
    let opt_rect = opt_out_node.bounds().expect("opt-out label bounds");
    click_at(
        &mut runtime,
        crate::num_cast::f64_as_f32(f64::midpoint(opt_rect.x0, opt_rect.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(opt_rect.y0, opt_rect.y1)),
        PointerButton::Primary,
    );
    assert_eq!(
        skipped_taps.get(),
        1,
        "leaf .selectable(false) must preserve tap pass-through inside .text_selectable(true)"
    );

    // 3. Clicking a button inside `.text_selectable(true)` triggers its action.
    let btn_rect = bounds_of(&mut runtime, Role::Button, "Action");
    click_at(
        &mut runtime,
        crate::num_cast::f64_as_f32(f64::midpoint(btn_rect.x0, btn_rect.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(btn_rect.y0, btn_rect.y1)),
        PointerButton::Primary,
    );
    assert_eq!(
        button_clicks.get(),
        1,
        "buttons inside .text_selectable(true) must stay clickable"
    );
}

/// Toggling a reactive `Binding<bool>` dynamically enables selection + copy,
/// then clears selection/focus and restores pass-through when toggled back off.
#[test]
fn reactive_selectable_toggle_enables_and_clears_selection() {
    let selectable = Binding::new(false);
    let taps = Rc::new(Cell::new(0usize));
    let selectable_for_view = selectable.clone();
    let taps_for_view = Rc::clone(&taps);

    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let taps = Rc::clone(&taps_for_view);
            AnyView::new(
                text("req_8f9a")
                    .selectable(selectable_for_view.clone())
                    .on_tap(move || {
                        taps.set(taps.get() + 1);
                    }),
            )
        }),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    let copied = attach_clipboard_spy(&mut runtime);

    // Initially false: non-selectable, clicks reach on_tap.
    let (_, node) = find_node_by_label(&mut runtime, "req_8f9a");
    assert!(!node.supports_action(Action::SetTextSelection));
    let rect = node.bounds().expect("bounds");
    let cx = crate::num_cast::f64_as_f32(f64::midpoint(rect.x0, rect.x1));
    let cy = crate::num_cast::f64_as_f32(f64::midpoint(rect.y0, rect.y1));
    click_at(&mut runtime, cx, cy, PointerButton::Primary);
    assert_eq!(taps.get(), 1);

    // Toggle to true: becomes selectable and copyable.
    selectable.set(true);
    let (label_id, node) = find_node_by_label(&mut runtime, "req_8f9a");
    assert!(node.supports_action(Action::SetTextSelection));
    click_at(&mut runtime, cx, cy, PointerButton::Primary);
    key_chord(&mut runtime, "a", primary_shortcut_modifiers());
    key_chord(&mut runtime, "c", primary_shortcut_modifiers());
    assert_eq!(copied.borrow().last().map(String::as_str), Some("req_8f9a"));

    // Verify accessibility SetTextSelection updates reported text_selection.
    assert!(runtime.perform_accessibility_action(ActionRequest {
        action: Action::SetTextSelection,
        target_tree: accesskit::TreeId::ROOT,
        target_node: label_id,
        data: Some(ActionData::SetTextSelection(TextSelection {
            anchor: TextPosition {
                node: label_id,
                character_index: 0,
            },
            focus: TextPosition {
                node: label_id,
                character_index: 3,
            },
        })),
    }));
    let (_, updated_node) = find_node_by_label(&mut runtime, "req_8f9a");
    let reported = updated_node
        .text_selection()
        .expect("selectable text must report its current selection to AccessKit");
    assert_eq!(reported.anchor.character_index, 0);
    assert_eq!(reported.focus.character_index, 3);
    key_chord(&mut runtime, "c", primary_shortcut_modifiers());
    assert_eq!(copied.borrow().last().map(String::as_str), Some("req"));

    // Toggle back to false: drops selection/focus and restores tap pass-through.
    selectable.set(false);
    let (_, disabled_node) = find_node_by_label(&mut runtime, "req_8f9a");
    assert!(!disabled_node.supports_action(Action::SetTextSelection));
    assert!(disabled_node.text_selection().is_none());
    let before_copies = copied.borrow().len();
    key_chord(&mut runtime, "c", primary_shortcut_modifiers());
    assert_eq!(
        copied.borrow().len(),
        before_copies,
        "copy must do nothing once selectable is toggled back to false"
    );
    let taps_before_restore = taps.get();
    click_at(&mut runtime, cx, cy, PointerButton::Primary);
    assert_eq!(
        taps.get(),
        taps_before_restore + 1,
        "toggling selectable back to false must restore tap pass-through"
    );
}

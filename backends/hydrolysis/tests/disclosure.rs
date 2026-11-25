//! Integration tests for [`Disclosure`]: focusable header, indicator
//! rendering, `expanded` accessibility reporting, lazy body creation/drop, and
//! keyboard (`Tab` + `Space`/`Enter`) + pointer activation.

mod support;

use core::cell::Cell;
use std::rc::Rc;

use accesskit::{Action as AccessibilityAction, Role};
use hydrolysis::{InputEvent, KeyCode, KeyState, Modifiers, keyboard_types};
use support::{mount_rendered, node, node_bounds_with_role, node_id_with_role, node_label_exists};
use waterui::Binding;
use waterui::Signal;
use waterui::component::label::TitleLabel;
use waterui::component::text;
use waterui::text::Text;
use waterui::widget::disclosure::Disclosure;
use waterui_core::Environment;

fn disclosure_view(
    expanded: &Binding<bool>,
    body_builds: Rc<Cell<usize>>,
) -> Disclosure<TitleLabel, impl Fn() -> Text + Clone + 'static> {
    Disclosure::new("Advanced", move || {
        body_builds.set(body_builds.get() + 1);
        text("Hidden details")
    })
    .expanded(expanded)
}

#[test]
fn disclosure_header_is_focusable_toggles_by_keyboard_and_pointer_and_builds_lazily() {
    let expanded = Binding::bool(false);
    let body_builds = Rc::new(Cell::new(0_usize));
    let view = disclosure_view(&expanded, Rc::clone(&body_builds));
    let mut app = mount_rendered(Environment::new(), view, 400, 300);
    app.settle();

    assert_eq!(
        body_builds.get(),
        0,
        "collapsed disclosure must build body lazily"
    );
    assert!(!node_label_exists(app.tree(), "Hidden details"));
    let header_id = node_id_with_role(app.tree(), Role::Button, "Advanced");
    assert_eq!(
        node(app.tree(), header_id).is_expanded(),
        Some(false),
        "collapsed header must report expanded=false to accessibility"
    );

    // Tab focuses the disclosure header; Space activates and expands it.
    app.press_key("Tab");
    assert_eq!(
        app.tree().focus,
        header_id,
        "Tab must focus the disclosure header"
    );

    let space_press = InputEvent::Key {
        logical_key: keyboard_types::Key::Character(" ".to_owned()),
        physical_code: keyboard_types::Code::Unidentified,
        repeat: false,
        key: KeyCode::Character(" ".to_owned()),
        state: KeyState::Pressed,
        modifiers: Modifiers::default(),
    };
    let space_release = InputEvent::Key {
        logical_key: keyboard_types::Key::Character(" ".to_owned()),
        physical_code: keyboard_types::Code::Unidentified,
        repeat: false,
        key: KeyCode::Character(" ".to_owned()),
        state: KeyState::Released,
        modifiers: Modifiers::default(),
    };
    app.runtime_mut().push_input_event(space_press);
    app.runtime_mut().push_input_event(space_release);
    app.settle();

    assert!(
        expanded.snapshot(),
        "Space key must expand focused disclosure"
    );
    assert_eq!(
        body_builds.get(),
        1,
        "expanding must lazily build the body once"
    );
    assert!(node_label_exists(app.tree(), "Hidden details"));
    let header_id = node_id_with_role(app.tree(), Role::Button, "Advanced");
    assert_eq!(node(app.tree(), header_id).is_expanded(), Some(true));

    // Enter collapses the focused disclosure again.
    app.press_key("Enter");
    assert!(
        !expanded.snapshot(),
        "Enter key must collapse focused disclosure"
    );
    assert!(!node_label_exists(app.tree(), "Hidden details"));
    let header_id = node_id_with_role(app.tree(), Role::Button, "Advanced");
    assert_eq!(node(app.tree(), header_id).is_expanded(), Some(false));

    // Pointer tap expands, then a11y Collapse/Expand actions drive it too.
    app.tap_node(header_id, 0.5, 0.5);
    assert!(
        expanded.snapshot(),
        "clicking the header must expand disclosure"
    );
    assert_eq!(body_builds.get(), 2, "re-expanding builds lazy body fresh");

    let header_id = node_id_with_role(app.tree(), Role::Button, "Advanced");
    app.perform_action(header_id, AccessibilityAction::Collapse, None);
    assert!(
        !expanded.snapshot(),
        "a11y Collapse must collapse disclosure"
    );
    let header_id = node_id_with_role(app.tree(), Role::Button, "Advanced");
    app.perform_action(header_id, AccessibilityAction::Expand, None);
    assert!(expanded.snapshot(), "a11y Expand must expand disclosure");
}

#[test]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped non-negative accessibility bounds fit pixel coordinates"
)]
fn disclosure_renders_visible_indicator_that_updates_with_expanded_state() {
    let expanded = Binding::bool(false);
    let body_builds = Rc::new(Cell::new(0_usize));
    let view = disclosure_view(&expanded, body_builds);
    let mut app = mount_rendered(Environment::new(), view, 400, 300);
    app.settle();

    let header_bounds = node_bounds_with_role(app.tree(), Role::Button, "Advanced");
    let collapsed = app.snapshot();
    let collapsed_rgba = collapsed.rgba8().to_vec();
    let width = collapsed.width() as usize;

    // Leading indicator lives in the left ~28pt of the header row.
    let ind_left = (header_bounds.x0.max(0.0) as usize).min(width);
    let ind_right = ((header_bounds.x0 + 28.0) as usize).min(width);
    let ind_top = (header_bounds.y0.max(0.0) as usize).min(collapsed.height() as usize);
    let ind_bottom = (header_bounds.y1 as usize).min(collapsed.height() as usize);

    let bg = &collapsed_rgba[0..4];
    let mut ink_pixels = 0_usize;
    for y in ind_top..ind_bottom {
        for x in ind_left..ind_right {
            let off = (y * width + x) * 4;
            if collapsed_rgba[off..off + 4] != *bg {
                ink_pixels += 1;
            }
        }
    }
    assert!(
        ink_pixels > 4,
        "disclosure header must draw a visible chevron indicator (got {ink_pixels} pixels)"
    );

    expanded.set(true);
    app.settle();
    let expanded_snap = app.snapshot();
    let expanded_rgba = expanded_snap.rgba8();
    let mut diff_pixels = 0_usize;
    for y in ind_top..ind_bottom {
        for x in ind_left..ind_right {
            let off = (y * width + x) * 4;
            if collapsed_rgba[off..off + 4] != expanded_rgba[off..off + 4] {
                diff_pixels += 1;
            }
        }
    }
    assert!(
        diff_pixels > 4,
        "indicator pixels must change between collapsed and expanded states (got {diff_pixels} diff pixels)"
    );
}

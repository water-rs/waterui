//! End-to-end accessibility coverage for the Material Design 3 theme package.

use core::convert::TryFrom as _;
use core::time::Duration;

use hydrolysis_m3::{
    assist_chip, dialog, dialog_action, extended_fab, fab, filled_icon_button, filter_chip,
    icon_button, input_chip, install, material_badge, material_card, material_list,
    material_list_item, material_menu, material_menu_divider, material_menu_item,
    material_navigation_view, material_sub_menu, material_tab, material_tabs, navigation_bar,
    navigation_drawer, navigation_drawer_item, navigation_tab, outlined_icon_button,
    outlined_segmented_button, outlined_segmented_button_set, plain_tooltip, rich_tooltip,
    suggestion_chip,
};
use waterui::ViewExt as _;
use waterui::component::{hstack, text, vstack};
use waterui::env::Environment;
use waterui::graphics::color::Srgb;
use waterui::id::Id;
use waterui::navigation::NavigationView;
use waterui::{Binding, Str};
use waterui_controls::{TextField, button, slider::slider, stepper::stepper, toggle};
use waterui_core::View;
use waterui_testing::{OffscreenApp, Role, Selector, SemanticApp, WaitOptions, WaitResult, ui};

fn mount_m3<V, F>(build: F) -> SemanticApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let mut env = Environment::new();
    install(&mut env);

    ui().environment(env).viewport(360, 320).mount(move || {
        vstack((build(),))
            .spacing(12.0)
            .padding_with(16.0)
            .background(Srgb::WHITE)
    })
}

fn mount_m3_fullscreen<V, F>(build: F) -> SemanticApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let mut env = Environment::new();
    install(&mut env);

    ui().environment(env).viewport(360, 320).mount(build)
}

fn mount_m3_offscreen<V, F>(build: F) -> OffscreenApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    ui().viewport(360, 320).theme(install).mount(move || {
        vstack((build(),))
            .spacing(12.0)
            .padding_with(16.0)
            .background(Srgb::WHITE)
    })
}

fn tab_id(value: i32) -> Id {
    Id::try_from(value).expect("test tab id must be non-zero")
}

#[test]
fn material_controls_expose_accessibility_semantics() {
    let enabled = Binding::bool(true);
    let enabled_for_view = enabled;
    let value = Binding::f64(0.32);
    let value_for_view = value;
    let count = Binding::i32(2);
    let count_for_view = count;
    let name = Binding::container(Str::from("Hydrolysis"));
    let name_for_view = name;

    let mut app = mount_m3(move || {
        vstack((
            button("Save").action(|| {}),
            toggle("Wi-Fi", &enabled_for_view),
            slider("Volume", &value_for_view),
            stepper("Quantity", &count_for_view),
            TextField::new(&name_for_view).label("Project"),
        ))
        .spacing(12.0)
    });

    app.query().role(Role::BUTTON).label("Save").assert_exists();
    app.query()
        .role(Role::SWITCH)
        .label("Wi-Fi")
        .checked(true)
        .assert_exists();
    app.query()
        .role(Role::SLIDER)
        .label("Volume")
        .assert_exists();
    app.query().label("Quantity").value("2").assert_exists();
    app.query()
        .role(Role::TEXT_INPUT)
        .label("Project")
        .value("Hydrolysis")
        .assert_exists();
}

#[test]
fn material_text_field_focus_is_routed_through_hydrolysis_accessibility_tree() {
    let name = Binding::container(Str::from(""));
    let name_for_view = name;
    let mut app = mount_m3(move || TextField::new(&name_for_view).label("Search"));

    let selector = Selector::default().role(Role::TEXT_INPUT).label("Search");
    assert!(
        app.query().role(Role::TEXT_INPUT).label("Search").focus(),
        "material text field focus should be routable through waterui-testing"
    );
    app.assert_ui_focus(&selector);
}

#[test]
fn material_assist_chip_exposes_button_semantics_and_tap_action() {
    let tapped = Binding::bool(false);
    let tapped_for_view = tapped.clone();
    let tapped_for_action = tapped;
    let mut app = mount_m3(move || {
        assist_chip("Assist").action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Assist")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Assist").tap(),
        "material assist chip should route tap actions through Hydrolysis gestures"
    );
    assert!(tapped_for_view.get(), "assist chip tap should update state");
}

#[test]
fn material_suggestion_chip_exposes_button_semantics_and_tap_action() {
    let tapped = Binding::bool(false);
    let tapped_for_view = tapped.clone();
    let tapped_for_action = tapped;
    let mut app = mount_m3(move || {
        suggestion_chip("Suggestion").action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Suggestion")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Suggestion").tap(),
        "material suggestion chip should route tap actions through Hydrolysis gestures"
    );
    assert!(
        tapped_for_view.get(),
        "suggestion chip tap should update state"
    );
}

#[test]
fn material_filter_chip_toggles_selection_and_exposes_button_semantics() {
    let selected = Binding::bool(false);
    let selected_for_view = selected.clone();
    let tapped = Binding::bool(false);
    let tapped_for_action = tapped.clone();
    let mut app = mount_m3(move || {
        filter_chip("Filter", &selected_for_view).action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Filter")
        .selected(false)
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Filter").tap(),
        "material filter chip should route tap actions through Hydrolysis gestures"
    );
    assert!(
        selected.get(),
        "filter chip tap should toggle selected state"
    );
    assert!(tapped.get(), "filter chip tap should invoke user action");
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::BUTTON)
                    .label("Filter")
                    .selected(true),
            )],
            WaitOptions::new(Duration::from_millis(200)),
        ) == WaitResult::Completed,
        "filter chip should expose selected state after tap"
    );
}

/// The plain `button()` control is the most-used Material component and had no
/// coverage of its own. A Material button is a container with a label inside it,
/// so its box must be at least the 40dp minimum height and wider than the label
/// by the style's horizontal padding — a button rendered as a bare label would
/// collapse to text metrics and satisfy neither.
#[test]
fn material_button_reserves_its_container_box_for_every_style() {
    const MIN_HEIGHT: f32 = 40.0;
    // The narrowest label still has to clear min-width plus the container's own
    // horizontal padding on both sides.
    const CONTAINER_PADDING_X: f32 = 24.0;

    let mut app = mount_m3(|| {
        vstack((
            button("Filled"),
            button("Outlined").bordered(),
            button("Prominent").bordered_prominent(),
        ))
        .spacing(8.0)
    });

    let mut previous_bottom = f32::MIN;
    for label in ["Filled", "Outlined", "Prominent"] {
        let bounds = app.query().role(Role::BUTTON).label(label).single().bounds();
        assert!(
            bounds.height() >= MIN_HEIGHT,
            "{label} button is {}pt tall, below the {MIN_HEIGHT}pt Material minimum — \
             a button drawn as a bare label collapses to its text height",
            bounds.height()
        );
        assert!(
            bounds.width() >= 2.0 * CONTAINER_PADDING_X,
            "{label} button is {}pt wide, narrower than its own horizontal padding",
            bounds.width()
        );
        assert!(
            bounds.y() >= previous_bottom,
            "{label} button overlaps the one above it"
        );
        previous_bottom = bounds.y() + bounds.height();
    }
}

#[test]
fn material_filter_chip_selected_state_changes_intrinsic_layout() {
    let unselected = Binding::bool(false);
    let unselected_for_view = unselected;
    let already_selected = Binding::bool(true);
    let mut unselected_app = mount_m3(move || {
        hstack((
            filter_chip("Filter", &unselected_for_view),
            filter_chip("Selected", &already_selected),
        ))
        .spacing(8.0)
    });

    let initial_filter_bounds = unselected_app
        .query()
        .role(Role::BUTTON)
        .label("Filter")
        .single()
        .bounds();
    let initial_selected_bounds = unselected_app
        .query()
        .role(Role::BUTTON)
        .label("Selected")
        .single()
        .bounds();

    let selected = Binding::bool(true);
    let selected_for_view = selected;
    let already_selected = Binding::bool(true);
    let mut selected_app = mount_m3(move || {
        hstack((
            filter_chip("Filter", &selected_for_view),
            filter_chip("Selected", &already_selected),
        ))
        .spacing(8.0)
    });

    let selected_filter_bounds = selected_app
        .query()
        .role(Role::BUTTON)
        .label("Filter")
        .single()
        .bounds();
    let shifted_selected_bounds = selected_app
        .query()
        .role(Role::BUTTON)
        .label("Selected")
        .single()
        .bounds();

    assert!(
        selected_filter_bounds.width() > initial_filter_bounds.width(),
        "selected filter chip should grow to include the leading checkmark"
    );
    assert!(
        shifted_selected_bounds.x() > initial_selected_bounds.x(),
        "sibling chip should shift after filter chip selection changes intrinsic width"
    );
}

#[test]
fn material_input_chip_exposes_primary_and_remove_button_semantics() {
    let primary_tapped = Binding::bool(false);
    let remove_tapped = Binding::bool(false);
    let primary_for_action = primary_tapped.clone();
    let remove_for_action = remove_tapped.clone();
    let mut app = mount_m3(move || {
        input_chip("Person")
            .action({
                let primary_for_action = primary_for_action.clone();
                move || primary_for_action.set(true)
            })
            .remove_action({
                let remove_for_action = remove_for_action.clone();
                move || remove_for_action.set(true)
            })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Person")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Remove Person")
        .assert_exists();

    assert!(
        app.query().role(Role::BUTTON).label("Person").tap(),
        "material input chip primary action should be tappable"
    );
    assert!(
        primary_tapped.get(),
        "input chip primary action should update state"
    );
    assert!(
        app.query().role(Role::BUTTON).label("Remove Person").tap(),
        "material input chip remove action should be tappable"
    );
    assert!(
        remove_tapped.get(),
        "input chip remove action should update state"
    );
}

#[test]
fn material_fab_exposes_button_semantics_and_tap_action() {
    let tapped = Binding::bool(false);
    let tapped_for_action = tapped.clone();
    let mut app = mount_m3(move || {
        fab("Create", text("+")).action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Create")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Create").tap(),
        "material FAB should route tap actions through Hydrolysis gestures"
    );
    assert!(tapped.get(), "FAB tap should update state");
}

#[test]
fn material_extended_fab_exposes_button_semantics_and_tap_action() {
    let tapped = Binding::bool(false);
    let tapped_for_action = tapped.clone();
    let mut app = mount_m3(move || {
        extended_fab("Create").action({
            let tapped_for_action = tapped_for_action.clone();
            move || tapped_for_action.set(true)
        })
    });

    app.query()
        .role(Role::BUTTON)
        .label("Create")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Create").tap(),
        "material extended FAB should route tap actions through Hydrolysis gestures"
    );
    assert!(tapped.get(), "extended FAB tap should update state");
}

#[test]
fn material_icon_buttons_expose_button_semantics_and_tap_actions() {
    let standard_tapped = Binding::bool(false);
    let filled_tapped = Binding::bool(false);
    let outlined_tapped = Binding::bool(false);
    let standard_for_action = standard_tapped.clone();
    let filled_for_action = filled_tapped.clone();
    let outlined_for_action = outlined_tapped.clone();
    let mut app = mount_m3(move || {
        hstack((
            icon_button("Favorite", text("*")).action({
                let standard_for_action = standard_for_action.clone();
                move || standard_for_action.set(true)
            }),
            filled_icon_button("Create", text("+")).action({
                let filled_for_action = filled_for_action.clone();
                move || filled_for_action.set(true)
            }),
            outlined_icon_button("More", text("...")).action({
                let outlined_for_action = outlined_for_action.clone();
                move || outlined_for_action.set(true)
            }),
        ))
        .spacing(8.0)
    });

    for label in ["Favorite", "Create", "More"] {
        app.query().role(Role::BUTTON).label(label).assert_exists();
        assert!(
            app.query().role(Role::BUTTON).label(label).tap(),
            "material icon button should route tap actions through Hydrolysis gestures"
        );
    }
    assert!(
        standard_tapped.get(),
        "standard icon button tap should update state"
    );
    assert!(
        filled_tapped.get(),
        "filled icon button tap should update state"
    );
    assert!(
        outlined_tapped.get(),
        "outlined icon button tap should update state"
    );
}

#[test]
fn material_tooltips_expose_accessibility_labels_and_action_semantics() {
    let action_tapped = Binding::bool(false);
    let action_for_view = action_tapped.clone();
    let mut app = mount_m3(move || {
        vstack((
            plain_tooltip("Adds this item to your favorites"),
            rich_tooltip("Favorite", "Adds this item to your favorites").action("Got it", {
                let action_for_view = action_for_view.clone();
                move || action_for_view.set(true)
            }),
        ))
        .spacing(12.0)
    });

    app.query()
        .label("Adds this item to your favorites")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Got it")
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Got it").tap(),
        "material rich tooltip action should route tap actions through Hydrolysis gestures"
    );
    assert!(
        action_tapped.get(),
        "rich tooltip action tap should update state"
    );
}

#[test]
fn anchored_material_tooltip_opens_on_keyboard_focus() {
    let mut app = mount_m3(move || {
        plain_tooltip("Adds this item to your favorites")
            .for_target(button("Favorite").action(|| {}))
    });

    app.query()
        .label("Adds this item to your favorites")
        .assert_not_exists();
    assert!(app.press_named_key("Tab"));
    app.query()
        .label("Adds this item to your favorites")
        .assert_exists();
    assert!(app.press_named_key("Escape"));
    app.query()
        .label("Adds this item to your favorites")
        .assert_not_exists();
}

#[test]
fn material_dialog_exposes_semantics_and_action_buttons() {
    let cancel_tapped = Binding::bool(false);
    let confirm_tapped = Binding::bool(false);
    let cancel_for_view = cancel_tapped.clone();
    let confirm_for_view = confirm_tapped.clone();
    let mut app = mount_m3(move || {
        dialog("Delete draft?", "This action cannot be undone.").actions((
            dialog_action("Cancel").action({
                let cancel_for_view = cancel_for_view.clone();
                move || cancel_for_view.set(true)
            }),
            dialog_action("Delete").action({
                let confirm_for_view = confirm_for_view.clone();
                move || confirm_for_view.set(true)
            }),
        ))
    });

    app.query().label("Delete draft?").assert_exists();
    app.query()
        .label("This action cannot be undone.")
        .assert_exists();
    for label in ["Cancel", "Delete"] {
        app.query().role(Role::BUTTON).label(label).assert_exists();
        assert!(
            app.query().role(Role::BUTTON).label(label).tap(),
            "material dialog action should route tap actions through Hydrolysis gestures"
        );
    }
    assert!(cancel_tapped.get(), "cancel action tap should update state");
    assert!(
        confirm_tapped.get(),
        "confirm action tap should update state"
    );
}

#[test]
fn material_navigation_bar_exposes_tab_semantics_and_selection() {
    let home_selected = Binding::bool(false);
    let search_selected = Binding::bool(true);
    let home_tapped = Binding::bool(false);
    let home_for_view = home_selected;
    let search_for_view = search_selected;
    let home_for_action = home_tapped.clone();
    let mut app = mount_m3(move || {
        navigation_bar((
            navigation_tab("Home", text("H"), &home_for_view).action({
                let home_for_action = home_for_action.clone();
                move || home_for_action.set(true)
            }),
            navigation_tab("Search", text("S"), &search_for_view),
        ))
    });

    app.query()
        .role(Role::TAB)
        .label("Home")
        .selected(false)
        .assert_exists();
    app.query()
        .role(Role::TAB)
        .label("Search")
        .selected(true)
        .assert_exists();
    assert!(
        app.query().role(Role::TAB).label("Home").tap(),
        "material navigation tab should route tap actions through Hydrolysis gestures"
    );
    assert!(home_tapped.get(), "navigation tab tap should update state");
}

#[test]
fn material_navigation_drawer_exposes_item_semantics_and_open_state() {
    let opened = Binding::bool(true);
    let inbox_selected = Binding::bool(true);
    let archive_selected = Binding::bool(false);
    let archive_tapped = Binding::bool(false);
    let opened_for_view = opened;
    let inbox_for_view = inbox_selected;
    let archive_for_view = archive_selected;
    let archive_for_action = archive_tapped.clone();
    let mut app = mount_m3(move || {
        navigation_drawer(
            &opened_for_view,
            vstack((
                navigation_drawer_item("Inbox", text("I"), &inbox_for_view),
                navigation_drawer_item("Archive", text("A"), &archive_for_view).action({
                    let archive_for_action = archive_for_action.clone();
                    move || archive_for_action.set(true)
                }),
            ))
            .spacing(4.0)
            .padding_with(12.0),
        )
    });

    app.query()
        .role(Role::BUTTON)
        .label("Inbox")
        .selected(true)
        .assert_exists();
    app.query()
        .label("Navigation drawer")
        .expanded(true)
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Archive")
        .selected(false)
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Archive").tap(),
        "material navigation drawer item should route tap actions through Hydrolysis gestures"
    );
    assert!(
        archive_tapped.get(),
        "navigation drawer item tap should update state"
    );
}

#[test]
fn material_modal_navigation_drawer_closes_from_escape_and_scrim() {
    let opened = Binding::bool(true);
    let opened_for_view = opened.clone();
    let overlay_tapped = Binding::bool(false);
    let overlay_for_view = overlay_tapped.clone();
    let mut app = mount_m3_fullscreen(move || {
        navigation_drawer(
            &opened_for_view,
            navigation_drawer_item("Inbox", text("I"), &Binding::bool(true)),
        )
        .modal()
        .close_on_escape()
        .close_on_overlay_click()
        .overlay_action({
            let overlay_for_view = overlay_for_view.clone();
            move || overlay_for_view.set(true)
        })
    });

    app.query()
        .label("Navigation drawer")
        .expanded(true)
        .assert_exists();
    assert!(app.pointer_down_at(330.0, 160.0));
    assert!(app.pointer_up_at(330.0, 160.0));
    assert!(overlay_tapped.get());
    assert!(!opened.get());
    assert!(app.wait_for_nonexistence(
        &Selector::default().label("Navigation drawer"),
        Duration::from_millis(250),
    ));

    opened.set(true);
    assert!(app.wait_for_existence(
        &Selector::default().label("Navigation drawer"),
        Duration::from_millis(250),
    ));
    assert!(app.press_named_key("Escape"));
    assert!(!opened.get());
}

#[test]
fn material_segmented_buttons_toggle_selection_and_expose_semantics() {
    let first_selected = Binding::bool(true);
    let second_selected = Binding::bool(false);
    let second_tapped = Binding::bool(false);
    let first_for_view = first_selected;
    let second_for_view = second_selected.clone();
    let second_for_action = second_tapped.clone();
    let mut app = mount_m3(move || {
        outlined_segmented_button_set((
            outlined_segmented_button("Day", &first_for_view).start(),
            outlined_segmented_button("Week", &second_for_view)
                .end()
                .action({
                    let second_for_action = second_for_action.clone();
                    move || second_for_action.set(true)
                }),
        ))
        .label("Range")
    });

    app.query()
        .role(Role::BUTTON)
        .label("Day")
        .selected(true)
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Week")
        .selected(false)
        .assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("Week").tap(),
        "segmented button tap should route through Hydrolysis gestures"
    );
    assert!(
        second_selected.get(),
        "segmented button tap should toggle its binding"
    );
    assert!(
        second_tapped.get(),
        "segmented button tap should invoke its action"
    );
}

#[test]
fn material_list_exposes_list_item_semantics_and_actions() {
    let reports_tapped = Binding::bool(false);
    let reports_for_action = reports_tapped.clone();
    let mut app = mount_m3(move || {
        material_list((
            material_list_item("Inbox")
                .supporting_text("3 new messages")
                .leading(text("I"))
                .trailing_supporting_text("Now"),
            material_list_item("Reports").trailing(text(">")).action({
                let reports_for_action = reports_for_action.clone();
                move || reports_for_action.set(true)
            }),
        ))
    });

    app.query().role(Role::LIST).assert_exists();
    let items = app.query().role(Role::LIST_ITEM).all();
    assert_eq!(items.len(), 2);
    assert!(
        items[1].tap_at(&mut app, 0.5, 0.5),
        "interactive material list item should route tap actions through Hydrolysis gestures"
    );
    assert!(
        reports_tapped.get(),
        "material list item tap should invoke its action"
    );
}

#[test]
fn material_tabs_expose_tab_semantics_and_switch_content() {
    let first = tab_id(1);
    let second = tab_id(2);
    let selection = Binding::container(first);
    let selection_for_view = selection.clone();
    let mut app = mount_m3(move || {
        material_tabs(
            &selection_for_view,
            vec![
                material_tab(first, "Photos", || {
                    NavigationView::new("Photos", text("photos content"))
                }),
                material_tab(second, "Albums", || {
                    NavigationView::new("Albums", text("albums content"))
                }),
            ],
        )
    });

    app.query().role(Role::TAB_LIST).assert_exists();
    app.query()
        .role(Role::TAB)
        .label("Photos")
        .selected(true)
        .assert_exists();
    app.query()
        .role(Role::TAB)
        .label("Albums")
        .selected(false)
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("photos content")
        .assert_exists();
    assert!(
        app.query().role(Role::TAB).label("Albums").tap(),
        "material tabs should route tab selection through Hydrolysis accessibility"
    );
    assert_eq!(selection.get(), second);
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .role(Role::LABEL)
                    .label("albums content"),
            )],
            WaitOptions::new(Duration::from_millis(200)),
        ) == WaitResult::Completed,
        "material tabs should show selected tab content"
    );
}

#[test]
fn material_card_exposes_group_semantics_and_preserves_content() {
    let mut app = mount_m3(|| {
        material_card(vstack((text("Project"), text("Material card content"))))
            .outlined()
            .label("Project card")
    });

    app.query().label("Project card").assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Material card content")
        .assert_exists();
}

#[test]
fn material_badge_preserves_badged_content_semantics() {
    let mut app = mount_m3(|| material_badge(3, text("Inbox")).label("Inbox, 3 new notifications"));

    app.query()
        .role(Role::LABEL)
        .label("Inbox, 3 new notifications")
        .assert_exists();
}

#[test]
fn material_menu_exposes_trigger_semantics() {
    let mut app = mount_m3(|| {
        material_menu(
            "Actions",
            (
                material_menu_item("Refresh").action(|| {}),
                material_menu_divider(),
                material_sub_menu("More", (material_menu_item("Archive").action(|| {}),)),
            ),
        )
    });

    let menu = app.query().role(Role::BUTTON).label("Actions").single();
    let bounds = menu.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "material menu trigger should expose non-zero button semantics"
    );
}

#[test]
fn material_navigation_view_preserves_title_and_content_semantics() {
    let mut app = mount_m3(|| material_navigation_view("Inbox", text("Primary content")));

    app.query().label("Inbox").assert_exists();
}

#[test]
#[ignore = "captures a real Hydrolysis focused text field PNG for direct visual review"]
fn material_focused_text_field_snapshot() {
    let name = Binding::container(Str::from("Hydrolysis"));
    let name_for_view = name;
    let mut app = mount_m3_offscreen(move || TextField::new(&name_for_view).label("Project"));

    assert!(
        app.query().role(Role::TEXT_INPUT).label("Project").focus(),
        "material text field should accept focus before visual capture"
    );
    let captured = app.capture_snapshot("material3-preview", "text-field-focused-caret", "focused");

    assert!(captured.path().is_file());
}

#[test]
fn material_collection_items_expose_accessibility_and_survive_membership_change() {
    use waterui::Identifiable;
    use waterui::component::ZStack;
    use waterui::reactive::collection::List;

    #[derive(Clone, Identifiable)]
    struct Row {
        #[id]
        id: u64,
        label: Str,
    }

    let list = List::from(vec![
        Row {
            id: 1,
            label: Str::from("Alpha"),
        },
        Row {
            id: 2,
            label: Str::from("Bravo"),
        },
        Row {
            id: 3,
            label: Str::from("Charlie"),
        },
    ]);
    let list_for_action = list.clone();

    // `ZStack::for_each` lowers to a non-virtualized `LazyContainer`, i.e. the
    // retained per-id collection path. Each row is a `text` label, so every item
    // must surface a LABEL node in the Hydrolysis accessibility tree.
    let mut app = mount_m3(move || {
        let list_for_action = list_for_action.clone();
        vstack((
            button("Remove Bravo").action(move || {
                if let Some(index) = list_for_action
                    .snapshot()
                    .iter()
                    .position(|row| row.id == 2)
                {
                    let _ = list_for_action.remove(index);
                }
            }),
            ZStack::for_each(list.clone(), |row: Row| text(row.label)),
        ))
    });

    for label in ["Alpha", "Bravo", "Charlie"] {
        app.query().role(Role::LABEL).label(label).assert_exists();
    }

    // Removing one item reconciles the collection by id: only that row's
    // accessibility node leaves the tree; the survivors keep theirs.
    assert!(
        app.query().role(Role::BUTTON).label("Remove Bravo").tap(),
        "remove button should route its tap through Hydrolysis gestures"
    );
    let bravo = Selector::default()
        .role(Role::LABEL)
        .label("Bravo".to_owned());
    assert!(
        app.wait_for_nonexistence(&bravo, Duration::from_secs(1)),
        "the removed collection item must leave the accessibility tree"
    );
    app.query().role(Role::LABEL).label("Alpha").assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Charlie")
        .assert_exists();
}

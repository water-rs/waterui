//! `WaterUI` Control Catalog — a Material Design 3 component gallery.
//!
//! The layout is a navigation drawer beside a detail pane (a [`NavigationSplitView`],
//! which collapses the drawer to a rail on narrow windows). The drawer lists the
//! framework's controls grouped by category; each group header expands or collapses
//! its items. Selecting a control shows it live and interactive in the detail area —
//! including its style/variant options — so the catalog demonstrates each control's
//! behavior directly.
//!
//! Interactive demo state lives in `DemoState`, owned at the window scope and
//! threaded into the demos. The split view rebuilds its detail builder on render,
//! so any state created *inside* a demo would reset; owning it externally keeps the
//! controls live (a button's counter actually counts, a toggle stays toggled).

use mdi::Icon;
use mdi::form_textbox;
use mdi::format_list_bulleted;
use mdi::gauge;
use mdi::gesture_tap_button;
use mdi::home;
use mdi::numeric;
use mdi::text_box_outline;
use mdi::toggle_switch;
use mdi::tune;
use waterui::app::{App, Windows};
use waterui::navigation::{NavigationSplitView, NavigationView};
use waterui::prelude::slider::slider;
use waterui::prelude::stepper::stepper;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::theme::color::Foreground;
use waterui_icons_material_icon as mdi;

/// A category grouping in the navigation sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Actions,
    Inputs,
    Selection,
    Display,
}

impl Category {
    const ALL: [Self; 4] = [Self::Actions, Self::Inputs, Self::Selection, Self::Display];

    const fn title(self) -> &'static str {
        match self {
            Self::Actions => "Actions",
            Self::Inputs => "Inputs",
            Self::Selection => "Selection",
            Self::Display => "Display",
        }
    }
}

/// The control indices belonging to a sidebar group, in catalog order.
fn group_item_indices(group: usize) -> Vec<usize> {
    let category = Category::ALL[group];
    controls()
        .iter()
        .enumerate()
        .filter(|(_, control)| control.category == category)
        .map(|(index, _)| index)
        .collect()
}

/// Reactive state for every interactive demo, owned at the window scope so it
/// survives the split view rebuilding its detail.
#[derive(Clone)]
struct DemoState {
    taps: Binding<i32>,
    wifi: Binding<bool>,
    bluetooth: Binding<bool>,
    volume: Binding<f64>,
    quantity: Binding<i32>,
    name: Binding<Str>,
    size: Binding<&'static str>,
}

impl DemoState {
    fn new() -> Self {
        Self {
            taps: binding(0),
            wifi: binding(true),
            bluetooth: binding(false),
            volume: binding(40.0),
            quantity: binding(2),
            name: binding(Str::from("")),
            size: Binding::new("Medium"),
        }
    }
}

/// A single catalog entry: a control with its drawer icon and live demo.
struct Control {
    title: &'static str,
    icon: fn() -> Icon,
    category: Category,
    demo: fn(&DemoState) -> AnyView,
}

/// The catalog, in drawer order.
fn controls() -> Vec<Control> {
    vec![
        Control {
            title: "Buttons",
            icon: gesture_tap_button,
            category: Category::Actions,
            demo: |s| AnyView::new(buttons_demo(&s.taps)),
        },
        Control {
            title: "Text Field",
            icon: form_textbox,
            category: Category::Inputs,
            demo: |s| AnyView::new(text_field_demo(&s.name)),
        },
        Control {
            title: "Slider",
            icon: tune,
            category: Category::Inputs,
            demo: |s| AnyView::new(slider_demo(&s.volume)),
        },
        Control {
            title: "Stepper",
            icon: numeric,
            category: Category::Inputs,
            demo: |s| AnyView::new(stepper_demo(&s.quantity)),
        },
        Control {
            title: "Toggle",
            icon: toggle_switch,
            category: Category::Selection,
            demo: |s| AnyView::new(toggle_demo(&s.wifi, &s.bluetooth)),
        },
        Control {
            title: "Picker",
            icon: format_list_bulleted,
            category: Category::Selection,
            demo: |s| AnyView::new(picker_demo(&s.size)),
        },
        Control {
            title: "Label",
            icon: text_box_outline,
            category: Category::Display,
            demo: |_| AnyView::new(label_demo()),
        },
        Control {
            title: "Progress",
            icon: gauge,
            category: Category::Display,
            demo: |_| AnyView::new(progress_demo()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Layout: collapsible grouped navigation sidebar + detail
// ---------------------------------------------------------------------------

/// The whole catalog: a single, stable [`NavigationSplitView`] (no `watch` at the
/// root, so it is never rebuilt as a unit). The split places the sidebar and detail
/// panes and collapses the sidebar to a rail on narrow windows.
fn catalog(
    selected: &Binding<Option<usize>>,
    groups_open: Vec<Binding<bool>>,
    state: DemoState,
) -> impl View + use<> {
    let sidebar_selected = selected.clone();
    NavigationSplitView::new(
        selected,
        move || sidebar(&sidebar_selected, &groups_open),
        move |index| control_detail(index, &state),
    )
    .sidebar_width(ColumnWidth::new(240.0, 320.0, 480.0))
    .placeholder(placeholder)
}

/// Sidebar: a native collapsible-section [`List`] bound to `selected` and each
/// category's `groups_open` binding.
fn sidebar(selected: &Binding<Option<usize>>, groups_open: &[Binding<bool>]) -> impl View + use<> {
    let section_for = |group: usize| {
        let category = Category::ALL[group];
        let rows: Vec<_> = group_item_indices(group)
            .into_iter()
            .map(|index| {
                let control = &controls()[index];
                let title = control.title;
                let icon = control.icon;
                (move || hstack((icon().width(20.0).height(20.0), text(title))).spacing(10.0))
                    .tag(index)
            })
            .collect();
        Section::new(category.title())
            .expanded(&groups_open[group])
            .content(rows)
    };

    List::content((
        section_for(0),
        section_for(1),
        section_for(2),
        section_for(3),
    ))
    .selection(selected)
}

/// The detail pane for the selected control: its title (in the navigation bar)
/// and its live demo, bound to the shared [`DemoState`].
fn control_detail(index: usize, state: &DemoState) -> NavigationView<impl View + use<>> {
    let all = controls();
    let control = &all[index];
    NavigationView::new(control.title, (control.demo)(state).padding_with(20.0))
}

/// Shown before any control is selected (wide layouts).
fn placeholder() -> impl View {
    vstack((
        text("WaterUI Controls").title().foreground(Foreground),
        text("Choose a control from the navigation drawer to see it live.")
            .body()
            .muted(),
    ))
    .spacing(10.0)
    .padding()
}

/// Shared intro line for a demo.
fn note(text_value: &'static str) -> impl View {
    text(text_value).body().muted()
}

// ---------------------------------------------------------------------------
// Control demos (state is passed in, never created here)
// ---------------------------------------------------------------------------

/// Increments a shared tap counter; reused by every button style.
fn bump(State(taps): State<Binding<i32>>) {
    *taps.get_mut() += 1;
}

fn buttons_demo(taps: &Binding<i32>) -> impl View {
    vstack((
        note("Every button shares one action. ButtonStyle controls the appearance."),
        text!("Taps: {taps}").body(),
        button("Automatic").action(bump).state(taps),
        button("Bordered").bordered().action(bump).state(taps),
        button("Bordered Prominent")
            .bordered_prominent()
            .action(bump)
            .state(taps),
        button("Plain").plain().action(bump).state(taps),
        button("Borderless").borderless().action(bump).state(taps),
        button("Link").link().action(bump).state(taps),
    ))
    .spacing(12.0)
    .leading()
}

fn toggle_demo(wifi: &Binding<bool>, bluetooth: &Binding<bool>) -> impl View {
    vstack((
        note("ToggleStyle switches between a switch and a checkbox."),
        Toggle::new("Wi-Fi", wifi).style(ToggleStyle::Switch),
        Toggle::new("Bluetooth", bluetooth).style(ToggleStyle::Checkbox),
        text!("Wi-Fi {wifi} · Bluetooth {bluetooth}").body(),
    ))
    .spacing(12.0)
    .leading()
}

fn slider_demo(volume: &Binding<f64>) -> impl View {
    let sizes = [
        ("Extra small", ControlSize::ExtraSmall),
        ("Small", ControlSize::Small),
        ("Medium", ControlSize::Medium),
        ("Large", ControlSize::Large),
        ("Extra large", ControlSize::ExtraLarge),
    ]
    .map(|(label, size)| {
        slider(label, volume)
            .range(0.0..=100.0)
            .size(size)
            .value_formatter(|v| Str::from(format!("{v:.0}")))
    });

    vstack((
        note("Drag a slider; the value indicator follows the thumb and the progress bar reflects the value."),
        slider("Volume", volume).range(0.0..=100.0),
        vstack(sizes).spacing(8.0),
        text!("Value: {volume}").body(),
        progress(volume.clone().map(|v| v / 100.0)).label("Volume"),
    ))
    .spacing(12.0)
    .leading()
}

fn stepper_demo(quantity: &Binding<i32>) -> impl View {
    vstack((
        note("Stepper adjusts an integer within a range."),
        stepper("Quantity", quantity).range(0..=10),
        text!("Quantity: {quantity}").body(),
    ))
    .spacing(12.0)
    .leading()
}

fn text_field_demo(name: &Binding<Str>) -> impl View {
    vstack((
        note("TextField binds to reactive text and echoes it live."),
        TextField::new("Name", name).prompt("Type your name"),
        text!("Echo: {name}").body(),
    ))
    .spacing(12.0)
    .leading()
}

fn size_items() -> Vec<PickerItem<&'static str>> {
    vec![
        text("Small").tag("Small"),
        text("Medium").tag("Medium"),
        text("Large").tag("Large"),
    ]
}

fn picker_demo(size: &Binding<&'static str>) -> impl View {
    vstack((
        note("PickerStyle renders the same selection as segmented, menu, or radio."),
        text("Segmented").subheadline(),
        picker("Size", size_items(), size).style(PickerStyle::Segmented),
        text("Menu").subheadline(),
        picker("Size", size_items(), size).style(PickerStyle::Menu),
        text("Radio").subheadline(),
        picker("Size", size_items(), size).style(PickerStyle::Radio),
        text!("Selected: {size}").body(),
    ))
    .spacing(12.0)
    .leading()
}

fn label_demo() -> impl View {
    vstack((
        note("LabelDisplayMode controls whether the title, icon, or both show."),
        label("Title and Icon")
            .icon(home())
            .leading()
            .display_mode(LabelDisplayMode::TitleAndIcon),
        label("Title Only")
            .icon(home())
            .leading()
            .display_mode(LabelDisplayMode::TitleOnly),
        label("Icon Only")
            .icon(home())
            .leading()
            .display_mode(LabelDisplayMode::IconOnly),
    ))
    .spacing(12.0)
    .leading()
}

fn progress_demo() -> impl View {
    vstack((
        note("Progress shows determinate completion."),
        progress(0.25).label("Downloading"),
        progress(0.5).label("Halfway"),
        progress(0.75).label("Almost there"),
    ))
    .spacing(16.0)
    .leading()
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// The window-scoped catalog state: the selected control, one open-flag per
/// group, and the interactive demo state.
type CatalogState = (Binding<Option<usize>>, Vec<Binding<bool>>, DemoState);

/// Builds fresh catalog state: `selected` starts on the first control, every
/// group starts expanded, and the demo state starts at its defaults.
fn new_state() -> CatalogState {
    let selected = Binding::new(Some(0));
    let groups_open = Category::ALL.iter().map(|_| binding(true)).collect();
    (selected, groups_open, DemoState::new())
}

/// Self-contained entry for previews and embedding.
#[preview]
pub fn demo() -> impl View {
    let (selected, groups_open, state) = new_state();
    catalog(&selected, groups_open, state)
}

/// Builds the gallery app, owning its selection and drawer state at window scope.
#[must_use]
pub fn app(env: Environment) -> App<impl Windows> {
    // Own all mutable state at the window scope so it persists across rebuilds.
    let (selected, groups_open, state) = new_state();
    App::new(env).window(Window::new("", move || {
        catalog(&selected, groups_open.clone(), state.clone())
    }))
}

#[cfg(test)]
mod tests {
    use super::{catalog, demo, new_state};
    use core::time::Duration;
    use hydrolysis_m3::Material3;
    use waterui::Signal;
    use waterui::Str;
    use waterui_testing::{OffscreenApp, Role, UiBuilder};

    /// Selecting each control in turn shows its demo — a smoke test that every
    /// catalog entry resolves and renders (the per-component MD3 visual review was
    /// done out-of-band with GPU snapshots).
    #[waterui::test(viewport = (1100, 760))]
    fn every_control_renders_its_demo(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));
        let controls = [
            ("Text Field", "Name"),
            ("Slider", "Volume"),
            ("Stepper", "Quantity"),
            ("Toggle", "Bluetooth"),
            ("Picker", "Segmented"),
            ("Label", "Title Only"),
            ("Progress", "Downloading"),
            ("Buttons", "Bordered Prominent"),
        ];
        for (item, confirm) in controls {
            app.query().role(Role::LIST_ITEM).label(item).tap();
            assert!(
                app.query()
                    .label_contains(confirm)
                    .wait_for_existence(Duration::from_secs(3)),
                "demo for {item} should render (waiting for {confirm})"
            );
        }
    }

    /// The drawer lists every group header and control, and the detail pane shows
    /// the selected control's live demo.
    #[waterui::test(viewport = (1100, 760))]
    fn catalog_lists_controls(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));
        for section in ["Actions", "Inputs", "Selection", "Display"] {
            app.query().label(section).assert_exists();
        }
        for control in ["Buttons", "Toggle", "Slider", "Picker", "Label"] {
            app.query().label(control).assert_exists();
        }
        app.query()
            .label_contains("Bordered Prominent")
            .assert_exists();
    }

    /// Selecting a control shows its live demo in the detail pane.
    #[waterui::test(viewport = (1100, 760))]
    fn selecting_control_shows_demo(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));
        app.query().role(Role::LIST_ITEM).label("Toggle").tap();
        assert!(
            app.query()
                .label_contains("Bluetooth")
                .wait_for_existence(Duration::from_secs(3)),
            "the Toggle demo should render after selection"
        );
    }

    /// Collapsing a group removes its items (and releases their space), while the
    /// rest of the catalog stays intact.
    #[waterui::test(viewport = (1100, 760))]
    fn collapsing_group_removes_items(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));
        app.query().label("Slider").assert_exists();
        // Tap the Inputs group header to collapse it.
        app.query().label("Inputs").tap();
        assert!(
            app.query()
                .label("Slider")
                .wait_for_nonexistence(Duration::from_secs(3)),
            "collapsing a group should remove its items"
        );
        app.query().label("Buttons").assert_exists();
        app.query()
            .label_contains("Bordered Prominent")
            .assert_exists();
    }

    /// Re-expanding a collapsed group restores its items: the transition's enter
    /// path brings the rows back (and they settle into the accessibility tree).
    #[waterui::test(viewport = (1100, 760))]
    fn expanding_group_restores_items(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));
        // Collapse Inputs, then expand it again.
        app.query().label("Inputs").tap();
        assert!(
            app.query()
                .label("Slider")
                .wait_for_nonexistence(Duration::from_secs(3)),
            "collapsing should remove the group's items"
        );
        app.query().label("Inputs").tap();
        assert!(
            app.query()
                .label("Slider")
                .wait_for_existence(Duration::from_secs(3)),
            "re-expanding should restore the group's items"
        );
    }

    /// A demo control is actually interactive: tapping a button increments the
    /// shared counter (state is owned externally, so it persists across rebuilds).
    #[waterui::test(viewport = (1100, 760))]
    fn button_demo_is_interactive(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));
        app.query().label_contains("Taps: 0").assert_exists();
        app.query().label("Bordered Prominent").tap();
        assert!(
            app.query()
                .label_contains("Taps: 1")
                .wait_for_existence(Duration::from_secs(3)),
            "tapping a demo button should increment the counter"
        );
    }

    /// Selecting the Toggle demo and flipping its switch and checkbox through
    /// accessibility updates their `DemoState` bindings (`wifi`/`bluetooth`,
    /// which start on and off respectively) and their exposed checked state.
    /// The two styles report different accesskit roles (`Switch` vs.
    /// `CheckBox`), so this also pins that `ToggleStyle` maps to distinct
    /// accessibility semantics rather than one shared role.
    #[waterui::test(viewport = (1100, 760))]
    fn toggle_demo_flips_switch_and_checkbox_bindings(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let wifi = state.wifi.clone();
        let bluetooth = state.bluetooth.clone();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));

        app.query().role(Role::LIST_ITEM).label("Toggle").tap();
        assert!(
            app.query()
                .role(Role::SWITCH)
                .label("Wi-Fi")
                .wait_for_existence(Duration::from_secs(3)),
            "the Toggle demo should render its Wi-Fi switch"
        );

        app.query()
            .role(Role::SWITCH)
            .label("Wi-Fi")
            .checked(true)
            .assert_exists();
        app.query()
            .role(Role::CHECKBOX)
            .label("Bluetooth")
            .checked(false)
            .assert_exists();

        app.query().role(Role::SWITCH).label("Wi-Fi").tap();
        assert!(
            !wifi.snapshot(),
            "flipping the Wi-Fi switch should clear its binding"
        );
        app.query()
            .role(Role::SWITCH)
            .label("Wi-Fi")
            .checked(false)
            .assert_exists();

        app.query().role(Role::CHECKBOX).label("Bluetooth").tap();
        assert!(
            bluetooth.snapshot(),
            "tapping the Bluetooth checkbox should set its binding"
        );
        app.query()
            .role(Role::CHECKBOX)
            .label("Bluetooth")
            .checked(true)
            .assert_exists();
    }

    /// Selecting the Slider demo and driving it through accessibility
    /// increment/decrement moves `DemoState::volume` by the slider's a11y
    /// step (`(range.end() - range.start()) / 100.0`, which is `1.0` for the
    /// demo's `0.0..=100.0` range), and the live echo text tracks it.
    #[waterui::test(viewport = (1100, 760))]
    fn slider_demo_increment_decrement_updates_volume(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let volume = state.volume.clone();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));

        app.query().role(Role::LIST_ITEM).label("Slider").tap();
        assert!(
            app.query()
                .role(Role::SLIDER)
                .label("Volume")
                .wait_for_existence(Duration::from_secs(3)),
            "the Slider demo should render its Volume slider"
        );
        app.query()
            .role(Role::LABEL)
            .label_contains("Value: 40")
            .assert_exists();

        app.query().role(Role::SLIDER).label("Volume").increment();
        assert!(
            (volume.snapshot() - 41.0).abs() < 0.0001,
            "incrementing the slider should raise the volume binding by one a11y step"
        );
        assert!(
            app.query()
                .role(Role::LABEL)
                .label_contains("Value: 41")
                .wait_for_existence(Duration::from_secs(3)),
            "the echoed value should track the incremented binding"
        );

        app.query().role(Role::SLIDER).label("Volume").decrement();
        assert!(
            (volume.snapshot() - 40.0).abs() < 0.0001,
            "decrementing the slider should restore the previous volume"
        );
        assert!(
            app.query()
                .role(Role::LABEL)
                .label_contains("Value: 40")
                .wait_for_existence(Duration::from_secs(3)),
            "the echoed value should track the decremented binding"
        );
    }

    /// Selecting the Stepper demo and driving it through accessibility
    /// increment/decrement moves `DemoState::quantity` by the stepper's fixed
    /// step of `1`, clamped to its `0..=10` range, and the live echo text
    /// tracks it.
    #[waterui::test(viewport = (1100, 760))]
    fn stepper_demo_increment_decrement_updates_quantity(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let quantity = state.quantity.clone();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));

        app.query().role(Role::LIST_ITEM).label("Stepper").tap();
        assert!(
            app.query()
                .label("Quantity")
                .wait_for_existence(Duration::from_secs(3)),
            "the Stepper demo should render its Quantity stepper"
        );

        app.query().label("Quantity").value("2").increment();
        assert_eq!(
            quantity.snapshot(),
            3,
            "incrementing the stepper should raise its binding by one"
        );
        assert!(
            app.query()
                .role(Role::LABEL)
                .label_contains("Quantity: 3")
                .wait_for_existence(Duration::from_secs(3)),
            "the echoed quantity should track the incremented binding"
        );

        app.query().label("Quantity").value("3").decrement();
        assert_eq!(
            quantity.snapshot(),
            2,
            "decrementing the stepper should restore the previous quantity"
        );
        assert!(
            app.query()
                .role(Role::LABEL)
                .label_contains("Quantity: 2")
                .wait_for_existence(Duration::from_secs(3)),
            "the echoed quantity should track the decremented binding"
        );
    }

    /// Selecting the Text Field demo and setting its text through
    /// accessibility updates `DemoState::name`, and the live echo text
    /// tracks it.
    #[waterui::test(viewport = (1100, 760))]
    fn text_field_demo_set_text_updates_name(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let name = state.name.clone();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));

        app.query().role(Role::LIST_ITEM).label("Text Field").tap();
        assert!(
            app.query()
                .role(Role::TEXT_INPUT)
                .label("Name")
                .wait_for_existence(Duration::from_secs(3)),
            "the Text Field demo should render its Name field"
        );

        app.query()
            .role(Role::TEXT_INPUT)
            .label("Name")
            .set_text("Ada Lovelace");
        assert_eq!(
            name.snapshot(),
            Str::from("Ada Lovelace"),
            "the text field should update its bound demo state"
        );
        assert!(
            app.query()
                .role(Role::LABEL)
                .label_contains("Echo: Ada Lovelace")
                .wait_for_existence(Duration::from_secs(3)),
            "the demo should echo the updated name live"
        );
    }

    /// A full navigation flow: the catalog opens on the first control
    /// (Buttons), so its title is the initial detail heading. Collapsing and
    /// re-expanding a group (Selection) restores its items, and selecting a
    /// control from the re-expanded group swaps the detail heading — which
    /// the [`NavigationView`](waterui::navigation::NavigationView) backend
    /// exposes as accesskit's `Header` role — to that control's title.
    #[waterui::test(viewport = (1100, 760))]
    fn expanding_a_collapsed_group_and_selecting_its_control_switches_the_heading(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app = ui.mount(move || catalog(&selected, groups_open.clone(), state.clone()));

        assert!(
            app.query()
                .role(Role::HEADER)
                .label("Buttons")
                .wait_for_existence(Duration::from_secs(3)),
            "the catalog should open on the Buttons demo heading"
        );

        // Collapse the Selection group (Toggle + Picker), then re-expand it.
        app.query().label("Selection").tap();
        assert!(
            app.query()
                .label("Toggle")
                .wait_for_nonexistence(Duration::from_secs(3)),
            "collapsing Selection should remove its items"
        );
        app.query().label("Selection").tap();
        assert!(
            app.query()
                .label("Toggle")
                .wait_for_existence(Duration::from_secs(3)),
            "re-expanding Selection should restore its items"
        );

        // Selecting Toggle from the re-expanded group swaps the detail heading.
        app.query().role(Role::LIST_ITEM).label("Toggle").tap();
        assert!(
            app.query()
                .role(Role::HEADER)
                .label("Toggle")
                .wait_for_existence(Duration::from_secs(3)),
            "the detail heading should switch to the selected control's title"
        );
        app.query()
            .role(Role::HEADER)
            .label("Buttons")
            .assert_not_exists();
        app.query()
            .role(Role::SWITCH)
            .label("Wi-Fi")
            .assert_exists();
    }

    /// Visual acceptance for the drawer: the selected row's active indicator and
    /// the Stepper demo's controls, both reviewed by eye.
    #[ignore = "writes visual acceptance PNG files for direct image review"]
    #[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (1100, 760))]
    fn catalog_drawer_and_stepper_render(ui: UiBuilder) {
        let (selected, groups_open, state) = new_state();
        let mut app: waterui_testing::OffscreenApp =
            ui.mount_offscreen(move || catalog(&selected, groups_open.clone(), state.clone()));
        app.query().role(Role::LIST_ITEM).label("Stepper").tap();
        assert!(
            app.query()
                .label_contains("Quantity")
                .wait_for_existence(Duration::from_secs(3)),
            "the Stepper demo should render before it is captured"
        );
        let _ = app.capture_snapshot("material3-preview", "gallery", "stepper-selected");
    }

    #[waterui::test(
        demo,
        theme = Material3::defaults(),
        viewport = (900, 600),
        offscreen
    )]
    fn catalog_buttons_demo_renders_non_blank_offscreen(app: &mut OffscreenApp) {
        app.query().label("Buttons").assert_exists();
        let snapshot = app.snapshot();
        assert_eq!(snapshot.width(), 900);
        assert_eq!(snapshot.height(), 600);
        let blank = snapshot.analyze_blank();
        assert!(
            !blank.is_blank(),
            "offscreen Buttons demo capture should not be blank: {blank:?}"
        );
    }
}

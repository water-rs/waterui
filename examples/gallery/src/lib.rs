//! WaterUI Control Catalog — a Material Design 3 component gallery.
//!
//! The layout is a navigation drawer beside a detail pane (a [`NavigationSplitView`],
//! which collapses the drawer to a rail on narrow windows). The drawer lists the
//! framework's controls grouped by category; each group header expands or collapses
//! its items. Selecting a control shows it live and interactive in the detail area —
//! including its style/variant options — so the catalog demonstrates each control's
//! behavior directly.
//!
//! Interactive demo state lives in [`DemoState`], owned at the window scope and
//! threaded into the demos. The split view rebuilds its detail builder on render,
//! so any state created *inside* a demo would reset; owning it externally keeps the
//! controls live (a button's counter actually counts, a toggle stays toggled).

use core::time::Duration;

use waterui::Color;
use waterui::Identifiable;
use waterui::animation::Animation;
use waterui::app::App;
use waterui::form::picker::{PickerStyle, picker};
use waterui::layout::HorizontalAlignment;
use waterui::layout::collection_transition;
use waterui::layout::stack::VStack;
use waterui::navigation::{NavigationSplitView, NavigationView};
use waterui::prelude::slider::slider;
use waterui::prelude::stepper::stepper;
use waterui::prelude::theme_color::{Foreground, MutedForeground, SurfaceVariant};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::Computed;
use waterui::reactive::binding;
use waterui::reactive::collection::List as ReactiveList;
use waterui::shape::Capsule;
use waterui_icons_material_icon as mdi;

// Material 3 navigation-drawer metrics (mdui reference: list-item / nav-drawer).
const DRAWER_ITEM_HEIGHT: f32 = 56.0;
const DRAWER_ICON_SIZE: f32 = 24.0;
const DRAWER_ITEM_PADDING: f32 = 16.0;
const DRAWER_ITEM_INDENT: f32 = 8.0; // control items nest slightly under their header

/// A category grouping in the navigation drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Actions,
    Inputs,
    Selection,
    Display,
}

impl Section {
    const ALL: [Self; 4] = [Self::Actions, Self::Inputs, Self::Selection, Self::Display];

    const fn title(self) -> &'static str {
        match self {
            Self::Actions => "Actions",
            Self::Inputs => "Inputs",
            Self::Selection => "Selection",
            Self::Display => "Display",
        }
    }

    const fn icon(self) -> fn() -> mdi::Svg {
        match self {
            Self::Actions => mdi::cursor_default_click,
            Self::Inputs => mdi::keyboard,
            Self::Selection => mdi::checkbox_marked,
            Self::Display => mdi::view_dashboard,
        }
    }
}

/// One row in the navigation drawer's reactive collection. Headers are always
/// present; an item is present only while its group is expanded. Membership
/// changes are diffed by [`Identifiable`] id, so collapsing a group truly
/// removes its rows and releases their layout space (no reserved gap).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Header(usize),
    Item(usize),
}

impl Identifiable for Row {
    type Id = u32;

    fn id(&self) -> u32 {
        // Headers occupy 0.. ; items live in a disjoint range so the two never
        // collide in the id-keyed reconcile.
        match *self {
            Self::Header(group) => group as u32,
            Self::Item(index) => 0x1000 + index as u32,
        }
    }
}

/// The control indices belonging to a drawer group, in catalog order.
fn group_item_indices(group: usize) -> Vec<usize> {
    let section = Section::ALL[group];
    controls()
        .iter()
        .enumerate()
        .filter(|(_, control)| control.section == section)
        .map(|(index, _)| index)
        .collect()
}

/// The fully expanded drawer order: every header followed by its items.
fn expanded_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    for group in 0..Section::ALL.len() {
        rows.push(Row::Header(group));
        rows.extend(group_item_indices(group).into_iter().map(Row::Item));
    }
    rows
}

/// Expands or collapses a group by reconciling the reactive row collection:
/// inserting the group's item rows right after its header, or removing them.
/// The collection engine diffs by id, so only the toggled group's rows change.
fn set_group_expanded(rows: &ReactiveList<Row>, group: usize, expanded: bool) {
    let items = group_item_indices(group);
    if expanded {
        let snapshot = rows.snapshot();
        let header_pos = snapshot
            .iter()
            .position(|row| *row == Row::Header(group))
            .expect("drawer group header must be present");
        for (at, index) in (header_pos + 1..).zip(items) {
            let present = rows.snapshot().as_slice().contains(&Row::Item(index));
            if !present {
                rows.insert(at, Row::Item(index));
            }
        }
    } else {
        while let Some(pos) = rows
            .snapshot()
            .iter()
            .position(|row| items.iter().any(|index| *row == Row::Item(*index)))
        {
            let _ = rows.remove(pos);
        }
    }
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
            size: Binding::container("Medium"),
        }
    }
}

/// A single catalog entry: a control with its drawer icon and live demo.
struct Control {
    title: &'static str,
    icon: fn() -> mdi::Svg,
    section: Section,
    demo: fn(&DemoState) -> AnyView,
}

/// The catalog, in drawer order.
fn controls() -> Vec<Control> {
    vec![
        Control {
            title: "Buttons",
            icon: mdi::gesture_tap_button,
            section: Section::Actions,
            demo: |s| AnyView::new(buttons_demo(&s.taps)),
        },
        Control {
            title: "Text Field",
            icon: mdi::form_textbox,
            section: Section::Inputs,
            demo: |s| AnyView::new(text_field_demo(&s.name)),
        },
        Control {
            title: "Slider",
            icon: mdi::tune,
            section: Section::Inputs,
            demo: |s| AnyView::new(slider_demo(&s.volume)),
        },
        Control {
            title: "Stepper",
            icon: mdi::numeric,
            section: Section::Inputs,
            demo: |s| AnyView::new(stepper_demo(&s.quantity)),
        },
        Control {
            title: "Toggle",
            icon: mdi::toggle_switch,
            section: Section::Selection,
            demo: |s| AnyView::new(toggle_demo(&s.wifi, &s.bluetooth)),
        },
        Control {
            title: "Picker",
            icon: mdi::format_list_bulleted,
            section: Section::Selection,
            demo: |s| AnyView::new(picker_demo(&s.size)),
        },
        Control {
            title: "Label",
            icon: mdi::text_box_outline,
            section: Section::Display,
            demo: |_| AnyView::new(label_demo()),
        },
        Control {
            title: "Progress",
            icon: mdi::gauge,
            section: Section::Display,
            demo: |_| AnyView::new(progress_demo()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Layout: collapsible grouped navigation drawer + detail
// ---------------------------------------------------------------------------

/// The whole catalog: a single, stable [`NavigationSplitView`] (no `watch` at the
/// root, so it is never rebuilt as a unit). The split places the sidebar and detail
/// panes and collapses the sidebar to a rail on narrow windows.
fn catalog(
    selected: Binding<Option<usize>>,
    groups_open: Vec<Binding<bool>>,
    rows: ReactiveList<Row>,
    state: DemoState,
) -> impl View {
    let sidebar_selected = selected.clone();
    NavigationSplitView::new(
        &selected,
        move || sidebar(sidebar_selected.clone(), groups_open.clone(), rows.clone()),
        move |index| control_detail(index, &state),
    )
    .sidebar_width(ColumnWidth::new(240.0, 320.0, 480.0))
    .placeholder(placeholder)
}

/// One Material 3 navigation-drawer row: a 56dp line with a leading icon, label,
/// and optional trailing view, painted on a 28dp-rounded active-indicator pill.
/// A Material 3 navigation-drawer row shell: a 56dp line holding a leading
/// tappable target (a [`button`], so the row is a single accessibility node) and
/// an optional trailing view, on a full-width 28dp-rounded active-indicator pill.
/// `active` drives the pill's reactive fill (`SurfaceVariant` when selected, else
/// transparent) on `.background` itself — initial-correct and selection-tracking —
/// and `.clip` rounds it to the MD3 pill shape.
fn drawer_row(
    leading: impl View,
    trailing: impl View,
    active: Computed<bool>,
    indent: f32,
) -> impl View {
    let selected: Color = SurfaceVariant.into();
    let clear: Color = waterui::color::Srgb::WHITE.with_opacity(0.0).into();
    let pill = signal_color(active.select(selected, clear).computed());
    hstack((leading, spacer(), trailing))
        .height(DRAWER_ITEM_HEIGHT)
        .padding_with(EdgeInsets::new(
            0.0,
            0.0,
            DRAWER_ITEM_PADDING + indent,
            DRAWER_ITEM_PADDING,
        ))
        .background(pill)
        // MD3's drawer active indicator is fully rounded: a pill, whose caps are
        // half the row height. `Capsule` says that directly and stays correct at any
        // row height, where a normalized `RoundedRectangle` radius would not.
        .clip(Capsule)
}

/// Sidebar: a fixed title above a reactive collection of drawer rows. Headers
/// stay put while their groups expand and collapse; the collection diffs
/// membership by id, so a collapse removes the group's items and releases their
/// space rather than leaving a reserved gap.
fn sidebar(
    selected: Binding<Option<usize>>,
    groups_open: Vec<Binding<bool>>,
    rows: ReactiveList<Row>,
) -> impl View {
    let drawer = VStack::for_each(rows.clone(), move |row| match row {
        Row::Header(group) => AnyView::new(group_header(
            group,
            groups_open[group].clone(),
            rows.clone(),
        )),
        Row::Item(index) => AnyView::new(item_row(index, selected.clone())),
    })
    .spacing(2.0)
    .alignment(HorizontalAlignment::Leading);

    // Animate group expand/collapse with Material 3 emphasized easing: items fade
    // and collapse along the stack axis as a group opens or closes, while still
    // releasing their layout space (no reserved gap). This routes the drawer to the
    // retained collection path, where the active-indicator pill tracks selection
    // through the items' fine-grained reactive backgrounds.
    let drawer = collection_transition(
        drawer,
        Animation::bezier(Duration::from_millis(250), 0.2, 0.0, 0.0, 1.0),
    );

    scroll(
        vstack((
            text("WaterUI Controls")
                .sub_headline()
                .bold()
                .foreground(Foreground)
                .padding_with(EdgeInsets::new(20.0, 12.0, 28.0, 16.0)),
            drawer,
        ))
        .spacing(4.0)
        .alignment(HorizontalAlignment::Leading)
        .padding_with(EdgeInsets::symmetric(8.0, 12.0)),
    )
}

/// A collapsible group header, styled as a Material 3 drawer row: section icon +
/// title (in the muted on-surface-variant role, since a header is not selectable)
/// with a trailing chevron that flips with the open state. Tapping toggles the
/// group and reconciles the reactive row collection.
fn group_header(group: usize, open: Binding<bool>, rows: ReactiveList<Row>) -> impl View {
    let section = Section::ALL[group];
    let muted: Color = MutedForeground.into();
    let chevron = zstack((
        mdi::chevron_right().visible(open.clone().map(|o| !o)),
        mdi::chevron_down().visible(open.clone()),
    ))
    .foreground(muted.clone())
    .width(DRAWER_ICON_SIZE)
    .height(DRAWER_ICON_SIZE);
    let header = button(label(section.title()).icon((section.icon())()).leading())
        .borderless()
        .action(move |_env: Environment| {
            let expanded = !open.get();
            open.set(expanded);
            set_group_expanded(&rows, group, expanded);
        });
    drawer_row(header, chevron, Computed::constant(false), 0.0)
}

/// A selectable control row, nested under its group header. Tapping selects the
/// control; the Material 3 active-indicator pill (a `SurfaceVariant` fill — the
/// default scheme's surface-variant matches secondary-container) tracks the
/// selection through a fine-grained reactive opacity. A [`button`] keeps the row
/// a single accessibility node that survives the reactive collection.
fn item_row(index: usize, selected: Binding<Option<usize>>) -> impl View {
    let control = &controls()[index];
    let title = control.title;
    let icon = control.icon;
    let is_selected = selected
        .clone()
        .map(move |current| current == Some(index))
        .computed();
    let activate = selected.clone();
    let item = button(label(title).icon(icon()).leading())
        .borderless()
        .action(move |_env: Environment| activate.set(Some(index)));
    drawer_row(item, (), is_selected, DRAWER_ITEM_INDENT)
}

/// The detail pane for the selected control: its title (in the navigation bar)
/// and its live demo, bound to the shared [`DemoState`].
fn control_detail(index: usize, state: &DemoState) -> NavigationView {
    let all = controls();
    let control = &all[index];
    NavigationView::new(
        control.title,
        (control.demo)(state).padding_with(EdgeInsets::all(20.0)),
    )
}

/// Shown before any control is selected (wide layouts).
fn placeholder() -> impl View {
    vstack((
        text("WaterUI Controls").title().foreground(Foreground),
        text("Choose a control from the navigation drawer to see it live.")
            .body()
            .foreground(MutedForeground),
    ))
    .spacing(10.0)
    .padding()
}

/// Shared intro line for a demo.
fn note(text_value: &'static str) -> impl View {
    text(text_value).body().foreground(MutedForeground)
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
    .alignment(HorizontalAlignment::Leading)
}

fn toggle_demo(wifi: &Binding<bool>, bluetooth: &Binding<bool>) -> impl View {
    vstack((
        note("ToggleStyle switches between a switch and a checkbox."),
        Toggle::new(wifi).label("Wi-Fi").style(ToggleStyle::Switch),
        Toggle::new(bluetooth)
            .label("Bluetooth")
            .style(ToggleStyle::Checkbox),
        text!("Wi-Fi {wifi} · Bluetooth {bluetooth}").body(),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn slider_demo(volume: &Binding<f64>) -> impl View {
    vstack((
        note("Drag the slider; the progress bar reflects the value."),
        slider("Volume", volume).range(0.0..=100.0),
        text!("Value: {volume}").body(),
        progress(volume.clone().map(|v| v / 100.0)).label("Volume"),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn stepper_demo(quantity: &Binding<i32>) -> impl View {
    vstack((
        note("Stepper adjusts an integer within a range."),
        stepper("Quantity", quantity).range(0..=10),
        text!("Quantity: {quantity}").body(),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn text_field_demo(name: &Binding<Str>) -> impl View {
    vstack((
        note("TextField binds to reactive text and echoes it live."),
        TextField::new(name).label("Name").prompt("Type your name"),
        text!("Echo: {name}").body(),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn size_items() -> Vec<waterui::form::picker::PickerItem<&'static str>> {
    vec![
        text("Small").tag("Small"),
        text("Medium").tag("Medium"),
        text("Large").tag("Large"),
    ]
}

fn picker_demo(size: &Binding<&'static str>) -> impl View {
    vstack((
        note("PickerStyle renders the same selection as segmented, menu, or radio."),
        text("Segmented").sub_headline(),
        picker(size_items(), size).style(PickerStyle::Segmented),
        text("Menu").sub_headline(),
        picker(size_items(), size).style(PickerStyle::Menu),
        text("Radio").sub_headline(),
        picker(size_items(), size).style(PickerStyle::Radio),
        text!("Selected: {size}").body(),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn label_demo() -> impl View {
    vstack((
        note("LabelDisplayMode controls whether the title, icon, or both show."),
        label("Title and Icon")
            .icon(mdi::home())
            .leading()
            .display_mode(LabelDisplayMode::TitleAndIcon),
        label("Title Only")
            .icon(mdi::home())
            .leading()
            .display_mode(LabelDisplayMode::TitleOnly),
        label("Icon Only")
            .icon(mdi::home())
            .leading()
            .display_mode(LabelDisplayMode::IconOnly),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn progress_demo() -> impl View {
    vstack((
        note("Progress shows determinate completion."),
        progress(0.25).label("Downloading"),
        progress(0.5).label("Halfway"),
        progress(0.75).label("Almost there"),
    ))
    .spacing(16.0)
    .alignment(HorizontalAlignment::Leading)
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// The window-scoped catalog state: the selected control, one open-flag per
/// group, the reactive drawer rows, and the interactive demo state.
type CatalogState = (
    Binding<Option<usize>>,
    Vec<Binding<bool>>,
    ReactiveList<Row>,
    DemoState,
);

/// Builds fresh catalog state: `selected` starts on the first control, every
/// group starts expanded, and the demo state starts at its defaults.
fn new_state() -> CatalogState {
    let selected = Binding::container(Some(0));
    let groups_open = Section::ALL.iter().map(|_| binding(true)).collect();
    let rows = ReactiveList::from(expanded_rows());
    (selected, groups_open, rows, DemoState::new())
}

/// Self-contained entry for previews and embedding.
#[preview]
pub fn demo() -> impl View {
    let (selected, groups_open, rows, state) = new_state();
    catalog(selected, groups_open, rows, state)
}

pub fn app(env: Environment) -> App {
    // Own all mutable state at the window scope so it persists across rebuilds.
    let (selected, groups_open, rows, state) = new_state();
    App::new(
        move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        },
        env,
    )
}

#[cfg(test)]
mod tests {
    use super::{catalog, new_state};
    use core::time::Duration;
    use waterui::Str;
    use waterui_testing::{Role, UiBuilder};

    /// Selecting each control in turn shows its demo — a smoke test that every
    /// catalog entry resolves and renders (the per-component MD3 visual review was
    /// done out-of-band with GPU snapshots).
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn every_control_renders_its_demo(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });
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
            app.query().label(item).tap();
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn catalog_lists_controls(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn selecting_control_shows_demo(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });
        app.query().label("Toggle").tap();
        assert!(
            app.query()
                .label_contains("Bluetooth")
                .wait_for_existence(Duration::from_secs(3)),
            "the Toggle demo should render after selection"
        );
    }

    /// Collapsing a group removes its items (and releases their space), while the
    /// rest of the catalog stays intact.
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn collapsing_group_removes_items(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn expanding_group_restores_items(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn button_demo_is_interactive(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn toggle_demo_flips_switch_and_checkbox_bindings(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let wifi = state.wifi.clone();
        let bluetooth = state.bluetooth.clone();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });

        app.query().label("Toggle").tap();
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
            !wifi.get(),
            "flipping the Wi-Fi switch should clear its binding"
        );
        app.query()
            .role(Role::SWITCH)
            .label("Wi-Fi")
            .checked(false)
            .assert_exists();

        app.query().role(Role::CHECKBOX).label("Bluetooth").tap();
        assert!(
            bluetooth.get(),
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn slider_demo_increment_decrement_updates_volume(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let volume = state.volume.clone();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });

        app.query().label("Slider").tap();
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
            (volume.get() - 41.0).abs() < 0.0001,
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
            (volume.get() - 40.0).abs() < 0.0001,
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn stepper_demo_increment_decrement_updates_quantity(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let quantity = state.quantity.clone();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });

        app.query().label("Stepper").tap();
        assert!(
            app.query()
                .label("Quantity")
                .wait_for_existence(Duration::from_secs(3)),
            "the Stepper demo should render its Quantity stepper"
        );

        app.query().label("Quantity").value("2").increment();
        assert_eq!(
            quantity.get(),
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
            quantity.get(),
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn text_field_demo_set_text_updates_name(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let name = state.name.clone();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });

        app.query().label("Text Field").tap();
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
            name.get(),
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
    #[waterui::test(theme = hydrolysis_m3::install, viewport = (1100, 760))]
    fn expanding_a_collapsed_group_and_selecting_its_control_switches_the_heading(ui: UiBuilder) {
        let (selected, groups_open, rows, state) = new_state();
        let mut app = ui.mount(move || {
            catalog(
                selected.clone(),
                groups_open.clone(),
                rows.clone(),
                state.clone(),
            )
        });

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
        app.query().label("Toggle").tap();
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
}

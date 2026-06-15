//! WaterUI Control Catalog — a Material Design 3 component gallery.
//!
//! The layout is a navigation drawer beside a detail pane (a [`NavigationSplitView`],
//! which collapses the drawer to a rail on narrow windows). The drawer lists the
//! framework's controls grouped by category; each group header expands or collapses
//! its items. Selecting a control shows it live and interactive in the detail area —
//! including its style/variant options — so the catalog demonstrates each control's
//! behavior directly instead of pointing elsewhere.

use waterui::Color;
use waterui::app::App;
use waterui::color::Srgb;
use waterui::form::picker::{PickerStyle, picker};
use waterui::layout::HorizontalAlignment;
use waterui::navigation::{NavigationSplitView, NavigationView};
use waterui::prelude::slider::slider;
use waterui::prelude::stepper::stepper;
use waterui::prelude::theme_color::{Foreground, MutedForeground, SurfaceVariant};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui_icons_material_icon as mdi;

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

/// A single catalog entry: a control with its drawer icon and live demo.
struct Control {
    title: &'static str,
    icon: fn() -> mdi::Svg,
    section: Section,
    demo: fn() -> AnyView,
}

/// The catalog, in drawer order.
fn controls() -> Vec<Control> {
    vec![
        Control {
            title: "Buttons",
            icon: mdi::gesture_tap_button,
            section: Section::Actions,
            demo: || AnyView::new(buttons_demo()),
        },
        Control {
            title: "Text Field",
            icon: mdi::form_textbox,
            section: Section::Inputs,
            demo: || AnyView::new(text_field_demo()),
        },
        Control {
            title: "Slider",
            icon: mdi::tune,
            section: Section::Inputs,
            demo: || AnyView::new(slider_demo()),
        },
        Control {
            title: "Stepper",
            icon: mdi::numeric,
            section: Section::Inputs,
            demo: || AnyView::new(stepper_demo()),
        },
        Control {
            title: "Toggle",
            icon: mdi::toggle_switch,
            section: Section::Selection,
            demo: || AnyView::new(toggle_demo()),
        },
        Control {
            title: "Picker",
            icon: mdi::format_list_bulleted,
            section: Section::Selection,
            demo: || AnyView::new(picker_demo()),
        },
        Control {
            title: "Label",
            icon: mdi::text_box_outline,
            section: Section::Display,
            demo: || AnyView::new(label_demo()),
        },
        Control {
            title: "Progress",
            icon: mdi::gauge,
            section: Section::Display,
            demo: || AnyView::new(progress_demo()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Layout: collapsible grouped navigation drawer + detail
// ---------------------------------------------------------------------------

/// The whole catalog: a single, stable [`NavigationSplitView`]. It is never
/// rebuilt (no `watch` at the root), so selecting a control or collapsing a
/// group never triggers a full-window rebuild. The split places the sidebar and
/// detail panes and collapses the sidebar to a rail on narrow windows.
fn catalog(selected: Binding<Option<usize>>, groups_open: Vec<Binding<bool>>) -> impl View {
    let sidebar_selected = selected.clone();
    NavigationSplitView::new(
        &selected,
        move || sidebar(sidebar_selected.clone(), groups_open.clone()),
        control_detail,
    )
    .sidebar_width(280.0)
    .placeholder(placeholder)
}

/// Sidebar: a title header followed by one collapsible group per category.
fn sidebar(selected: Binding<Option<usize>>, groups_open: Vec<Binding<bool>>) -> impl View {
    let all = controls();
    let mut rows: Vec<AnyView> = vec![AnyView::new(
        text("WaterUI Controls")
            .sub_headline()
            .bold()
            .foreground(Foreground)
            .padding_with(EdgeInsets::new(12.0, 12.0, 14.0, 12.0)),
    )];

    for (group_index, &section) in Section::ALL.iter().enumerate() {
        let open = groups_open[group_index].clone();
        rows.push(AnyView::new(group_header(section, &open)));
        for (index, control) in all
            .iter()
            .enumerate()
            .filter(|(_, control)| control.section == section)
        {
            rows.push(AnyView::new(item_row(index, control, &selected, &open)));
        }
    }

    scroll(
        vstack(rows)
            .spacing(2.0)
            .alignment(HorizontalAlignment::Leading)
            .padding_with(EdgeInsets::symmetric(6.0, 6.0)),
    )
}

/// A collapsible group header: an accessible button (group icon + title) that
/// toggles the group, plus a chevron that reflects the open state.
fn group_header(section: Section, open: &Binding<bool>) -> impl View {
    let header = button(label(section.title()).icon((section.icon())()).leading())
        .borderless()
        .action(|State(open): State<Binding<bool>>| open.set(!open.get()))
        .state(open);
    let chevron = zstack((
        mdi::chevron_right().visible(open.clone().map(|o| !o)),
        mdi::chevron_down().visible(open.clone()),
    ));
    hstack((header, spacer(), chevron)).padding_with(EdgeInsets::symmetric(2.0, 10.0))
}

/// A selectable control row, indented under its group and hidden when the group
/// is collapsed. A [`button`] keeps it a proper accessible, focusable target.
fn item_row(
    index: usize,
    control: &Control,
    selected: &Binding<Option<usize>>,
    open: &Binding<bool>,
) -> impl View {
    let is_selected = selected.clone().map(move |current| current == Some(index));
    let selected_bg: Color = SurfaceVariant.into();
    let clear_bg: Color = Srgb::WHITE.with_opacity(0.0).into();
    let background = is_selected.select(selected_bg, clear_bg).computed();

    button(label(control.title).icon((control.icon)()).leading())
        .borderless()
        .action(move |State(sel): State<Binding<Option<usize>>>| sel.set(Some(index)))
        .state(selected)
        .background(background)
        .padding_with(EdgeInsets::new(3.0, 3.0, 30.0, 8.0))
        .visible(open.clone())
}

/// The detail pane for the selected control: its title (in the navigation bar)
/// and its live demo.
fn control_detail(index: usize) -> NavigationView {
    let all = controls();
    let control = &all[index];
    NavigationView::new(
        control.title,
        (control.demo)().padding_with(EdgeInsets::all(20.0)),
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
// Control demos
// ---------------------------------------------------------------------------

/// Increments a shared tap counter; reused by every button style.
fn bump(State(taps): State<Binding<i32>>) {
    taps.set(taps.get() + 1);
}

fn buttons_demo() -> impl View {
    let taps: Binding<i32> = binding(0);
    vstack((
        note("Every button shares one action. ButtonStyle controls the appearance."),
        text!("Taps: {taps}").body(),
        button("Automatic").action(bump).state(&taps),
        button("Bordered").bordered().action(bump).state(&taps),
        button("Bordered Prominent")
            .bordered_prominent()
            .action(bump)
            .state(&taps),
        button("Plain").plain().action(bump).state(&taps),
        button("Borderless").borderless().action(bump).state(&taps),
        button("Link").link().action(bump).state(&taps),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn toggle_demo() -> impl View {
    let wifi = binding(true);
    let bluetooth = binding(false);
    vstack((
        note("ToggleStyle switches between a switch and a checkbox."),
        Toggle::new(&wifi).label("Wi-Fi").style(ToggleStyle::Switch),
        Toggle::new(&bluetooth)
            .label("Bluetooth")
            .style(ToggleStyle::Checkbox),
        text!("Wi-Fi {wifi} · Bluetooth {bluetooth}").body(),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn slider_demo() -> impl View {
    let value = binding(40.0);
    vstack((
        note("Drag the slider; the progress bar reflects the value."),
        slider("Volume", &value).range(0.0..=100.0),
        text!("Value: {value}").body(),
        progress(value.clone().map(|v| v / 100.0)),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn stepper_demo() -> impl View {
    let quantity = binding(2);
    vstack((
        note("Stepper adjusts an integer within a range."),
        stepper("Quantity", &quantity).range(0..=10),
        text!("Quantity: {quantity}").body(),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

fn text_field_demo() -> impl View {
    let name: Binding<Str> = binding(Str::from(""));
    vstack((
        note("TextField binds to reactive text and echoes it live."),
        TextField::new(&name).label("Name").prompt("Type your name"),
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

fn picker_demo() -> impl View {
    let size = Binding::container("Medium");
    vstack((
        note("PickerStyle renders the same selection as segmented, menu, or radio."),
        text("Segmented").sub_headline(),
        picker(size_items(), &size).style(PickerStyle::Segmented),
        text("Menu").sub_headline(),
        picker(size_items(), &size).style(PickerStyle::Menu),
        text("Radio").sub_headline(),
        picker(size_items(), &size).style(PickerStyle::Radio),
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
        text("25%").caption(),
        progress(0.25),
        text("50%").caption(),
        progress(0.5),
        text("75%").caption(),
        progress(0.75),
    ))
    .spacing(12.0)
    .alignment(HorizontalAlignment::Leading)
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Builds fresh catalog state: the selected control plus one open-flag per group.
fn new_state() -> (Binding<Option<usize>>, Vec<Binding<bool>>) {
    let selected = Binding::container(Some(0));
    let groups_open = Section::ALL.iter().map(|_| binding(true)).collect();
    (selected, groups_open)
}

/// Self-contained entry for previews and embedding.
#[preview]
pub fn demo() -> impl View {
    let (selected, groups_open) = new_state();
    catalog(selected, groups_open)
}

pub fn app(env: Environment) -> App {
    // Own the selection + group-open state at the window scope so it persists
    // across rebuilds.
    let (selected, groups_open) = new_state();
    App::new(move || catalog(selected.clone(), groups_open.clone()), env)
}

#[cfg(test)]
mod tests {
    use super::{catalog, new_state};
    use core::time::Duration;
    use waterui::Binding;
    use waterui::env::Environment;
    use waterui_testing::{SemanticApp, ui};

    /// Mounts the catalog with caller-owned state (as a real app window owns its
    /// root state) so interaction survives the test driver's full-tree rebuilds.
    fn mount(
        selected: Binding<Option<usize>>,
        groups_open: Vec<Binding<bool>>,
        width: u32,
        height: u32,
    ) -> SemanticApp {
        let mut env = Environment::new();
        hydrolysis_m3::install(&mut env);
        ui().environment(env)
            .viewport(width, height)
            .mount(move || catalog(selected.clone(), groups_open.clone()))
    }

    /// The drawer lists every group header and control, and the detail pane shows
    /// the selected control's live demo.
    #[test]
    fn catalog_lists_controls() {
        let (selected, groups_open) = new_state();
        let mut app = mount(selected, groups_open, 1100, 760);
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
    #[test]
    fn selecting_control_shows_demo() {
        let (selected, groups_open) = new_state();
        let mut app = mount(selected, groups_open, 1100, 760);
        assert!(
            app.query().label("Toggle").tap(),
            "drawer item should be tappable"
        );
        assert!(
            app.query()
                .label_contains("Bluetooth")
                .wait_for_existence(Duration::from_secs(3)),
            "the Toggle demo should render after selection"
        );
    }

    /// Collapsing a group hides its items while the rest of the catalog stays
    /// intact (the regression that previously blanked the window).
    #[test]
    fn collapsing_group_hides_items_and_keeps_layout() {
        let (selected, groups_open) = new_state();
        let inputs_open = groups_open[1].clone(); // Section::ALL[1] == Inputs
        let mut app = mount(selected, groups_open, 1100, 760);
        app.query().label("Slider").assert_exists();
        inputs_open.set(false);
        assert!(
            app.query()
                .label("Slider")
                .wait_for_nonexistence(Duration::from_secs(3)),
            "collapsing a group should hide its items"
        );
        app.query().label("Buttons").assert_exists();
        app.query()
            .label_contains("Bordered Prominent")
            .assert_exists();
    }

    /// The group-header button toggles its group accessibly (Click action), and
    /// selecting an item updates the detail — both without blanking.
    #[test]
    fn group_header_button_collapses_group() {
        let (selected, groups_open) = new_state();
        let mut app = mount(selected, groups_open, 1100, 760);
        assert!(app.query().label("Slider").tap(), "item should be tappable");
        assert!(
            app.query().label("Inputs").tap(),
            "group header should be tappable"
        );
        assert!(
            app.query()
                .label("Slider")
                .wait_for_nonexistence(Duration::from_secs(3)),
            "tapping the group header should collapse its items"
        );
    }
}

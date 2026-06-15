//! WaterUI Control Catalog — a Material Design 3 component gallery.
//!
//! The layout is a top app bar plus a collapsible navigation drawer and a detail
//! area. The drawer lists the framework's controls grouped by category; the menu
//! button in the app bar collapses the drawer to an icon-only rail and expands it
//! again. Selecting a control shows it live and interactive in the detail area —
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
// Layout: top app bar + collapsible drawer/rail + detail
// ---------------------------------------------------------------------------

/// The whole catalog. The split view is the root so the window gives it full
/// bounds (it places the sidebar + detail panes itself). Toggling `expanded`
/// rebuilds it so the sidebar can switch between a labelled drawer (wide) and an
/// icon-only rail (narrow).
fn catalog(expanded: Binding<bool>, selected: Binding<Option<usize>>) -> impl View {
    let watched = expanded.clone();
    watch(watched, move |open| {
        let selected = selected.clone();
        let sidebar_selected = selected.clone();
        let sidebar_toggle = expanded.clone();
        NavigationSplitView::new(
            &selected,
            move || drawer(open, sidebar_toggle.clone(), sidebar_selected.clone()),
            control_detail,
        )
        .sidebar_width(if open { 256.0 } else { 88.0 })
        .placeholder(placeholder)
    })
}

/// The drawer header: a menu button that toggles the drawer, plus the title
/// (shown only when expanded).
fn drawer_header(expanded: bool, toggle: &Binding<bool>) -> impl View {
    let menu = button(label("Toggle navigation").icon(mdi::menu()).icon_only())
        .borderless()
        .action(|State(open): State<Binding<bool>>| open.set(!open.get()))
        .state(toggle);
    let title = text("WaterUI Controls")
        .sub_headline()
        .bold()
        .foreground(Foreground)
        .visible(expanded);
    hstack((menu, title))
        .spacing(10.0)
        .padding_with(EdgeInsets::new(8.0, 8.0, 6.0, 8.0))
}

/// Sidebar contents: a header, then section headers + labelled rows (drawer), or
/// icon-only rows (rail) when collapsed.
fn drawer(expanded: bool, toggle: Binding<bool>, selected: Binding<Option<usize>>) -> impl View {
    let all = controls();
    let mut rows: Vec<AnyView> = vec![AnyView::new(drawer_header(expanded, &toggle))];

    for section in Section::ALL {
        if expanded {
            rows.push(AnyView::new(
                text(section.title())
                    .caption()
                    .bold()
                    .foreground(MutedForeground)
                    .padding_with(EdgeInsets::new(14.0, 6.0, 12.0, 8.0)),
            ));
        } else {
            rows.push(AnyView::new(Divider));
        }
        for (index, control) in all
            .iter()
            .enumerate()
            .filter(|(_, control)| control.section == section)
        {
            rows.push(AnyView::new(drawer_item(
                index, control, expanded, &selected,
            )));
        }
    }

    scroll(
        vstack(rows)
            .spacing(4.0)
            .alignment(HorizontalAlignment::Leading)
            .padding_with(EdgeInsets::symmetric(8.0, 8.0)),
    )
}

/// A selectable sidebar row. Uses a [`Label`] so the M3 backend renders an
/// icon + title (or icon-only when collapsed) with proper accessibility.
fn drawer_item(
    index: usize,
    control: &Control,
    expanded: bool,
    selected: &Binding<Option<usize>>,
) -> impl View {
    let is_selected = selected.clone().map(move |current| current == Some(index));
    let selected_bg: Color = SurfaceVariant.into();
    let clear_bg: Color = Srgb::WHITE.with_opacity(0.0).into();
    let background = is_selected.select(selected_bg, clear_bg).computed();

    let mode = if expanded {
        LabelDisplayMode::TitleAndIcon
    } else {
        LabelDisplayMode::IconOnly
    };

    button(
        label(control.title)
            .icon((control.icon)())
            .leading()
            .display_mode(mode),
    )
    .borderless()
    .action(move |State(sel): State<Binding<Option<usize>>>| sel.set(Some(index)))
    .state(selected)
    .background(background)
    .padding_with(EdgeInsets::symmetric(2.0, 4.0))
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

/// Self-contained entry for previews and embedding.
#[preview]
pub fn demo() -> impl View {
    catalog(binding(true), Binding::container(Some(0)))
}

#[preview]
fn rail() -> impl View {
    catalog(binding(false), Binding::container(Some(4)))
}

pub fn app(env: Environment) -> App {
    // Own the drawer + selection state at the window scope so they persist
    // across content rebuilds.
    let expanded = binding(true);
    let selected = Binding::container(Some(0));
    App::new(move || catalog(expanded.clone(), selected.clone()), env)
}

#[cfg(test)]
mod tests {
    use super::catalog;
    use core::time::Duration;
    use waterui::Binding;
    use waterui::env::Environment;
    use waterui::reactive::binding;
    use waterui_testing::{SemanticApp, ui};

    /// Mounts the catalog with caller-owned state (as a real app window owns its
    /// root state) so interaction survives the test driver's full-tree rebuilds.
    fn mount(
        expanded: Binding<bool>,
        selected: Binding<Option<usize>>,
        width: u32,
        height: u32,
    ) -> SemanticApp {
        let mut env = Environment::new();
        hydrolysis_m3::install(&mut env);
        ui().environment(env)
            .viewport(width, height)
            .mount(move || catalog(expanded.clone(), selected.clone()))
    }

    /// The expanded drawer lists section headers and every control, and the
    /// detail pane shows the selected control's live demo.
    #[test]
    fn catalog_lists_controls() {
        let mut app = mount(binding(true), Binding::container(Some(0)), 1100, 760);
        for section in ["Actions", "Inputs", "Selection", "Display"] {
            app.query().label(section).assert_exists();
        }
        for control in ["Buttons", "Toggle", "Slider", "Picker", "Label"] {
            app.query().label(control).assert_exists();
        }
        // Buttons is selected, so the detail shows a button-style variant.
        app.query()
            .label_contains("Bordered Prominent")
            .assert_exists();
    }

    /// Selecting a control in the drawer shows its live demo in the detail pane.
    #[test]
    fn selecting_control_shows_demo() {
        let mut app = mount(binding(true), Binding::container(Some(0)), 1100, 760);
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

    /// The menu button collapses the drawer to an icon-only rail, hiding the
    /// section headers.
    #[test]
    fn menu_button_collapses_to_rail() {
        let mut app = mount(binding(true), Binding::container(Some(0)), 1100, 760);
        app.query().label("Actions").assert_exists();
        assert!(
            app.query().label("Toggle navigation").tap(),
            "menu button should be tappable"
        );
        assert!(
            app.query()
                .label("Actions")
                .wait_for_nonexistence(Duration::from_secs(3)),
            "section headers should disappear in the collapsed rail"
        );
    }
}

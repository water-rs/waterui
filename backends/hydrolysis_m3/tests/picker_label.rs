//! A picker announces the label it was constructed with.
//!
//! `PickerConfig::label` is mandatory precisely so assistive technology has a
//! name to announce, but the self-drawn renderer never read it: the menu style
//! announced the *selected option* as the control's name — the same string it
//! already reports as the value — and the radio and segmented styles announced
//! nothing at all. A screen reader could tell you "Medium" without ever saying
//! what "Medium" was choosing.

#![expect(
    clippy::needless_pass_by_value,
    reason = "the binding is moved into the mounted body, which must own it"
)]

use waterui::form::{Picker, PickerItem, PickerStyle};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_testing::UiBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Size {
    Small,
    Medium,
    Large,
}

impl Size {
    const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    const fn name(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }
}

/// Mounts a picker whose label names the control rather than the choice.
///
/// The binding is owned here because the mounted body must outlive the call,
/// and `Picker::new` only borrows it to build the config.
fn sized_picker(selection: Binding<Size>, style: PickerStyle) -> impl View {
    let items: Vec<PickerItem<Size>> = Size::ALL
        .into_iter()
        .map(|size| PickerItem::new(size, text::text(size.name())))
        .collect();
    Picker::new("Shirt size", items, &selection).style(style)
}

/// The menu style names the control and reports the selection as its value.
/// Those are two different strings and it used to give the selection for both.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 300))]
fn a_menu_picker_announces_its_own_label(ui: UiBuilder) {
    let selection = binding(Size::Medium);
    let mut app = ui.mount(move || sized_picker(selection.clone(), PickerStyle::Menu));
    app.settle();

    let _ = app.query().label("Shirt size").single();
}

/// The radio style groups its options, and the group is what carries the name.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 300))]
fn a_radio_picker_announces_its_own_label(ui: UiBuilder) {
    let selection = binding(Size::Small);
    let mut app = ui.mount(move || sized_picker(selection.clone(), PickerStyle::Radio));
    app.settle();

    let _ = app.query().label("Shirt size").single();
}

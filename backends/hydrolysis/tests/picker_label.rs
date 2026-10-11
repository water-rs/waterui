//! Visual evidence for water-rs/hydrolysis#97: a labelled menu picker mounted
//! under Material 3 in its light and dark schemes. The rendered field label
//! must sit inside the field above the selected value, styled as the M3
//! exposed-dropdown label; the pair is captured through the canonical
//! `TestArtifacts` layout so the result can be judged by eye.

use hydrolysis_m3::{Material3, MaterialColorScheme};
use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::reactive::binding;
use waterui_core::AnyView;
use waterui_form::picker::{PickerStyle, picker};
use waterui_testing::ui;
use waterui_text::text;

fn labelled_menu_picker() -> impl Fn() -> AnyView {
    let selection = binding(0i32);
    move || {
        AnyView::new(
            vstack((picker(
                "Size",
                vec![text("Small").tag(0i32), text("Large").tag(1i32)],
                &selection,
            )
            .style(PickerStyle::Menu),))
            .padding(),
        )
    }
}

#[test]
fn menu_picker_label_material3_snapshots() {
    let mut light = ui()
        .viewport(360, 140)
        .theme(Material3::with_colors(MaterialColorScheme::baseline_light()))
        .mount_offscreen(labelled_menu_picker());
    light.capture_snapshot("hydrolysis", "menu-picker-label", "light");

    let mut dark = ui()
        .viewport(360, 140)
        .theme(Material3::dark())
        .mount_offscreen(labelled_menu_picker());
    dark.capture_snapshot("hydrolysis", "menu-picker-label", "dark");
}

fn labelled_group_picker(style: PickerStyle, hide_label: bool) -> impl Fn() -> AnyView {
    let selection = binding(0i32);
    move || {
        let group = picker(
            "Size",
            vec![text("Small").tag(0i32), text("Large").tag(1i32)],
            &selection,
        )
        .style(style);
        let group = if hide_label {
            group.hide_label()
        } else {
            group
        };
        AnyView::new(vstack((group,)).padding())
    }
}

/// Visual evidence for water-rs/hydrolysis#101: the radio group's heading
/// sits above the option rows, styled like the menu field's label.
#[test]
fn radio_picker_label_material3_snapshots() {
    let mut light = ui()
        .viewport(360, 200)
        .theme(Material3::with_colors(MaterialColorScheme::baseline_light()))
        .mount_offscreen(labelled_group_picker(PickerStyle::Radio, false));
    light.capture_snapshot("hydrolysis", "radio-picker-label", "light");

    let mut dark = ui()
        .viewport(360, 200)
        .theme(Material3::dark())
        .mount_offscreen(labelled_group_picker(PickerStyle::Radio, false));
    dark.capture_snapshot("hydrolysis", "radio-picker-label", "dark");
}

/// Visual evidence for water-rs/hydrolysis#101: the segmented group's heading
/// sits above the segment row, styled like the menu field's label.
#[test]
fn segmented_picker_label_material3_snapshots() {
    let mut light = ui()
        .viewport(360, 120)
        .theme(Material3::with_colors(MaterialColorScheme::baseline_light()))
        .mount_offscreen(labelled_group_picker(PickerStyle::Segmented, false));
    light.capture_snapshot("hydrolysis", "segmented-picker-label", "light");

    let mut dark = ui()
        .viewport(360, 120)
        .theme(Material3::dark())
        .mount_offscreen(labelled_group_picker(PickerStyle::Segmented, false));
    dark.capture_snapshot("hydrolysis", "segmented-picker-label", "dark");

    // The same picker with a hidden label: the segment row keeps the exact
    // height it had before the label work, so the pair can be judged by eye.
    let mut nolabel = ui()
        .viewport(360, 120)
        .theme(Material3::with_colors(MaterialColorScheme::baseline_light()))
        .mount_offscreen(labelled_group_picker(PickerStyle::Segmented, true));
    nolabel.capture_snapshot("hydrolysis", "segmented-picker-nolabel", "light");
}

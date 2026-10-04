//! Material 3 interaction-state showcase: selected, hovered and pressed list
//! rows, a FAB, and a keyboard-focusable button. Runs a real winit window for
//! screenshot capture on the X display. Not committed.

use hydrolysis::run;
use hydrolysis_m3::{Material3, fab, material_card, material_list, material_list_item};
use waterui::Environment;
use waterui::app::App;
use waterui::prelude::*;
use waterui_controls::button;

fn gallery_view() -> impl View {
    vstack((
        text("M3 interaction states").size(22.0).padding(),
        material_list((
            material_list_item("Selected row")
                .supporting_text("selected container color")
                .selected(true),
            material_list_item("Hover target")
                .supporting_text("state layer, 12dp corner")
                .action(|| {}),
            material_list_item("Press target")
                .supporting_text("state layer, 16dp corner")
                .action(|| {}),
        )),
        material_card(text("Elevated card").padding())
            .elevated()
            .padding(),
        hstack((button("Focusable button").action(|| {}),))
            .spacing(12.0)
            .padding(),
        hstack((fab("Add", text("+")),)).padding(),
    ))
    .spacing(16.0)
    .padding_with(24.0)
    .background(Color::srgb_hex("#FFFBFE"))
}

fn main() {
    run(
        App::new(gallery_view, Environment::new()),
        Material3::defaults(),
    );
}

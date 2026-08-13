//! Layout regressions the Inspector depends on.
//!
//! `.visible(false)` is opacity-based and deliberately keeps its space, so a
//! hidden row leaves a gap; the panes use `when(...).otherwise(...)` where the
//! space should collapse. That is the documented behaviour, not a defect, and
//! it is exercised here so a change to it is noticed.

use hydrolysis_m3::install;
use waterui::prelude::*;
use waterui_testing::{OffscreenApp, Role, ui as test_ui};

/// A list under a header must render its rows. A column that left its growing
/// child out of its own height handed the list nothing to draw in, which is how
/// the Tasks pane lost its table.
#[core::prelude::v1::test]
fn a_list_under_a_header_renders_its_rows() {
    let rows = ["FramePump", "VideoTick", "LayoutPass"];

    let mut app: OffscreenApp = test_ui()
        .viewport(600, 400)
        .theme(install)
        .mount_offscreen(move || {
            vstack((
                text("Task").caption(),
                vstack(
                    rows.iter()
                        .map(|row| text(*row).anyview())
                        .collect::<Vec<_>>(),
                )
                .alignment(HorizontalAlignment::Leading)
                .spacing(8.0),
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(12.0)
        });

    for row in ["FramePump", "VideoTick", "LayoutPass"] {
        app.query().role(Role::LABEL).label(row).assert_exists();
    }
}

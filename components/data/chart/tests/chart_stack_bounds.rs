//! A chart sharing a stack with ordinary content.
//!
//! The accessibility bounds and the painted pixels come from the same layout
//! pass, so asserting on the bounds is a cheap way to keep the two honest.

use hydrolysis_m3::install;
use waterui::prelude::*;
use waterui_chart::{DataPoint, LineChart};
use waterui_testing::{OffscreenApp, Role, ui as test_ui};

fn series() -> Vec<DataPoint> {
    (0..40_u16)
        .map(|index| DataPoint::new(f32::from(index), f32::from(index % 7)))
        .collect()
}

/// A chart takes the space nobody else claimed. It must not take space its
/// siblings are already using.
#[core::prelude::v1::test]
fn a_chart_does_not_flatten_the_rows_beside_it() {
    fn cell(label: &'static str, value: &'static str) -> impl View {
        vstack((text(value).headline(), text(label).caption()))
            .alignment(HorizontalAlignment::Leading)
            .spacing(2.0)
    }

    let mut app: OffscreenApp = test_ui()
        .viewport(600, 500)
        .theme(install)
        .mount_offscreen(|| {
            vstack((
                LineChart::new(series()).line_width(1.5).height(160.0),
                Divider,
                vstack((
                    hstack((cell("Kind", "reencode"), cell("Total", "3.39ms"))).spacing(32.0),
                    hstack((cell("Layout", "1.13ms"), cell("Encode", "1.13ms"))).spacing(32.0),
                ))
                .alignment(HorizontalAlignment::Leading)
                .spacing(16.0),
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(16.0)
        });

    let kind = app.query().role(Role::LABEL).label("Kind").single().bounds();
    let layout = app.query().role(Role::LABEL).label("Layout").single().bounds();

    assert!(
        kind.height() > 1.0,
        "the first row was flattened to {}pt",
        kind.height()
    );
    assert!(
        layout.y() >= kind.y() + kind.height(),
        "rows overlap: the first ends at {}, the second starts at {}",
        kind.y() + kind.height(),
        layout.y()
    );
}

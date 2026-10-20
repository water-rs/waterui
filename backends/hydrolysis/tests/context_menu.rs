//! The drawn `.context_menu` presentation mounted through the public runtime
//! surface: a right-click on a menu carrying an accessory lifts the source
//! view through the dim backdrop's hole — the source keeps drawing itself, so
//! the lift shows its whole content, not an empty shell.

use accesskit::Role;
use hydrolysis::{InputEvent, PointerKind};
use waterui::layout::frame::Frame;
use waterui::prelude::*;

mod support;

use support::{f64_as_f32, mount_rendered_with, node_bounds_with_role};

/// Narrows a finite `f64` pixel coordinate to `usize`, truncating toward zero.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test layout coordinates are finite and well inside usize range"
)]
const fn f64_as_usize(v: f64) -> usize {
    v as usize
}

/// water-rs/waterui#1929: an accessory with no explicit `preview` lifts the
/// source through the dim's hole. Mounted under `Material3` — the style the
/// issue reports — the lifted `button("host")` must still carry its label:
/// every sample through the button's interior draws what the same point
/// drew before the menu opened.
#[test]
fn the_lifted_source_keeps_its_label_with_an_accessory_but_no_preview() {
    let mut app = mount_rendered_with(
        Environment::new(),
        || {
            vstack((spacer(), {
                Frame::new(
                    button("host").action(|| {}).context_menu(
                        ContextMenu::new(vec!["Copy".action(|| {})])
                            .accessory(text("preview").padding_with(6.0)),
                    ),
                )
                .width(160.0)
                .height(80.0)
            }))
        },
        320,
        480,
        hydrolysis_m3::Material3::defaults(),
    );
    app.settle();

    let baseline = app.snapshot().rgba8.clone();
    let host = node_bounds_with_role(app.tree(), Role::Button, "host");
    app.secondary_click_at(
        f64_as_f32(host.x0 + host.width() / 2.0),
        f64_as_f32(host.y0 + host.height() / 2.0),
    );
    // The pointer still rests over the source after the click; park it in a
    // window corner so the source's hover state layer does not read as a
    // content change through the hole.
    app.runtime_mut().push_input_event(InputEvent::PointerMove {
        id: 1,
        kind: PointerKind::Mouse,
        x: 8.0,
        y: 8.0,
    });
    app.settle();
    let (width, opened) = {
        let snapshot = app.snapshot();
        (
            usize::try_from(snapshot.width).expect("the frame width fits usize"),
            snapshot.rgba8.clone(),
        )
    };
    let mid_y = f64_as_usize(host.y0 + host.height() / 2.0);
    let mut mismatches = Vec::new();
    for px in (f64_as_usize(host.x0) + 4..f64_as_usize(host.x1) - 4).step_by(2) {
        let offset = (mid_y * width + px) * 4;
        let baseline_px = &baseline[offset..offset + 4];
        let opened_px = &opened[offset..offset + 4];
        let drift = baseline_px
            .iter()
            .zip(opened_px)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        if drift > 16 {
            mismatches.push((px, baseline_px.to_vec(), opened_px.to_vec()));
        }
    }
    assert!(
        mismatches.is_empty(),
        "the lifted source's interior did not draw through the hole: {mismatches:?}"
    );
}

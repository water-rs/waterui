//! End-to-end coverage for tappable spans in styled text
//! (water-rs/waterui#1352): a `TapSpan` surfaces as a `Link` accessibility
//! child, activates through both the semantic `tap` and the pointer
//! `tap_at`, and leaves the rest of the line untappable.

use std::cell::Cell;
use std::rc::Rc;

use waterui::text::{styled::StyledStr, text};
use waterui_testing::{Role, Styled, TestStyle, UiBuilder};

/// "#waterui" is the only tappable span on the line.
#[waterui::test(theme = TestStyle::new(), viewport = (390, 200))]
fn a_tap_span_is_tappable_and_publishes_a_link(ui: UiBuilder<Styled<TestStyle>>) {
    let taps = Rc::new(Cell::new(0_u32));
    let payload_seen = Rc::new(Cell::new(false));
    let (taps_in, seen_in) = (Rc::clone(&taps), Rc::clone(&payload_seen));
    let mut app = ui.mount_offscreen(move || {
        let tagged = StyledStr::plain("#waterui").on_tap("waterui", {
            let (taps, seen) = (Rc::clone(&taps_in), Rc::clone(&seen_in));
            move |payload: &'static str| {
                seen.set(payload == "waterui");
                taps.set(taps.get() + 1);
            }
        });
        text(StyledStr::plain("shipping ") + tagged + " tonight")
    });

    // (a) the span is a `Link` accessibility node named by its substring, and
    // its `Click` action delivers the payload.
    app.query().role(Role::LINK).label("#waterui").tap();
    assert_eq!(taps.get(), 1, "accessibility tap must activate the span");
    assert!(payload_seen.get(), "the span's payload must arrive");

    // (b) a pointer tap inside the span's laid-out rectangle activates the
    // same action through the gesture path.
    let link = app.query().role(Role::LINK).label("#waterui").single();
    link.tap_at(&mut app, 0.5, 0.5);
    assert_eq!(taps.get(), 2, "pointer tap must activate the span");

    // (c) a pointer tap on the untapped remainder of the text activates
    // nothing: the point just left of the span's first rectangle sits on
    // "shipping", inside the label's bounds but outside every span target.
    let point = (link.bounds().x() - 2.0, link.center().1);
    app.tap_at(point.0, point.1);
    assert_eq!(
        taps.get(),
        2,
        "a tap outside every span rectangle must not activate the span"
    );
}

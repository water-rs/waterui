//! Accessibility bounds follow the geometry owner's response rectangle
//! (water-rs/waterui#1803, `docs/layout-spec.md` §7.2): a view assigned more
//! than it answered reports its own answer-sized box, centred in the
//! allocation, unless a naming modifier chooses the allocation or an
//! explicit region.

mod support;

use accesskit::Rect;
use support::{mount_rendered, node_bounds};
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityGeometry;
use waterui::component::{hstack, text};
use waterui::layout::{Point, Rect as LayoutRect, Size};
use waterui_core::Environment;

const WIDTH: u32 = 400;
const HEIGHT: u32 = 600;
const TOLERANCE: f64 = 0.5;

fn window() -> Rect {
    Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT))
}

const fn centre(rect: Rect) -> (f64, f64) {
    (
        f64::midpoint(rect.x0, rect.x1),
        f64::midpoint(rect.y0, rect.y1),
    )
}

#[track_caller]
fn assert_centred_in_window(bounds: Rect) {
    let window = window();
    let (bounds_centre, window_centre) = (centre(bounds), centre(window));
    assert!(
        (bounds_centre.0 - window_centre.0).abs() < TOLERANCE
            && (bounds_centre.1 - window_centre.1).abs() < TOLERANCE,
        "the response is centred in the window, got {bounds:?}"
    );
}

#[track_caller]
fn assert_answer_sized(bounds: Rect) {
    assert!(
        bounds.width() > 0.0
            && bounds.height() > 0.0
            && bounds.width() < window().width() / 2.0
            && bounds.height() < window().height() / 2.0,
        "the response is the content's own size, not the allocation, got {bounds:?}"
    );
}

#[test]
fn allocation_geometry_reports_the_allocation() {
    let mut app = mount_rendered(
        Environment::new(),
        || text("Hello").a11y_geometry(AccessibilityGeometry::Allocation),
        WIDTH,
        HEIGHT,
    );
    app.settle();
    assert_eq!(node_bounds(app.tree(), "Hello"), window());
}

#[test]
fn region_geometry_reports_the_region_in_owner_coordinates() {
    let region = LayoutRect::new(Point::new(10.0, 20.0), Size::new(30.0, 40.0));
    let mut app = mount_rendered(
        Environment::new(),
        move || text("Hello").a11y_geometry(AccessibilityGeometry::Region(region)),
        WIDTH,
        HEIGHT,
    );
    app.settle();
    assert_eq!(
        node_bounds(app.tree(), "Hello"),
        Rect::new(10.0, 20.0, 40.0, 60.0)
    );
}

#[test]
fn named_group_reports_its_response_rectangle() {
    let mut app = mount_rendered(
        Environment::new(),
        || hstack((text("One"), text("Two"))).a11y_label("Pair"),
        WIDTH,
        HEIGHT,
    );
    app.settle();
    let bounds = node_bounds(app.tree(), "Pair");
    assert_answer_sized(bounds);
    assert_centred_in_window(bounds);
}

#[test]
fn named_group_with_allocation_geometry_reports_the_allocation() {
    let mut app = mount_rendered(
        Environment::new(),
        || {
            hstack((text("One"), text("Two")))
                .a11y_label("Pair")
                .a11y_geometry(AccessibilityGeometry::Allocation)
        },
        WIDTH,
        HEIGHT,
    );
    app.settle();
    assert_eq!(node_bounds(app.tree(), "Pair"), window());
}

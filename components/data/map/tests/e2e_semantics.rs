//! End-to-end accessibility semantics coverage for the map component.

use std::time::Duration;

use waterui::ViewExt as _;
use waterui::{Binding, View};
use waterui_map::{Annotation, Coordinate, Latitude, Longitude, Map, Region};
use waterui_testing::{Role, Selector, UiBuilder, WaitOptions, WaitResult};

const fn coordinate(latitude: f64, longitude: f64) -> Coordinate {
    Coordinate::new(
        Latitude::new_unchecked(latitude),
        Longitude::new_unchecked(longitude),
    )
}

const fn initial_region() -> Region {
    Region::new(coordinate(37.7749, -122.4194), 0.12, 0.12)
}

const fn updated_region() -> Region {
    Region::new(coordinate(35.6764, 139.6500), 0.20, 0.20)
}

fn initial_annotations() -> Vec<Annotation> {
    vec![Annotation::new(
        coordinate(37.7749, -122.4194),
        "San Francisco",
    )]
}

fn updated_annotations() -> Vec<Annotation> {
    vec![
        Annotation::new(coordinate(35.6764, 139.6500), "Tokyo Station").subtitle("Transit"),
        Annotation::new(coordinate(35.6895, 139.6917), "Shinjuku"),
    ]
}

#[waterui::test(viewport = (420, 320))]
fn map_exposes_accessibility_surface_and_reactive_annotations(ui: UiBuilder) {
    let region = Binding::container(initial_region());
    let annotations = Binding::container(initial_annotations());
    let region_for_view = region.clone();
    let annotations_for_view = annotations.clone();

    let mut app = ui.mount(move || map_view(region_for_view.clone(), annotations_for_view.clone()));

    app.query()
        .role(Role::IMAGE)
        .label("City map")
        .assert_exists();
    app.query()
        .label("Center: 37.7749, -122.4194 span 0.120 x 0.120")
        .assert_exists();
    app.query().label("Pins: 1").assert_exists();
    app.query()
        .label("Annotations: San Francisco")
        .assert_exists();

    region.set(updated_region());
    annotations.set(updated_annotations());
    assert!(
        app.wait_for(
            &[
                app.expect_exists(
                    Selector::default().label("Center: 35.6764, 139.6500 span 0.200 x 0.200"),
                ),
                app.expect_exists(Selector::default().label("Pins: 2")),
                app.expect_exists(
                    Selector::default().label("Annotations: Tokyo Station (Transit) | Shinjuku"),
                ),
            ],
            WaitOptions::new(Duration::from_millis(250)),
        ) == WaitResult::Completed,
        "map-exposes-accessibility-surface-and-reactive-annotations: expected updated region and annotations to appear"
    );
}

fn map_view(region: Binding<Region>, annotations: Binding<Vec<Annotation>>) -> impl View {
    Map::new(region)
        .annotations(annotations)
        .shows_user_location(true)
        .shows_compass(true)
        .shows_scale(true)
        .a11y_label("City map")
        .size(320.0, 280.0)
}

//! End-to-end accessibility semantics coverage for the map component.

use std::time::Duration;

use waterui::ViewExt as _;
use waterui::{Binding, Environment, View};
use waterui_map::{Annotation, Coordinate, Latitude, Longitude, Map, Region};
use waterui_map_gpu::MapGpuOptions;
use waterui_testing::{Role, Selector, UiBuilder, WaitOptions, WaitResult};
use waterui_url::Url;

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

/// The GPU map is a realization the test installs, exactly as an application
/// installs it in `app(env)`: `waterui-map` is the semantic component and
/// `waterui-map-gpu` — a crate of its own that the `waterui` facade does not
/// carry — supplies the `Hook<MapConfig>` that draws it. Hydrolysis keeps no
/// map bridge, so a `Map` mounted without this environment panics in
/// `native_measure.rs` instead of rendering.
fn map_env() -> Environment {
    let mut env = Environment::new();
    env.insert(MapGpuOptions::new(Url::new(
        "https://tiles.openfreemap.org/styles/positron",
    )));
    waterui_map_gpu::install(&mut env);
    env
}

#[waterui::test(viewport = (420, 320))]
fn map_exposes_accessibility_surface_and_reactive_annotations(ui: UiBuilder) {
    let region = Binding::container(initial_region());
    let annotations = Binding::container(initial_annotations());
    let region_for_view = region.clone();
    let annotations_for_view = annotations.clone();

    let mut app = ui
        .environment(map_env())
        .mount(move || map_view(region_for_view.clone(), annotations_for_view.clone()));

    // The projection draws its whole world into one GPU texture, so assistive
    // technology sees a single image node whose label describes the camera and
    // pins. That label is computed from the same signals the map was built
    // with, which is what makes the reactive assertions below meaningful.
    app.query()
        .role(Role::IMAGE)
        .label("Map centered at 37.7749, -122.4194, 1 pin: San Francisco")
        .assert_exists();

    region.set(updated_region());
    annotations.set(updated_annotations());
    assert!(
        app.wait_for(
            &[app.expect_exists(
                Selector::default()
                    .label("Map centered at 35.6764, 139.6500, 2 pins: Tokyo Station, Shinjuku")
            )],
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
        .size(320.0, 280.0)
}

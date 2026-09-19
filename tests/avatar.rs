//! Semantic acceptance for the `Avatar` composer.
//!
//! Avatar is a Rust-side composition (Framework Design Principle #2) — a
//! stack, a frame, a clip and theme tokens, with no FFI type of its own — so
//! its contract is exactly what the accessibility tree exposes.

use core::time::Duration;
use std::path::{Path, PathBuf};

use waterui::Str;
use waterui::Url;
use waterui::prelude::*;
use waterui_testing::{Role, UiBuilder};

/// A 240×240 test portrait: four saturated quadrants.
///
/// Every feature is chosen to be read off the rendered avatar directly. The
/// quadrant boundaries show whether the picture is centred and undistorted,
/// and the silhouette's edge is the boundary between a saturated colour and
/// the page — which reads in both colour schemes, unlike a light or dark
/// border, either of which disappears into one of the two backgrounds and
/// makes a circular clip look like an octagon.
///
/// The portrait is wider than it is tall and carries a white disc at its
/// centre: an avatar that stretched the picture would show that disc as an
/// ellipse, one that covers and crops keeps it round. Quadrants alone could
/// not tell the two apart, since both leave their boundaries at the centre.
fn write_test_portrait(dir: &Path) -> PathBuf {
    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 200;
    const DISC_RADIUS: f64 = 60.0;
    let path = dir.join("portrait.png");
    let mut pixels = image::RgbaImage::new(WIDTH, HEIGHT);
    let (centre_x, centre_y) = (f64::from(WIDTH) / 2.0, f64::from(HEIGHT) / 2.0);
    for (x, y, pixel) in pixels.enumerate_pixels_mut() {
        let (dx, dy) = (f64::from(x) + 0.5 - centre_x, f64::from(y) + 0.5 - centre_y);
        *pixel = if dx.hypot(dy) <= DISC_RADIUS {
            image::Rgba([255, 255, 255, 255])
        } else {
            match (x < WIDTH / 2, y < HEIGHT / 2) {
                (true, true) => image::Rgba([220, 40, 40, 255]),
                (false, true) => image::Rgba([40, 160, 60, 255]),
                (true, false) => image::Rgba([40, 80, 220, 255]),
                (false, false) => image::Rgba([230, 190, 40, 255]),
            }
        };
    }
    std::fs::create_dir_all(dir).expect("test portrait directory");
    pixels.save(&path).expect("test portrait should encode");
    path
}

fn portrait_url() -> Url {
    let dir = std::env::temp_dir().join("waterui-avatar-tests");
    Url::from_file_path_str(write_test_portrait(&dir).to_string_lossy().into_owned())
}

#[waterui::test]
fn an_initials_avatar_publishes_one_node_carrying_the_name(ui: UiBuilder) {
    let mut app = ui.viewport(120, 120).mount(|| avatar("Ada Lovelace"));

    let nodes = app.query().role(Role::IMAGE).all();
    assert_eq!(
        nodes.len(),
        1,
        "an avatar is one image node, whatever it draws inside itself"
    );
    app.query()
        .role(Role::IMAGE)
        .label("Ada Lovelace")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("AL")
        .assert_not_exists();
}

#[waterui::test]
fn a_pictured_avatar_publishes_the_same_single_node(ui: UiBuilder) {
    let url = portrait_url();
    let mut app = ui
        .viewport(120, 120)
        .mount(move || avatar("Grace Hopper").image(url.clone()));

    let nodes = app.query().role(Role::IMAGE).all();
    assert_eq!(
        nodes.len(),
        1,
        "the decorative picture must not add a second, unlabelled node"
    );
    app.query()
        .role(Role::IMAGE)
        .label("Grace Hopper")
        .assert_exists();
}

#[waterui::test]
fn renaming_relabels_the_node_it_already_published(ui: UiBuilder) {
    let name = Binding::container(Str::from("Ada Lovelace"));
    let name_for_view = name.clone();
    let mut app = ui
        .viewport(120, 120)
        .mount(move || avatar(name_for_view.clone()));

    let before = app
        .query()
        .role(Role::IMAGE)
        .label("Ada Lovelace")
        .single()
        .id();

    name.set(Str::from("Grace Hopper"));
    assert!(
        app.query()
            .role(Role::IMAGE)
            .label("Grace Hopper")
            .wait_for_existence(Duration::from_secs(2)),
        "the avatar's label must follow its name signal"
    );

    let after = app
        .query()
        .role(Role::IMAGE)
        .label("Grace Hopper")
        .single()
        .id();
    assert_eq!(
        before, after,
        "a name change relabels the published node; it must not replace the subtree"
    );
    app.query()
        .role(Role::IMAGE)
        .label("Ada Lovelace")
        .assert_not_exists();
}

#[waterui::test]
fn the_monogram_follows_the_name_without_a_second_node(ui: UiBuilder) {
    let name = Binding::container(Str::from("Ada Lovelace"));
    let name_for_view = name.clone();
    let mut app = ui
        .viewport(120, 120)
        .mount(move || avatar(name_for_view.clone()));

    name.set(Str::from("山田 太郎"));
    assert!(
        app.query()
            .role(Role::IMAGE)
            .label("山田 太郎")
            .wait_for_existence(Duration::from_secs(2)),
        "the avatar's label must follow its name signal"
    );
    assert_eq!(
        app.query().role(Role::IMAGE).all().len(),
        1,
        "the monogram redraw must not publish a node of its own"
    );
}

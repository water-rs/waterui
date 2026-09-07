//! Semantic and visual acceptance for the `Avatar` composer.
//!
//! Avatar is a Rust-side composition (Framework Design Principle #2) — a
//! stack, a frame, a clip and theme tokens, with no FFI type of its own — so
//! its contract is exactly what the accessibility tree exposes plus what the
//! renderer draws. The PNG-producing test is ignored by default and reviewed
//! by eye.

use core::cell::Cell;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use hydrolysis_m3::install as install_m3;
use waterui::Str;
use waterui::media::photo::Event as PhotoEvent;
use waterui::prelude::*;
use waterui::shape::RoundedRectangle;
use waterui::widget::avatar::Avatar;
use waterui_testing::{OffscreenApp, Role, UiBuilder};

/// A 240×240 test portrait: four saturated quadrants.
///
/// Every feature is chosen to be read off the rendered avatar directly. The
/// quadrant boundaries show whether the picture is centred and undistorted,
/// and the silhouette's edge is the boundary between a saturated colour and
/// the page — which reads in both colour schemes, unlike a light or dark
/// border, either of which disappears into one of the two backgrounds and
/// makes a circular clip look like an octagon.
fn write_test_portrait(dir: &Path) -> PathBuf {
    const SIDE: u32 = 240;
    let path = dir.join("portrait.png");
    let mut pixels = image::RgbaImage::new(SIDE, SIDE);
    for (x, y, pixel) in pixels.enumerate_pixels_mut() {
        *pixel = match (x < SIDE / 2, y < SIDE / 2) {
            (true, true) => image::Rgba([220, 40, 40, 255]),
            (false, true) => image::Rgba([40, 160, 60, 255]),
            (true, false) => image::Rgba([40, 80, 220, 255]),
            (false, false) => image::Rgba([230, 190, 40, 255]),
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

#[waterui::test(theme = install_m3)]
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

#[waterui::test(theme = install_m3)]
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

#[waterui::test(theme = install_m3)]
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

#[waterui::test(theme = install_m3)]
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

/// One row of the gallery a reviewer reads: every silhouette, the ring, the
/// monogram and a real picture.
///
/// `observed` is the picture whose load the capture waits on; it is attached to
/// one avatar per gallery so the wait is on a real completion event rather than
/// a guessed duration.
fn row(url: Url, side: f32, observed: Option<Rc<Cell<bool>>>) -> impl View {
    let watched = url.clone();
    hstack((
        avatar("Ada Lovelace").size(side),
        avatar("山田 太郎")
            .size(side)
            .shape(RoundedRectangle::new(0.25)),
        avatar("Katherine Johnson")
            .size(side)
            .ring(Color::new(theme_color::Accent), side / 20.0),
        avatar("Grace Hopper").image(url).size(side),
        observed.map(|observed| {
            Avatar::new("Grace Hopper", move || {
                let observed = Rc::clone(&observed);
                Photo::new(watched.clone())
                    .resizable()
                    .on_event(move |event: PhotoEvent| {
                        if matches!(event, PhotoEvent::Loaded) {
                            observed.set(true);
                        }
                    })
            })
            .size(side)
            .ring(Color::new(theme_color::Accent), side / 20.0)
        }),
    ))
    .spacing(16.0)
}

/// The gallery: a realistic list-row size on top, and the same avatars at a
/// size where the clip's edge, the ring's concentricity and the monogram's
/// optical centring can be judged by eye.
fn gallery(url: Url, loaded: Rc<Cell<bool>>) -> impl View {
    vstack((row(url.clone(), 40.0, None), row(url, 128.0, Some(loaded))))
        .spacing(20.0)
        .padding_with(16.0)
}

fn capture(ui: UiBuilder, stage: &str) {
    let url = portrait_url();
    let loaded = Rc::new(Cell::new(false));
    let observer = Rc::clone(&loaded);
    let mut app: OffscreenApp = ui
        .viewport(820, 236)
        .mount_offscreen(move || gallery(url.clone(), Rc::clone(&observer)));

    assert!(
        app.pump_until(Duration::from_secs(5), || loaded.get()),
        "the test portrait must decode before the gallery is captured"
    );
    let _ = app.capture_snapshot("avatar-preview", "gallery", stage);
}

#[ignore = "writes a visual acceptance PNG for direct image review"]
#[waterui::test(theme = install_m3)]
fn avatar_gallery_light(ui: UiBuilder) {
    capture(ui, "light");
}

#[ignore = "writes a visual acceptance PNG for direct image review"]
#[waterui::test]
fn avatar_gallery_dark(ui: UiBuilder) {
    capture(
        ui.theme(|env: &mut Environment| {
            hydrolysis_m3::install_with_colors(
                env,
                hydrolysis_m3::MaterialColorScheme::baseline_dark(),
            );
        }),
        "dark",
    );
}

//! End-to-end accessibility-semantics tests for the `media` component.

use std::time::Duration;

/// How long a live photo's motion may take to play out and hand the still back.
///
/// This is wall-clock — `SemanticApp::wait_for` pumps against `Instant::now()` —
/// and playback advances a frame at a time. On a machine with no GPU a frame
/// costs orders of magnitude more than on one with a real adapter, so a budget
/// tuned to hardware buys far fewer frames there and the playback never reaches
/// its end. Generous rather than tight: the assertion is that the motion
/// finishes and the still returns, not that it does so quickly.
const MOTION_PLAYBACK_BUDGET: Duration = Duration::from_secs(120);

use hydrolysis_m3::install as install_m3;
use image::ImageEncoder as _;
use waterui::Binding;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_media::{
    LivePhoto, Media, Photo, Url, live::LivePhotoSource, photo::Event as PhotoEvent,
};
use waterui_testing::{Role, Selector, UiBuilder, WaitOptions, WaitResult};

fn sample_image_path() -> String {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("waterui-media-testing-sample-{unique}.png"));
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
            2,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .expect("sample media PNG should encode");
    std::fs::write(&path, png).expect("sample media PNG should write");
    path.to_string_lossy().into_owned()
}

fn sample_video_url() -> Url {
    Url::from_file_path_str(format!(
        "{}/tests/fixtures/live-photo-motion.mp4",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn media_video_view() -> impl waterui::View {
    Media::Video(sample_video_url()).size(420.0, 260.0)
}

fn live_photo_view(source: LivePhotoSource) -> impl waterui::View {
    LivePhoto::new(source)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Sample live photo")
}

fn assert_image_eventually_exists(app: &mut UiTestApp, label: &str) {
    if app.wait_for(
        &[app.expect_exists(Selector::default().role(Role::IMAGE).label(label))],
        WaitOptions::new(Duration::from_millis(750)),
    ) == WaitResult::Completed
    {
        return;
    }
    let images = app.query().role(Role::IMAGE).all();
    let labels = images
        .iter()
        .map(|node| node.node().label().unwrap_or("<missing>").to_owned())
        .collect::<Vec<_>>();
    let tree = app
        .tree()
        .nodes()
        .values()
        .map(|node| format!("{:?}:{:?}", node.role(), node.label()))
        .collect::<Vec<_>>();
    panic!(
        "{label}: expected image semantics to appear after local file load; found {} image node(s): {:?}; tree={:?}",
        labels.len(),
        labels,
        tree
    );
}

type UiTestApp = waterui_testing::SemanticApp;

#[waterui::test(theme = install_m3)]
fn photo_exposes_accessibility_image_after_load(ui: UiBuilder) {
    let sample_path = sample_image_path();
    let loaded = Binding::bool(false);
    let last_event = Binding::container(String::new());
    let loaded_for_view = loaded.clone();
    let last_event_for_view = last_event.clone();
    let mut app = ui.mount(move || {
        let loaded_for_event = loaded_for_view.clone();
        let last_event_for_event = last_event_for_view.clone();
        Photo::from_path(sample_path.clone())
            .on_event(move |event| match event {
                PhotoEvent::Loaded => {
                    loaded_for_event.set(true);
                    last_event_for_event.set(String::from("loaded"));
                }
                PhotoEvent::Error(message) => {
                    last_event_for_event.set(format!("error:{message}"));
                }
            })
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Sample photo")
    });

    assert_image_eventually_exists(&mut app, "Sample photo");
    assert!(
        !last_event.get().starts_with("error:"),
        "photo_exposes_accessibility_image_after_load: {event}",
        event = last_event.get()
    );
    assert!(
        loaded.get(),
        "photo load event should mark the image as loaded"
    );
}

#[waterui::test(theme = install_m3)]
fn media_image_exposes_accessibility_image_after_load(ui: UiBuilder) {
    let sample_path = sample_image_path();
    let mut app = ui.mount(move || {
        Media::Image(Url::from_file_path_str(sample_path.clone()))
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Media image")
    });
    assert_image_eventually_exists(&mut app, "Media image");
}

#[waterui::test(media_video_view, theme = install_m3, viewport = (480, 320))]
fn media_video_uses_video_player_accessibility_controls(app: &mut UiTestApp) {
    app.query().role(Role::BUTTON).label("Play").assert_exists();
    app.query().role(Role::BUTTON).label("Mute").assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Playback speed 1.0 times")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Disable pitch preservation")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Subtitles automatic")
        .assert_exists();
    assert_eq!(
        app.query().role(Role::SLIDER).all().len(),
        2,
        "media-video-uses-video-player-accessibility-controls: expected timeline and volume sliders"
    );
}

#[waterui::test(theme = install_m3)]
fn live_photo_exposes_still_image_accessibility_before_activation(ui: UiBuilder) {
    let sample_path = sample_image_path();
    let source = LivePhotoSource::new(Url::from_file_path_str(sample_path), sample_video_url());
    let mut app = ui.mount(move || live_photo_view(source.clone()));

    assert_image_eventually_exists(&mut app, "Sample live photo");
}

#[waterui::test(theme = install_m3, viewport = (180, 140))]
fn live_photo_long_press_plays_motion_once_and_recovers(ui: UiBuilder) {
    let sample_path = sample_image_path();
    let source = LivePhotoSource::new(Url::from_file_path_str(sample_path), sample_video_url());
    let mut app = ui.mount_offscreen(move || {
        LivePhoto::new(source.clone())
            .activation_duration_ms(40)
            .size(120.0, 80.0)
    });

    let initial_still = app.expect_exists(Selector::default().role(Role::IMAGE));
    assert_eq!(
        app.wait_for(
            &[initial_still],
            WaitOptions::new(Duration::from_millis(750)),
        ),
        WaitResult::Completed,
        "the live photo must expose its initial still image"
    );
    let bounds = app.query().role(Role::IMAGE).single().bounds();
    assert!(
        (bounds.width() - 120.0).abs() < 0.5 && (bounds.height() - 80.0).abs() < 0.5,
        "the live photo still must fill its proposed 120x80 bounds, got {bounds:?}"
    );
    let (center_x, center_y) = bounds.center();

    app.tap_at(center_x, center_y);
    app.query()
        .role(Role::IMAGE)
        .label("Video content")
        .assert_not_exists();

    app.pointer_down_at(center_x, center_y);
    let motion = app.expect_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion], WaitOptions::new(Duration::from_secs(1))),
        WaitResult::Completed,
        "holding past the activation duration must mount live photo motion"
    );
    app.pointer_up_at(center_x, center_y);

    let motion_gone =
        app.expect_not_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion_gone], WaitOptions::new(MOTION_PLAYBACK_BUDGET)),
        WaitResult::Completed,
        "completed motion playback must return to the still photo"
    );
    app.query().role(Role::IMAGE).assert_exists();

    let bounds = app.query().role(Role::IMAGE).single().bounds();
    let (center_x, center_y) = bounds.center();
    app.pointer_down_at(center_x, center_y);
    let motion = app.expect_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion], WaitOptions::new(Duration::from_secs(1))),
        WaitResult::Completed,
        "live photo must support replay after returning to its still image"
    );
    app.pointer_up_at(center_x, center_y);
    let motion_gone =
        app.expect_not_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion_gone], WaitOptions::new(MOTION_PLAYBACK_BUDGET)),
        WaitResult::Completed,
        "replayed motion must also stop after one pass"
    );
    app.query().role(Role::IMAGE).assert_exists();
}

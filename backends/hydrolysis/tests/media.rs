//! Renderer presentation tests for media: the live photo's long-press motion
//! lifecycle through the pointer pipeline.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); the case names its origin file and asserts what it
//! asserted there, mounted under `Material3::defaults()` on the rendered
//! runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// How long a live photo's motion may take to play out and hand the still back.
///
/// This is wall-clock — `SemanticApp::wait_for` pumps against `Instant::now()` —
/// and playback advances a frame at a time, so a runner without a GPU needs
/// considerably longer than the fixture's own 0.75s. Generous rather than
/// tight: the assertion is that the motion finishes and the still returns, not
/// that it does so quickly.
const MOTION_PLAYBACK_BUDGET: Duration = Duration::from_secs(30);

use image::ImageEncoder as _;
use waterui::ViewExt as _;
use waterui_media::{
    LivePhoto, Url,
    live::{Event as LivePhotoEvent, LivePhotoSource},
};
use waterui_testing::{Role, Selector, Styled, UiBuilder, WaitOptions, WaitResult};

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

/// The live photo's motion clip: 96x64, 9 frames at 12fps, AV1 in MP4.
///
/// AV1 deliberately, and not only because it is royalty-free: every platform
/// decodes it through the same `rav1d` software decoder, so this test exercises
/// one decode path everywhere. The H.264 clip it replaced could only be decoded
/// by a platform's hardware codec — Media Foundation on Windows, VA-API on
/// Linux — neither of which exists on a CI runner, which left this test
/// asserting nothing on two of the three platforms it runs on.
fn sample_video_url() -> Url {
    Url::from_file_path_str(format!(
        "{}/tests/fixtures/live-photo-motion.mp4",
        env!("CARGO_MANIFEST_DIR")
    ))
}

// Origin: waterui `components/multimedia/media/tests/e2e_semantics.rs`.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (180, 140))]
fn live_photo_long_press_plays_motion_once_and_recovers(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let sample_path = sample_image_path();
    let source = LivePhotoSource::new(Url::from_file_path_str(sample_path), sample_video_url());
    // Without this the test cannot fail for the reason it exists: motion that
    // errors out unmounts exactly like motion that played, so a runner with no
    // working decoder would satisfy every assertion below in milliseconds.
    let motion_outcomes: Rc<RefCell<Vec<LivePhotoEvent>>> = Rc::default();
    let recorder = Rc::clone(&motion_outcomes);
    let mut app = ui.mount_offscreen(move || {
        let recorder = Rc::clone(&recorder);
        LivePhoto::new(source.clone())
            .on_event(move |event| recorder.borrow_mut().push(event))
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

    // The motion is a transient: settling after the press would wait for the
    // playback it starts, so queue the press and observe the tree instead.
    app.queue_pointer_down_at(center_x, center_y);
    let motion = app.expect_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion], WaitOptions::new(Duration::from_secs(1))),
        WaitResult::Completed,
        "holding past the activation duration must mount live photo motion"
    );
    app.queue_pointer_up_at(center_x, center_y);

    let motion_gone =
        app.expect_not_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion_gone], WaitOptions::new(MOTION_PLAYBACK_BUDGET)),
        WaitResult::Completed,
        "completed motion playback must return to the still photo"
    );
    app.query().role(Role::IMAGE).assert_exists();
    assert_motion_played(&motion_outcomes, 1);

    let bounds = app.query().role(Role::IMAGE).single().bounds();
    let (center_x, center_y) = bounds.center();
    app.queue_pointer_down_at(center_x, center_y);
    let motion = app.expect_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion], WaitOptions::new(Duration::from_secs(1))),
        WaitResult::Completed,
        "live photo must support replay after returning to its still image"
    );
    app.queue_pointer_up_at(center_x, center_y);
    let motion_gone =
        app.expect_not_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(
        app.wait_for(&[motion_gone], WaitOptions::new(MOTION_PLAYBACK_BUDGET)),
        WaitResult::Completed,
        "replayed motion must also stop after one pass"
    );
    app.query().role(Role::IMAGE).assert_exists();
    assert_motion_played(&motion_outcomes, 2);
}

/// Asserts the live photo has played its motion through `plays` times.
///
/// The still photo comes back either way, so the events are the only evidence
/// that anything was decoded: a `MotionFailed` here means this machine could
/// not play the clip, which is a failure to report and not a pass to collect.
fn assert_motion_played(outcomes: &Rc<RefCell<Vec<LivePhotoEvent>>>, plays: usize) {
    let outcomes = outcomes.borrow();
    if let Some(LivePhotoEvent::MotionFailed(message)) = outcomes
        .iter()
        .find(|event| matches!(event, LivePhotoEvent::MotionFailed(_)))
    {
        panic!("live photo motion failed to play instead of finishing: {message}");
    }
    let ended = outcomes
        .iter()
        .filter(|event| matches!(event, LivePhotoEvent::MotionEnded))
        .count();
    assert_eq!(
        ended, plays,
        "expected {plays} completed motion playback(s), observed {outcomes:?}"
    );
}

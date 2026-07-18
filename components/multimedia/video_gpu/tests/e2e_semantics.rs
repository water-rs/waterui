//! End-to-end semantic coverage for `WaterUI` video controls.

use std::{cell::RefCell, rc::Rc, time::Duration};

use hydrolysis_m3::install as install_m3;
use waterui::View;
use waterui::ViewExt as _;
use waterui::env::Environment;
use waterui_testing::{Role, Selector, SemanticApp, WaitOptions, WaitResult, ui};
use waterui_video::{
    Event, MediaItem, PlaybackSession, PlayerController, Playlist, Url, VideoPlayer, video,
};
use waterui_video_gpu::install as install_video_gpu;

fn missing_video_url() -> Url {
    Url::from_file_path_str(format!(
        "{}/tests/fixtures/missing-video.mp4",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn tree_debug(app: &mut waterui_testing::SemanticApp) -> String {
    let buttons = app
        .query()
        .role(Role::BUTTON)
        .all()
        .iter()
        .map(|node| {
            format!(
                "{:?}:{:?}:enabled={}:bounds={:?}",
                node.node().role(),
                node.node().label(),
                node.node().enabled(),
                node.bounds(),
            )
        })
        .collect::<Vec<_>>();
    let sliders = app
        .query()
        .role(Role::SLIDER)
        .all()
        .iter()
        .map(|node| {
            format!(
                "{:?}:{:?}:value={:?}:bounds={:?}",
                node.node().role(),
                node.node().label(),
                node.node().value(),
                node.bounds(),
            )
        })
        .collect::<Vec<_>>();
    let tree = app
        .tree()
        .nodes()
        .values()
        .map(|node| {
            format!(
                "{:?}:{:?}:value={:?}:enabled={}:children={:?}",
                node.role(),
                node.label(),
                node.value(),
                node.enabled(),
                node.children()
            )
        })
        .collect::<Vec<_>>();
    format!("buttons={buttons:?} sliders={sliders:?} tree={tree:?}")
}

fn raw_video_view() -> impl View {
    video(missing_video_url()).size(240.0, 160.0)
}

fn themed_environment() -> Environment {
    let mut env = Environment::new();
    install_m3(&mut env);
    install_video_gpu(&mut env);
    env
}

fn assert_initial_player_semantics(app: &mut SemanticApp) {
    for label in [
        "Play",
        "Mute",
        "Playback speed 1.0 times",
        "Disable pitch preservation",
        "Video quality automatic",
        "Subtitles automatic",
        "Rewind 10 seconds",
        "Forward 10 seconds",
    ] {
        app.query().role(Role::BUTTON).label(label).assert_exists();
    }
    #[cfg(any(target_os = "android", target_os = "ios", target_os = "macos"))]
    app.query()
        .role(Role::BUTTON)
        .label("Enter picture in picture")
        .assert_exists();
    app.query()
        .role(Role::IMAGE)
        .label("Video content")
        .assert_exists();
    assert_eq!(
        app.query().role(Role::SLIDER).all().len(),
        2,
        "video-player-controls-are-accessible-and-reactive: expected timeline and volume sliders"
    );
    app.query()
        .role(Role::BUTTON)
        .label("Previous")
        .assert_not_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Next")
        .assert_not_exists();
}

fn assert_queue_navigation_reacts(app: &mut SemanticApp, controller: &PlayerController) {
    controller.add_item(MediaItem::from(missing_video_url()));
    assert!(
        app.wait_for(
            &[app.expect_exists(Selector::default().role(Role::BUTTON).label("Next"))],
            WaitOptions::new(Duration::from_millis(200)),
        ) == WaitResult::Completed,
        "video-player-controls-are-accessible-and-reactive: next should appear after appending an item; {}",
        tree_debug(app)
    );
    controller
        .next()
        .expect("the appended playlist item must be navigable");
    assert_eq!(
        app.wait_for(
            &[app.expect_exists(Selector::default().role(Role::BUTTON).label("Previous"))],
            WaitOptions::new(Duration::from_millis(200)),
        ),
        WaitResult::Completed,
        "video-player-controls-are-accessible-and-reactive: previous should appear after advancing; {}",
        tree_debug(app)
    );
    app.query()
        .role(Role::BUTTON)
        .label("Next")
        .assert_not_exists();
}

fn tap_and_assert_label(app: &mut SemanticApp, current: &str, updated: &str) {
    assert!(
        app.query().role(Role::BUTTON).label(current).tap(),
        "video-player-controls-are-accessible-and-reactive: `{current}` action should succeed"
    );
    let expected = app.expect_exists(Selector::default().role(Role::BUTTON).label(updated));
    assert_eq!(
        app.wait_for(&[expected], WaitOptions::new(Duration::from_millis(200))),
        WaitResult::Completed,
        "video-player-controls-are-accessible-and-reactive: `{updated}` label should appear; {}",
        tree_debug(app)
    );
}

fn assert_non_transport_controls_react(app: &mut SemanticApp) {
    tap_and_assert_label(app, "Mute", "Unmute");
    tap_and_assert_label(app, "Playback speed 1.0 times", "Playback speed 1.5 times");
    tap_and_assert_label(
        app,
        "Disable pitch preservation",
        "Enable pitch preservation",
    );
    assert!(
        app.query().role(Role::SLIDER).label("Volume").increment(),
        "video-player-controls-are-accessible-and-reactive: volume adjustment should succeed"
    );
}

#[waterui::test(raw_video_view)]
fn raw_video_exposes_default_accessibility_image(app: &mut SemanticApp) {
    assert_eq!(
        app.query().role(Role::IMAGE).all().len(),
        1,
        "raw-video-exposes-explicit-accessibility-image: expected exactly one image node"
    );
    let node = app
        .query()
        .role(Role::IMAGE)
        .label("Video content")
        .single();
    let bounds = node.bounds();
    assert!(
        bounds.width() > 0.0,
        "raw-video-exposes-explicit-accessibility-image: width must be positive"
    );
    assert!(
        bounds.height() > 0.0,
        "raw-video-exposes-explicit-accessibility-image: height must be positive"
    );
}

#[test]
fn video_player_controls_are_accessible_and_reactive() {
    let mounted_controller = Rc::new(RefCell::new(None));
    let controller_for_view = Rc::clone(&mounted_controller);

    let mut app = ui()
        .environment(themed_environment())
        .viewport(480, 320)
        .mount(move || {
            let session = PlaybackSession::new(Playlist::single(missing_video_url()));
            controller_for_view.replace(Some(session.controller()));
            VideoPlayer::new(session).size(420.0, 260.0)
        });
    let controller = mounted_controller
        .borrow()
        .clone()
        .expect("mounting the player must publish its controller to the test");

    assert_initial_player_semantics(&mut app);
    assert_queue_navigation_reacts(&mut app, &controller);
    tap_and_assert_label(&mut app, "Play", "Pause");
    assert_non_transport_controls_react(&mut app);
}

#[test]
fn self_drawn_player_controls_render_without_retrying_a_failed_source() {
    let decoder_errors = Rc::new(RefCell::new(Vec::new()));
    let decoder_errors_for_view = Rc::clone(&decoder_errors);
    let mut app = ui()
        .environment(themed_environment())
        .viewport(480, 320)
        .theme(|_: &mut Environment| {})
        .mount(move || {
            let session = PlaybackSession::new(Playlist::single(missing_video_url()));
            VideoPlayer::new(session)
                .on_event({
                    let decoder_errors = Rc::clone(&decoder_errors_for_view);
                    move |event: Event| {
                        if let Event::Error { message } = event {
                            decoder_errors.borrow_mut().push(message);
                        }
                    }
                })
                .size(420.0, 260.0)
        });

    assert_initial_player_semantics(&mut app);
    assert_non_transport_controls_react(&mut app);

    let captured = app.capture_snapshot("video", "video-player-controls", "after-interactions");
    assert_eq!(
        (captured.snapshot().width, captured.snapshot().height),
        (480, 320),
        "self-drawn player snapshot must preserve the configured viewport",
    );
    assert_eq!(
        decoder_errors.borrow().len(),
        1,
        "a failed source must emit once and remain failed across later render frames: {:?}",
        decoder_errors.borrow(),
    );
}

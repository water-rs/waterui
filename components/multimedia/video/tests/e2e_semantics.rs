//! End-to-end semantic coverage for `WaterUI` video controls.

use std::time::Duration;

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::{Binding, View};
use waterui_testing::{Role, Selector, SemanticApp, WaitOptions, WaitResult, ui};
use waterui_video::{Url, Video, VideoPlayer};

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
                "{:?}:{:?}:enabled={}",
                node.node().role(),
                node.node().label(),
                node.node().enabled()
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
    format!("buttons={buttons:?} tree={tree:?}")
}

fn raw_video_view() -> impl View {
    Video::new(missing_video_url())
        .size(240.0, 160.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Raw video")
}

#[waterui::test(raw_video_view)]
fn raw_video_exposes_explicit_accessibility_image(app: &mut SemanticApp) {
    assert_eq!(
        app.query().role(Role::IMAGE).all().len(),
        1,
        "raw-video-exposes-explicit-accessibility-image: expected exactly one image node"
    );
    let node = app.query().role(Role::IMAGE).single();
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
    let has_previous = Binding::bool(false);
    let has_next = Binding::bool(false);
    let has_previous_for_view = has_previous.clone();
    let has_next_for_view = has_next.clone();

    let mut app = ui().viewport(480, 320).mount(move || {
        VideoPlayer::new(missing_video_url())
            .has_previous(&has_previous_for_view)
            .has_next(&has_next_for_view)
            .size(420.0, 260.0)
    });

    app.query().role(Role::BUTTON).label("Play").assert_exists();
    app.query().role(Role::BUTTON).label("Mute").assert_exists();
    assert_eq!(
        app.query().role(Role::SLIDER).all().len(),
        3,
        "video-player-controls-are-accessible-and-reactive: expected timeline, volume, and speed sliders"
    );
    app.query()
        .role(Role::BUTTON)
        .label("Previous")
        .assert_not_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Next")
        .assert_not_exists();

    has_previous.set(true);
    has_next.set(true);
    assert!(
        app.wait_for(
            &[
                app.expect_exists(Selector::default().role(Role::BUTTON).label("Previous")),
                app.expect_exists(Selector::default().role(Role::BUTTON).label("Next")),
            ],
            WaitOptions::new(Duration::from_millis(200)),
        ) == WaitResult::Completed,
        "video-player-controls-are-accessible-and-reactive: queue navigation buttons should appear when bindings enable them; {}",
        tree_debug(&mut app)
    );

    assert!(
        app.query().role(Role::BUTTON).label("Play").tap(),
        "video-player-controls-are-accessible-and-reactive: play button tap should succeed"
    );
    app.query()
        .role(Role::BUTTON)
        .label("Pause")
        .assert_exists();

    assert!(
        app.query().role(Role::BUTTON).label("Mute").tap(),
        "video-player-controls-are-accessible-and-reactive: mute button tap should succeed"
    );
    app.query()
        .role(Role::BUTTON)
        .label("Unmute")
        .assert_exists();
}

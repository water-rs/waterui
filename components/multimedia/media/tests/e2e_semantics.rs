//! End-to-end accessibility-semantics tests for the `media` component.

use std::time::Duration;

use image::ImageEncoder as _;
use waterui::Binding;
use waterui::OneOf2;
use waterui::Signal;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::component::label;
use waterui::component::lazy::Lazy;
use waterui::component::text;
use waterui::layout::frame::Frame;
use waterui::layout::scroll::scroll;
use waterui_media::{
    LivePhoto, Media, MediaPicker, Photo, PhotoCache, PhotoEvent, PhotoStatus, Url,
    live::LivePhotoSource, media_picker::Selected, photo,
};
use waterui_testing::{Role, UiBuilder};
use waterui_testing::{RuntimeDriver, SemanticApp, TestStyle};

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

fn media_video_view() -> impl waterui::View {
    Media::Video(sample_video_url()).size(420.0, 260.0)
}

fn live_photo_view(source: LivePhotoSource) -> impl waterui::View {
    LivePhoto::new(source)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Sample live photo")
}

fn assert_image_eventually_exists<R: RuntimeDriver>(app: &mut SemanticApp<R>, label: &str) {
    if app
        .query()
        .role(Role::IMAGE)
        .label(label)
        .wait_for_existence(Duration::from_millis(750))
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

#[waterui::test()]
fn photo_exposes_accessibility_image_after_load(ui: UiBuilder) {
    let sample_path = sample_image_path();
    let loaded = Binding::bool(false);
    let last_event = Binding::container(String::new());
    let loaded_for_view = loaded.clone();
    let last_event_for_view = last_event.clone();
    // `PhotoEvent::Loaded` waits for the frame's registration with the engine,
    // which only the rendered runtime performs — the semantic runtime never
    // records a scene.
    let mut app = ui.theme(TestStyle::default()).mount_offscreen(move || {
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
                other => panic!("unexpected photo event {other:?}"),
            })
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Sample photo")
    });

    assert_image_eventually_exists(app.semantic_mut(), "Sample photo");
    assert!(
        !last_event.snapshot().starts_with("error:"),
        "photo_exposes_accessibility_image_after_load: {event}",
        event = last_event.snapshot()
    );
    assert!(
        loaded.snapshot(),
        "photo load event should mark the image as loaded"
    );
}

#[waterui::test()]
fn media_image_exposes_accessibility_image_after_load(ui: UiBuilder) {
    let sample_path = sample_image_path();
    let mut app = ui.mount(move || {
        Media::Image(Url::from_file_path_str(sample_path.clone()))
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Media image")
    });
    assert_image_eventually_exists(&mut app, "Media image");
}

#[waterui::test(media_video_view, viewport = (480, 320))]
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

#[waterui::test()]
fn live_photo_exposes_still_image_accessibility_before_activation(ui: UiBuilder) {
    let sample_path = sample_image_path();
    let source = LivePhotoSource::new(Url::from_file_path_str(sample_path), sample_video_url());
    let mut app = ui.mount(move || live_photo_view(source.clone()));

    assert_image_eventually_exists(&mut app, "Sample live photo");
}

#[waterui::test()]
fn photo_placeholder_failure_and_status_track_each_source(ui: UiBuilder) {
    let valid_path = sample_image_path();
    let missing_path = format!("{valid_path}.missing");
    let source = Binding::container(Url::from_file_path_str(valid_path));
    let status = Binding::container(PhotoStatus::Loading);
    let source_for_view = source.clone();
    let status_for_view = status.clone();

    let mut app = ui.theme(TestStyle::default()).mount_offscreen(move || {
        photo(source_for_view.clone())
            .resizable()
            .placeholder(|| text("Loading photo"))
            .on_failure(|reason: &str| waterui::text!("Load failed: {reason}"))
            .status(&status_for_view)
            .size(120.0, 80.0)
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Tracked photo")
    });

    assert_image_eventually_exists(app.semantic_mut(), "Tracked photo");
    assert_eq!(status.snapshot(), PhotoStatus::Ready);
    assert!(
        !app.semantic_mut()
            .query()
            .role(Role::LABEL)
            .label("Loading photo")
            .exists(),
        "placeholder must hide once the photo settles"
    );

    // Switching to a missing source resets status to Loading and settles on Failed + failure view.
    source.set(Url::from_file_path_str(missing_path));
    assert_eq!(
        status.snapshot(),
        PhotoStatus::Loading,
        "changing source must reset bound status to Loading"
    );
    app.semantic_mut()
        .query()
        .role(Role::LABEL)
        .assert_exists_within(Duration::from_millis(750));
    let reason = status
        .snapshot()
        .failure()
        .expect("missing file must settle PhotoStatus::Failed")
        .to_owned();
    assert_ne!(reason, "");
    app.semantic_mut()
        .query()
        .role(Role::LABEL)
        .label(format!("Load failed: {reason}"))
        .assert_exists();
}

#[derive(Clone, waterui::Identifiable)]
struct LazyRowItem {
    #[id]
    id: usize,
    url: Option<Url>,
}

#[waterui::test()]
fn photo_in_lazy_list_serves_from_environment_cache_without_refetch(ui: UiBuilder) {
    let row_zero_path = sample_image_path();
    let row_zero_url = Url::from_file_path_str(row_zero_path.clone());
    let row_zero_status = Binding::container(PhotoStatus::Loading);
    let status_for_view = row_zero_status.clone();
    let cache = PhotoCache::new(1024 * 1024);

    let mut items =
        waterui::reactive::collection::List::from_iter((0..12_usize).map(|id| LazyRowItem {
            id,
            url: (id == 0).then(|| row_zero_url.clone()),
        }));
    let items_for_view = items.clone();

    let mut app = ui
        .with(cache.clone())
        .viewport(320, 200)
        .theme(TestStyle::default())
        .mount_offscreen(move || {
            let status_for_view = status_for_view.clone();
            scroll(Lazy::for_each(items_for_view.clone(), move |row| {
                if let Some(url) = row.url {
                    OneOf2::A(
                        photo(url)
                            .resizable()
                            .placeholder(waterui::component::progress::loading)
                            .on_failure(|err: &str| waterui::text!("Row 0 error: {err}"))
                            .status(&status_for_view)
                            .size(160.0, 60.0)
                            .a11y_role(AccessibilityRole::Image)
                            .a11y_label("Row 0 photo"),
                    )
                } else {
                    let row_id = row.id;
                    OneOf2::B(
                        Frame::new(waterui::text!("Row {row_id}"))
                            .width(160.0)
                            .height(60.0),
                    )
                }
            }))
        });

    assert_image_eventually_exists(app.semantic_mut(), "Row 0 photo");
    assert_eq!(row_zero_status.snapshot(), PhotoStatus::Ready);
    assert_eq!(cache.len(), 1);

    // Delete the file on disk so any refetch on remount would fail with ENOENT.
    std::fs::remove_file(&row_zero_path).expect("temporary sample PNG must be removable");

    // Remove row 0 from the lazy collection so the row unmounts completely.
    let _ = items.remove(0);
    app.semantic_mut().settle();
    assert!(
        !app.semantic_mut()
            .query()
            .role(Role::IMAGE)
            .label("Row 0 photo")
            .exists(),
        "removing row 0 must unmount its Photo view"
    );

    // Re-insert row 0 at the top: a brand-new `Photo` mounts and serves from `PhotoCache`
    // without touching the deleted file on disk.
    items.insert(
        0,
        LazyRowItem {
            id: 0,
            url: Some(row_zero_url),
        },
    );
    assert_image_eventually_exists(app.semantic_mut(), "Row 0 photo");
    assert_eq!(
        row_zero_status.snapshot(),
        PhotoStatus::Ready,
        "remounted row 0 must settle Ready from PhotoCache even after disk source removal"
    );
}

#[waterui::test(viewport = (320, 200))]
fn media_picker_exposes_label_to_accessibility_tree(ui: UiBuilder) {
    let selection = Binding::container(None::<Selected>);
    let selection_for_second = selection.clone();

    let mut app = ui.mount(move || {
        waterui::component::vstack((
            MediaPicker::new("Choose photo", &selection),
            MediaPicker::new(label("Choose clip").icon(()), &selection_for_second),
        ))
    });

    app.query()
        .role(Role::BUTTON)
        .label("Choose photo")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Choose clip")
        .assert_exists();
}

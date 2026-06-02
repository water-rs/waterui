//! Media Picker Example - Demonstrates media selection and loading
//!
//! This example showcases:
//! - MediaPicker component for selecting photos, videos, and live photos
//! - Selected::load() for retrieving selected media content
//! - Displaying loaded media (Photo, Video, LivePhoto)
//! - Filter options for different media types

use std::rc::Rc;

use waterui::app::App;
use waterui::component::Dynamic;
use waterui::media::live::LivePhotoSource;
use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};
use waterui::media::{LivePhoto, Media};
use waterui::prelude::theme_color::{Accent, MutedForeground};
use waterui::prelude::*;
use waterui::reactive::{binding, impl_constant};
use waterui::{AnyView, Signal};

/// Combined state for the media display area
#[derive(Debug, Clone, PartialEq)]
enum DisplayState {
    Empty,
    Loaded(Media),
    Error(String),
}

impl_constant!(DisplayState);

fn main() -> impl View {
    // Single state binding for cleaner reactivity
    let display_state: Binding<DisplayState> = binding(DisplayState::Empty);
    let image_selection: Binding<Option<Selected>> = Binding::default();
    let video_selection: Binding<Option<Selected>> = Binding::default();
    let live_photo_selection: Binding<Option<Selected>> = Binding::default();

    // Main layout
    vstack((
        // Title
        text("Media Picker Demo").title().bold().padding_with(16.0),
        // Picker buttons row
        hstack((
            picker_button(
                "Pick Image",
                MediaFilter::Image,
                &image_selection,
                &display_state,
            ),
            picker_button(
                "Pick Video",
                MediaFilter::Video,
                &video_selection,
                &display_state,
            ),
            picker_button(
                "Pick Live Photo",
                MediaFilter::LivePhoto,
                &live_photo_selection,
                &display_state,
            ),
        ))
        .spacing(12.0)
        .padding_with(16.0),
        // Divider
        Divider,
        // Media display area
        media_display_area(display_state.clone()),
        spacer(),
    ))
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

/// Creates a picker button that opens the media picker with the given filter
fn picker_button(
    label: &'static str,
    filter: MediaFilter,
    selection: &Binding<Option<Selected>>,
    display_state: &Binding<DisplayState>,
) -> impl View {
    let state = display_state.clone();
    let sel = selection.clone();
    let expected_filter = filter.clone();

    // Create media picker and watch for selection changes
    MediaPicker::new(&sel)
        .filter(filter.clone())
        .label(text(label))
        .on_change(&sel, {
            let state = state.clone();
            let expected_filter = expected_filter.clone();
            move |new_selection| {
                let Some(selected) = new_selection else {
                    state.set(DisplayState::Empty);
                    return;
                };

                let media = selected.load();
                tracing::debug!("Loaded media: {:?}", media);
                match validate_media_result(&media, &expected_filter) {
                    Ok(()) => state.set(DisplayState::Loaded(media)),
                    Err(message) => {
                        tracing::error!("{message}");
                        state.set(DisplayState::Error(message));
                    }
                }
            }
        })
}

/// Displays the loaded media or a placeholder.
fn media_display_area(display_state: Binding<DisplayState>) -> impl View {
    signal_driven_display(display_state, |state| match state {
        DisplayState::Empty => AnyView::new(
            vstack((
                text("No media selected")
                    .sub_headline()
                    .foreground(MutedForeground),
                text("Tap a button above to select media")
                    .body()
                    .foreground(MutedForeground),
            ))
            .spacing(8.0),
        ),
        DisplayState::Loaded(media) => AnyView::new(media_view(media)),
        DisplayState::Error(message) => AnyView::new(
            vstack((
                text("Error").sub_headline().bold().foreground(Accent),
                text(message).body().foreground(MutedForeground),
            ))
            .spacing(8.0)
            .padding_with(16.0),
        ),
    })
}

/// Creates a view for the loaded media based on its type
fn media_view(media: Media) -> AnyView {
    match media {
        Media::Image(url) => AnyView::new(image_view(url)),
        Media::Video(url) => AnyView::new(video_view(url)),
        Media::LivePhoto(source) => AnyView::new(live_photo_view(source)),
    }
}

fn signal_driven_display(
    source: Binding<DisplayState>,
    build: impl Fn(DisplayState) -> AnyView + 'static,
) -> impl View {
    let (handler, dynamic) = Dynamic::new();
    let build = Rc::new(build);
    handler.set(build(source.get()));

    let guard = source.watch({
        let build = Rc::clone(&build);
        move |ctx| {
            let metadata = ctx.metadata().clone();
            handler.set_with_metadata(build(ctx.into_value()), metadata);
        }
    });

    dynamic.retain((guard, source, build))
}

fn image_view(url: Url) -> impl View {
    tracing::debug!("Displaying image from: {}", url);
    vstack((
        Photo::new(url.clone()).on_event(move |event| {
            tracing::debug!("Photo event: {:?}", event);
        }),
        text("Image")
            .body()
            .foreground(MutedForeground)
            .padding_with(8.0),
    ))
}

fn video_view(url: Url) -> impl View {
    tracing::debug!("Displaying video from: {}", url);
    vstack((
        VideoPlayer::new(url)
            .show_controls(true)
            .aspect_ratio(AspectRatio::Fit),
        text("Video")
            .body()
            .foreground(MutedForeground)
            .padding_with(8.0),
    ))
}

fn live_photo_view(source: LivePhotoSource) -> impl View {
    tracing::debug!("Displaying live photo");
    vstack((
        LivePhoto::new(source),
        text("Live Photo")
            .body()
            .foreground(MutedForeground)
            .padding_with(8.0),
    ))
}

fn validate_media_result(media: &Media, expected_filter: &MediaFilter) -> Result<(), String> {
    // Sanity check: if native says "image" but returns a video file URL, that's a bug.
    if let Media::Image(url) = media
        && looks_like_video_url(url)
    {
        return Err(format!(
            "BUG: native returned a video URL but labeled it as an image: {url}"
        ));
    }

    // Fast-fail for mismatched filter/type contracts.
    let matches_filter = match expected_filter {
        MediaFilter::Image => matches!(media, Media::Image(_)),
        MediaFilter::Video => matches!(media, Media::Video(_)),
        MediaFilter::LivePhoto => matches!(media, Media::LivePhoto(_)),
        MediaFilter::All(filters) | MediaFilter::Any(filters) => filters.iter().any(|f| {
            matches!(
                (f, media),
                (MediaFilter::Image, Media::Image(_))
                    | (MediaFilter::Video, Media::Video(_))
                    | (MediaFilter::LivePhoto, Media::LivePhoto(_))
            )
        }),
        MediaFilter::Not(filters) => !filters.iter().any(|f| {
            matches!(
                (f, media),
                (MediaFilter::Image, Media::Image(_))
                    | (MediaFilter::Video, Media::Video(_))
                    | (MediaFilter::LivePhoto, Media::LivePhoto(_))
            )
        }),
    };

    if matches_filter {
        Ok(())
    } else {
        Err(format!(
            "BUG: native returned {media:?} for requested filter {expected_filter:?}"
        ))
    }
}

fn looks_like_video_url(url: &Url) -> bool {
    let raw = url.as_str();
    let without_query = raw.split(['?', '#']).next().unwrap_or(raw);
    let filename = without_query.rsplit('/').next().unwrap_or(without_query);
    let Some((_, extension)) = filename.rsplit_once('.') else {
        return false;
    };
    extension.eq_ignore_ascii_case("mp4")
        || extension.eq_ignore_ascii_case("mov")
        || extension.eq_ignore_ascii_case("m4v")
        || extension.eq_ignore_ascii_case("mkv")
        || extension.eq_ignore_ascii_case("webm")
        || extension.eq_ignore_ascii_case("avi")
        || extension.eq_ignore_ascii_case("mpg")
        || extension.eq_ignore_ascii_case("mpeg")
}

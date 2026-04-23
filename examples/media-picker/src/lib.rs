//! Media Picker Example - Demonstrates media selection and loading
//!
//! This example showcases:
//! - MediaPicker component for selecting photos, videos, and live photos
//! - Selected::load() for asynchronously loading media content
//! - Displaying loaded media (Photo, Video, LivePhoto)
//! - Filter options for different media types

use waterui::app::App;
use waterui::component::Dynamic;
use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};
use waterui::media::{LivePhoto, Media};
use waterui::prelude::theme_color::{Accent, MutedForeground};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::task::spawn_local;
use waterui::{view, view_builder};

/// Combined state for the media display area
#[derive(Debug, Clone, PartialEq)]
enum DisplayState {
    Empty,
    Loading,
    Loaded(Media),
    Error(String),
}

fn main() -> impl View {
    // Single state binding for cleaner reactivity
    let display_state: Binding<DisplayState> = binding(DisplayState::Empty);
    let load_revision = Binding::i32(0);
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
                &load_revision,
                &display_state,
            ),
            picker_button(
                "Pick Video",
                MediaFilter::Video,
                &video_selection,
                &load_revision,
                &display_state,
            ),
            picker_button(
                "Pick Live Photo",
                MediaFilter::LivePhoto,
                &live_photo_selection,
                &load_revision,
                &display_state,
            ),
        ))
        .spacing(12.0)
        .padding_with(16.0),
        // Divider
        Divider,
        // Media display area - single Dynamic::watch, no nesting
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
    load_revision: &Binding<i32>,
    display_state: &Binding<DisplayState>,
) -> impl View {
    let state = display_state.clone();
    let load_revision = load_revision.clone();
    let sel = selection.clone();
    let expected_filter = filter.clone();

    // Create media picker and watch for selection changes
    MediaPicker::new(&sel)
        .filter(filter.clone())
        .label(text(label))
        .on_change(&sel, {
            let state = state.clone();
            let expected_filter = expected_filter.clone();
            let load_revision = load_revision.clone();
            move |new_selection| {
                let revision = load_revision.get().wrapping_add(1);
                load_revision.set(revision);

                let Some(selected) = new_selection else {
                    state.set(DisplayState::Empty);
                    return;
                };

                // Show loading state immediately
                state.set(DisplayState::Loading);

                let state = state.clone();
                let expected_filter = expected_filter.clone();
                let load_revision = load_revision.clone();

                // Load the selected media asynchronously
                spawn_local(async move {
                    let media = selected.load().await;

                    if load_revision.get() != revision {
                        tracing::debug!(
                            "Discarding stale media load completion: revision {revision}"
                        );
                        return;
                    }

                    tracing::debug!("Loaded media: {:?}", media);
                    match validate_media_result(&media, &expected_filter) {
                        Ok(()) => state.set(DisplayState::Loaded(media)),
                        Err(message) => {
                            tracing::error!("{message}");
                            state.set(DisplayState::Error(message));
                        }
                    }
                })
                .detach();
            }
        })
}

/// Displays the loaded media or a placeholder - single Dynamic::watch
fn media_display_area(display_state: Binding<DisplayState>) -> impl View {
    Dynamic::watch(display_state, move |state| {
        view! {
            match state {
                DisplayState::Empty => vstack((
                    text("No media selected")
                        .sub_headline()
                        .foreground(MutedForeground),
                    text("Tap a button above to select media")
                        .body()
                        .foreground(MutedForeground),
                ))
                .spacing(8.0),

                DisplayState::Loading => vstack((
                    loading(),
                    text("Loading media...").body().foreground(MutedForeground),
                ))
                .spacing(12.0),

                DisplayState::Loaded(media) => media_view(media),

                DisplayState::Error(message) => vstack((
                    text("Error").sub_headline().bold().foreground(Accent),
                    text(message).body().foreground(MutedForeground),
                ))
                .spacing(8.0)
                .padding_with(16.0),
            }
        }
    })
}

/// Creates a view for the loaded media based on its type
#[view_builder]
fn media_view(media: Media) -> impl View {
    match media {
        Media::Image(url) => {
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
        Media::Video(url) => {
            tracing::debug!("Displaying video from: {}", url);
            video_view(url)
        }
        Media::LivePhoto(source) => {
            tracing::debug!("Displaying live photo");
            vstack((
                LivePhoto::new(source),
                text("Live Photo")
                    .body()
                    .foreground(MutedForeground)
                    .padding_with(8.0),
            ))
        }
    }
}

fn video_view(url: Url) -> impl View {
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

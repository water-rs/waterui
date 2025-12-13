//! Media Picker Example - Demonstrates media selection and loading
//!
//! This example showcases:
//! - MediaPicker component for selecting photos, videos, and live photos
//! - Selected::load() for asynchronously loading media content
//! - Displaying loaded media (Photo, Video, LivePhoto)
//! - Filter options for different media types

use waterui::app::App;
use waterui::component::Dynamic;
use waterui::media::Media;
use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::task::spawn_local;

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
    let image_selection: Binding<Option<Selected>> = Binding::default();
    let video_selection: Binding<Option<Selected>> = Binding::default();
    let live_photo_selection: Binding<Option<Selected>> = Binding::default();

    // Main layout
    vstack((
        // Title
        text("Media Picker Demo")
            .size(28.0)
            .bold()
            .padding_with(16.0),
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
                if let Some(selected) = new_selection {
                    // Show loading state immediately
                    state.set(DisplayState::Loading);

                    let state = state.clone();
                    let expected_filter = expected_filter.clone();

                    // Load the selected media asynchronously
                    spawn_local(async move {
                        let media = selected.load().await;
                        log::debug!("Loaded media: {:?}", media);
                        match validate_media_result(&media, &expected_filter) {
                            Ok(()) => state.set(DisplayState::Loaded(media)),
                            Err(message) => {
                                log::error!("{message}");
                                state.set(DisplayState::Error(message));
                            }
                        }
                    })
                    .detach();
                }
            }
        })
}

/// Displays the loaded media or a placeholder - single Dynamic::watch
fn media_display_area(display_state: Binding<DisplayState>) -> impl View {
    Dynamic::watch(display_state, move |state| match state {
        DisplayState::Empty => vstack((
            text("No media selected")
                .size(18.0)
                .foreground(theme_color::MutedForeground),
            text("Tap a button above to select media")
                .size(14.0)
                .foreground(theme_color::MutedForeground),
        ))
        .spacing(8.0)
        .anyview(),

        DisplayState::Loading => vstack((
            loading(),
            text("Loading media...").foreground(theme_color::MutedForeground),
        ))
        .spacing(12.0)
        .anyview(),

        DisplayState::Loaded(media) => media_view(media),

        DisplayState::Error(message) => vstack((
            text("Error")
                .size(18.0)
                .bold()
                .foreground(theme_color::Accent),
            text(message)
                .size(14.0)
                .foreground(theme_color::MutedForeground),
        ))
        .spacing(8.0)
        .padding_with(16.0)
        .anyview(),
    })
}

/// Creates a view for the loaded media based on its type
fn media_view(media: Media) -> AnyView {
    match media {
        Media::Image(url) => {
            log::debug!("Displaying image from: {}", url);
            vstack((
                Photo::new(url.clone()).on_event(move |event| {
                    log::debug!("Photo event: {:?}", event);
                }),
                text("Image")
                    .size(14.0)
                    .foreground(theme_color::MutedForeground)
                    .padding_with(8.0),
            ))
            .anyview()
        }
        Media::Video(url) => {
            log::debug!("Displaying video from: {}", url);
            video_view(url)
        }
        Media::LivePhoto(source) => {
            log::debug!("Displaying live photo");
            vstack((
                live_photo_view(source),
                text("Live Photo")
                    .size(14.0)
                    .foreground(theme_color::MutedForeground)
                    .padding_with(8.0),
            ))
            .anyview()
        }
    }
}

fn video_view(url: Url) -> AnyView {
    vstack((
        VideoPlayer::new(url)
            .show_controls(true)
            .aspect_ratio(AspectRatio::Fit),
        text("Video")
            .size(14.0)
            .foreground(theme_color::MutedForeground)
            .padding_with(8.0),
    ))
    .anyview()
}

fn live_photo_view(source: waterui::media::live::LivePhotoSource) -> AnyView {
    let is_playing = binding(false);
    let image_url = source.image.clone();
    let video_url = source.video.clone();

    Dynamic::watch(is_playing.clone(), move |playing| {
        if playing {
            Video::new(video_url.clone())
                .loops(false)
                .on_event({
                    let is_playing = is_playing.clone();
                    move |event| match event {
                        video::Event::Ended | video::Event::Error { .. } => {
                            is_playing.set(false);
                        }
                        _ => {}
                    }
                })
                .anyview()
        } else {
            Photo::new(image_url.clone())
                .on_tap({
                    let is_playing = is_playing.clone();
                    move || is_playing.set(true)
                })
                .overlay(
                    button(text("Play"))
                        .action({
                            let is_playing = is_playing.clone();
                            move || is_playing.set(true)
                        })
                        .padding_with(10.0)
                        .background(Color::from(theme_color::Surface)),
                )
                .anyview()
        }
    })
    .anyview()
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

waterui_ffi::export!();

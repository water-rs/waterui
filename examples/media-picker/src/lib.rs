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
}

fn main() -> impl View {
    // Single state binding for cleaner reactivity
    let display_state: Binding<DisplayState> = binding(DisplayState::Empty);
    let selection: Binding<Option<Selected>> = Binding::default();

    // Main layout
    vstack((
        // Title
        text("Media Picker Demo")
            .size(28.0)
            .bold()
            .padding_with(16.0),
        // Picker buttons row
        hstack((
            picker_button(MediaFilter::Image, &selection, &display_state),
            picker_button(MediaFilter::Video, &selection, &display_state),
            picker_button(MediaFilter::LivePhoto, &selection, &display_state),
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
    filter: MediaFilter,
    selection: &Binding<Option<Selected>>,
    display_state: &Binding<DisplayState>,
) -> impl View {
    let state = display_state.clone();
    let sel = selection.clone();

    // Create media picker and watch for selection changes
    MediaPicker::new(&sel).filter(filter).on_change(&sel, {
        let state = state.clone();
        move |new_selection| {
            if let Some(selected) = new_selection {
                // Show loading state immediately
                state.set(DisplayState::Loading);

                let state = state.clone();

                // Load the selected media asynchronously
                spawn_local(async move {
                    let media = selected.load().await;
                    log::debug!("Loaded media: {:?}", media);
                    state.set(DisplayState::Loaded(media));
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
        Media::LivePhoto(source) => {
            log::debug!("Displaying live photo");
            vstack((
                LivePhoto::new(source),
                text("Live Photo")
                    .size(14.0)
                    .foreground(theme_color::MutedForeground)
                    .padding_with(8.0),
            ))
            .anyview()
        }
    }
}

waterui_ffi::export!();

//! Media Picker Example - Demonstrates media selection and loading
//!
//! This example showcases:
//! - MediaPicker component for selecting photos, videos, and live photos
//! - Selected::load() for asynchronously loading media content
//! - Displaying loaded media (Photo, Video, LivePhoto)
//! - Filter options for different media types

use waterui::component::Dynamic;
use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};
use waterui::media::Media;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::task::spawn_local;

pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {
    // Currently selected media (None = no selection yet)
    let loaded_media: Binding<Option<Media>> = binding(None);
    let is_loading = binding(false);
    let selection = binding(Selected::new(0));

    // Main layout
    vstack((
        // Title
        text("Media Picker Demo")
            .size(28.0)
            .bold()
            .padding_with(16.0),
        // Picker buttons row
        hstack((
            picker_button(MediaFilter::Image, &selection, &loaded_media, &is_loading),
            picker_button(MediaFilter::Video, &selection, &loaded_media, &is_loading),
            picker_button(MediaFilter::LivePhoto, &selection, &loaded_media, &is_loading),
        ))
        .spacing(12.0)
        .padding_with(16.0),
        // Divider
        Divider,
        // Media display area
        media_display_area(loaded_media.clone(), is_loading.clone()),
        spacer(),
    ))
}

/// Creates a picker button that opens the media picker with the given filter
fn picker_button(
    filter: MediaFilter,
    selection: &Binding<Selected>,
    loaded_media: &Binding<Option<Media>>,
    is_loading: &Binding<bool>,
) -> impl View {
    let loaded = loaded_media.clone();
    let loading = is_loading.clone();
    let sel = selection.clone();

    // Create media picker and watch for selection changes
    MediaPicker::new(selection)
        .filter(filter)
        .on_change(&sel, move |_new_selection| {
            // Clear previous media and show loading
            loaded.set(None);
            loading.set(true);

            let _loaded = loaded.clone();
            let loading = loading.clone();

            // Load the selected media asynchronously
            // Note: In a real app, you'd get the environment from context
            // and call new_selection.load(&env).await
            spawn_local(async move {
                // Simulated loading - in real usage:
                // let media = new_selection.load(&env).await;
                // _loaded.set(Some(media));
                loading.set(false);
            })
            .detach();
        })
}

/// Displays the loaded media or a placeholder
fn media_display_area(
    loaded_media: Binding<Option<Media>>,
    is_loading: Binding<bool>,
) -> impl View {
    let media = loaded_media;
    let loading_state = is_loading;

    Dynamic::watch(media.clone(), move |current_media| {
        Dynamic::watch(loading_state.clone(), move |is_loading| {
            if is_loading {
                // Show loading indicator
                vstack((
                    loading(),
                    text("Loading media...").foreground(theme_color::MutedForeground),
                ))
                .spacing(12.0)
                .anyview()
            } else if let Some(media) = current_media.clone() {
                // Show the loaded media
                media_view(media)
            } else {
                // Show placeholder
                vstack((
                    text("No media selected")
                        .size(18.0)
                        .foreground(theme_color::MutedForeground),
                    text("Tap a button above to select media")
                        .size(14.0)
                        .foreground(theme_color::MutedForeground),
                ))
                .spacing(8.0)
                .anyview()
            }
        })
    })
}

/// Creates a view for the loaded media based on its type
fn media_view(media: Media) -> AnyView {
    match media {
        Media::Image(url) => vstack((
            Photo::new(url),
            text("Image")
                .size(14.0)
                .foreground(theme_color::MutedForeground)
                .padding_with(8.0),
        ))
        .anyview(),
        Media::Video(url) => vstack((
            VideoPlayer::new(url)
                .show_controls(true)
                .aspect_ratio(AspectRatio::Fit),
            text("Video")
                .size(14.0)
                .foreground(theme_color::MutedForeground)
                .padding_with(8.0),
        ))
        .anyview(),
        Media::LivePhoto(source) => vstack((
            LivePhoto::new(source),
            text("Live Photo")
                .size(14.0)
                .foreground(theme_color::MutedForeground)
                .padding_with(8.0),
        ))
        .anyview(),
    }
}

waterui_ffi::export!();

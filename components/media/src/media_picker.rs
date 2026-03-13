//! # Media Picker
//!
//! This module provides media selection functionality through `MediaPicker`.

use alloc::{string::ToString, vec::Vec};

use waterui_controls::{IntoLabel, button};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Binding, Computed, Environment, Signal, View, reactive::impl_constant};
use waterui_text::{Text, text};

#[cfg(feature = "std")]
use waterkit_dialog::{MediaType, PhotoPicker as KitPhotoPicker};

use crate::{Media, url::Url};

/// A media picker view that lets users select photos, videos, or live media.
///
/// `MediaPicker` renders as a button that, when clicked, presents the native
/// platform media picker via WaterKit dialog APIs.
#[derive(Debug)]
pub struct MediaPicker<Label> {
    selection: Binding<Option<Selected>>,
    filter: Computed<MediaFilter>,
    label: Label,
}

impl MediaPicker<Text> {
    /// Creates a new `MediaPicker` with a selection binding.
    #[must_use]
    pub fn new(selection: &Binding<Option<Selected>>) -> Self {
        Self {
            selection: selection.clone(),
            filter: MediaFilter::Image.into_computed(),
            label: text("Select Media"),
        }
    }
}

impl<Label> MediaPicker<Label>
where
    Label: IntoLabel + 'static,
{
    /// Sets the media filter for this picker.
    #[must_use]
    pub fn filter(mut self, filter: impl IntoComputed<MediaFilter>) -> Self {
        self.filter = filter.into_computed();
        self
    }

    /// Sets a custom label for the picker button.
    #[must_use]
    pub fn label<NewLabel: IntoLabel + 'static>(self, label: NewLabel) -> MediaPicker<NewLabel> {
        MediaPicker {
            selection: self.selection,
            filter: self.filter,
            label,
        }
    }
}

impl<Label> View for MediaPicker<Label>
where
    Label: IntoLabel + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let selection = self.selection.clone();
        let filter = self.filter.clone();

        button(self.label).action_async(move || {
            let selection = selection.clone();
            let filter = filter.clone();
            async move {
                #[cfg(feature = "std")]
                {
                    let requested_filter = filter.get();
                    let picker = KitPhotoPicker::new()
                        .with_media_type(media_type_from_filter(&requested_filter));
                    let handle = match picker.pick().await {
                        Ok(handle) => handle,
                        Err(error) => {
                            tracing::warn!("MediaPicker failed to present picker dialog: {error}");
                            return;
                        }
                    };

                    let Some(handle) = handle else {
                        return;
                    };

                    let path = match handle.load().await {
                        Ok(path) => path,
                        Err(error) => {
                            tracing::warn!("MediaPicker failed to load selected media: {error}");
                            return;
                        }
                    };

                    let media = media_from_loaded_path(&path, &requested_filter);
                    selection.set(Some(Selected { media }));
                }

                #[cfg(not(feature = "std"))]
                {
                    panic!("MediaPicker requires the `std` feature to use native picker dialogs");
                }
            }
        })
    }
}

#[cfg(feature = "std")]
fn media_type_from_filter(filter: &MediaFilter) -> MediaType {
    match filter {
        MediaFilter::Image => MediaType::Image,
        MediaFilter::Video => MediaType::Video,
        MediaFilter::LivePhoto => MediaType::LivePhoto,
        MediaFilter::All(filters) | MediaFilter::Any(filters) => {
            if filters.iter().any(requests_live_photo) {
                MediaType::LivePhoto
            } else if filters.iter().all(|f| matches!(f, MediaFilter::Video)) {
                MediaType::Video
            } else {
                MediaType::Image
            }
        }
        MediaFilter::Not(filters) => {
            if filters.iter().any(|f| matches!(f, MediaFilter::Image)) {
                MediaType::Video
            } else {
                MediaType::Image
            }
        }
    }
}

#[cfg(feature = "std")]
fn media_from_loaded_path(path: &std::path::Path, requested_filter: &MediaFilter) -> Media {
    let path_str = path.to_string_lossy().to_string();
    let url = Url::from_file_path_str(path_str);

    match requested_filter {
        MediaFilter::Image => Media::Image(url),
        MediaFilter::Video => Media::Video(url),
        MediaFilter::LivePhoto => {
            tracing::warn!(
                "MediaPicker selected LivePhoto via WaterKit dialog; current API returns a single asset, falling back to image/video inference"
            );
            infer_media_from_path(url, path)
        }
        MediaFilter::All(_) | MediaFilter::Any(_) | MediaFilter::Not(_) => {
            if requests_live_photo(requested_filter) {
                tracing::warn!(
                    "MediaPicker selected a LivePhoto-combinator filter; current API returns a single asset, falling back to image/video inference"
                );
            }
            infer_media_from_path(url, path)
        }
    }
}

#[cfg(feature = "std")]
fn requests_live_photo(filter: &MediaFilter) -> bool {
    match filter {
        MediaFilter::LivePhoto => true,
        MediaFilter::All(filters) | MediaFilter::Any(filters) => {
            filters.iter().any(requests_live_photo)
        }
        MediaFilter::Not(_) | MediaFilter::Image | MediaFilter::Video => false,
    }
}

#[cfg(feature = "std")]
fn infer_media_from_path(url: Url, path: &std::path::Path) -> Media {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());

    let is_video = ext.as_deref().is_some_and(|extension| {
        matches!(
            extension,
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "3gp" | "hevc"
        )
    });

    if is_video {
        Media::Video(url)
    } else {
        Media::Image(url)
    }
}

/// Represents a selected media item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    media: Media,
}

impl Selected {
    /// Load the selected media item asynchronously.
    #[must_use]
    pub async fn load(self) -> Media {
        self.media
    }

    /// Returns a reference to the loaded media payload.
    #[must_use]
    pub const fn media(&self) -> &Media {
        &self.media
    }
}

/// Represents filters that can be applied to media selection.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaFilter {
    /// Filter for live photos.
    LivePhoto,
    /// Filter for videos.
    Video,
    /// Filter for images.
    Image,
    /// Filter for all of the specified filters.
    All(Vec<Self>),
    /// Filter for none of the specified filters.
    Not(Vec<Self>),
    /// Filter for any of the specified filters.
    Any(Vec<Self>),
}

impl_constant!(MediaFilter);

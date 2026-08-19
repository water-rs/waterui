//! # Media Picker
//!
//! This module provides media selection functionality through `MediaPicker`.

use alloc::vec::Vec;

#[cfg(feature = "std")]
use alloc::string::ToString;
#[cfg(feature = "std")]
use std::path::Path;
use waterui_controls::{IntoLabel, button};
#[cfg(feature = "std")]
use waterui_core::Signal;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Binding, Computed, Environment, View, reactive::impl_constant};
use waterui_text::{Text, text};

#[cfg(feature = "std")]
use waterkit_dialog::{LoadedMedia, MediaType, PhotoPicker as KitPhotoPicker};

use crate::Media;
#[cfg(feature = "std")]
use crate::{live::LivePhotoSource, url::Url};

/// A media picker view that lets users select photos, videos, or live media.
///
/// `MediaPicker` renders as a button that, when clicked, presents the native
/// platform media picker via `WaterKit` dialog APIs.
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
        #[cfg(feature = "std")]
        let selection = self.selection.clone();
        #[cfg(feature = "std")]
        let filter = self.filter.clone();

        button(self.label).action_async(move || {
            #[cfg(feature = "std")]
            let selection = selection.clone();
            #[cfg(feature = "std")]
            let filter = filter.clone();
            async move {
                #[cfg(feature = "std")]
                {
                    let requested_filter = filter.get();
                    let picker = KitPhotoPicker::new()
                        .with_media_type(media_type_from_filter(requested_filter));
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

                    let loaded = match handle.load_media().await {
                        Ok(loaded) => loaded,
                        Err(error) => {
                            tracing::warn!("MediaPicker failed to load selected media: {error}");
                            return;
                        }
                    };

                    let media = media_from_loaded_selection(loaded);
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
const fn media_type_from_filter(filter: MediaFilter) -> MediaType {
    match filter {
        MediaFilter::Image => MediaType::Image,
        MediaFilter::Video => MediaType::Video,
        MediaFilter::LivePhoto => MediaType::LivePhoto,
    }
}

#[cfg(feature = "std")]
fn media_from_loaded_selection(loaded: LoadedMedia) -> Media {
    match loaded {
        LoadedMedia::Image(path) => {
            Media::Image(Url::from_file_path_str(path.to_string_lossy().to_string()))
        }
        LoadedMedia::Video(path) => {
            Media::Video(Url::from_file_path_str(path.to_string_lossy().to_string()))
        }
        LoadedMedia::LivePhoto(live_photo) => {
            let (image, video) = live_photo.into_parts();
            live_photo_from_paths(&image, &video)
        }
    }
}

#[cfg(feature = "std")]
fn live_photo_from_paths(image: &Path, video: &Path) -> Media {
    Media::LivePhoto(LivePhotoSource::new(
        Url::from_file_path_str(image.to_string_lossy().to_string()),
        Url::from_file_path_str(video.to_string_lossy().to_string()),
    ))
}


/// Represents a selected media item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    media: Media,
}

impl Selected {
    /// Load the selected media item asynchronously.
    #[must_use]
    pub fn load(self) -> Media {
        self.media
    }

    /// Returns a reference to the loaded media payload.
    #[must_use]
    pub const fn media(&self) -> &Media {
        &self.media
    }
}

/// The kind of media a [`MediaPicker`] offers for selection.
///
/// Platform pickers accept exactly one media kind per presentation, so the
/// filter is deliberately a plain choice rather than a boolean algebra: every
/// value here is honored exactly by the native picker, never approximated.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaFilter {
    /// Filter for live photos.
    LivePhoto,
    /// Filter for videos.
    Video,
    /// Filter for images.
    Image,
}

impl_constant!(MediaFilter);

#[cfg(all(test, feature = "std"))]
mod tests {
    use std::path::PathBuf;

    use waterkit_dialog::MediaType;

    use super::{MediaFilter, live_photo_from_paths, media_type_from_filter};
    use crate::{Media, Url, live::LivePhotoSource};

    #[test]
    fn live_photo_selection_preserves_paired_resources() {
        let image = PathBuf::from("waterui-live-photo.heic");
        let video = PathBuf::from("waterui-live-photo.mov");

        assert_eq!(
            live_photo_from_paths(&image, &video),
            Media::LivePhoto(LivePhotoSource::new(
                Url::from_file_path_str(image.to_string_lossy().to_string()),
                Url::from_file_path_str(video.to_string_lossy().to_string()),
            ))
        );
    }

    #[test]
    fn every_filter_maps_to_its_exact_platform_media_type() {
        assert_eq!(
            media_type_from_filter(MediaFilter::LivePhoto),
            MediaType::LivePhoto
        );
        assert_eq!(media_type_from_filter(MediaFilter::Image), MediaType::Image);
        assert_eq!(media_type_from_filter(MediaFilter::Video), MediaType::Video);
    }
}

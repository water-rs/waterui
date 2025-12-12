//! # Media Picker
//!
//! This module provides media selection functionality through `MediaPicker`.
//!
//! ## Platform Support
//!
//! The `MediaPicker` is available on iOS, macOS, and Android platforms.

use std::fmt::Debug;

use alloc::rc::Rc;

use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Binding, Computed, Environment, configurable, reactive::impl_constant};

use crate::Media;

/// Configuration for the `MediaPicker` component.
#[derive(Debug)]
pub struct MediaPickerConfig {
    /// The current selection binding (native writes to this when user picks).
    pub selection: Binding<Selected>,
    /// A filter to apply to media selection.
    pub filter: Computed<MediaFilter>,
}
type MediaPickerCallback = Box<dyn FnOnce(Media)>;

/// A media loader function type.
///
/// Should be registered to environment to handle loading media based on selection.
#[derive(Clone)]
pub struct MediaLoader(Rc<dyn Fn(Selected, MediaPickerCallback)>);

impl Debug for MediaLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaLoader").finish()
    }
}

impl MediaLoader {
    /// Creates a new MediaLoader with the given function.
    pub fn new<F>(f: F) -> Self
    where
        F: 'static + Fn(Selected, MediaPickerCallback),
    {
        Self(Rc::new(f))
    }

    /// Calls the media loader function.
    pub fn load(&self, selection: Selected, callback: MediaPickerCallback) {
        (self.0)(selection, callback);
    }
}

configurable!(
    #[doc = "A media picker view that lets users select photos, videos, or live media."]
    MediaPicker,
    MediaPickerConfig
);

impl MediaPicker {
    /// Creates a new MediaPicker with a selection binding.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let selection = binding(Selected::new(0));
    /// let picker = MediaPicker::new(&selection);
    /// ```
    pub fn new(selection: &Binding<Selected>) -> Self {
        Self(MediaPickerConfig {
            selection: selection.clone(),
            filter: MediaFilter::Image.into_computed(),
        })
    }

    /// Sets the media filter for this picker.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// MediaPicker::new(&selection).filter(MediaFilter::Video);
    /// ```
    #[must_use]
    pub fn filter(mut self, filter: impl IntoComputed<MediaFilter>) -> Self {
        self.0.filter = filter.into_computed();
        self
    }
}

/// Represents a selected media item by its unique identifier.
///
/// This handle is returned by `MediaPicker` when a user selects media.
/// Use [`Selected::load()`] to asynchronously retrieve the actual media content.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Selected(pub u32);

impl Selected {
    /// Creates a new Selected with the given ID.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns the inner ID value.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.0
    }

    /// Load the selected media item asynchronously.
    ///
    /// This method retrieves the actual media content from the platform's media library
    /// based on the selection ID obtained from `MediaPicker`.
    ///
    /// # Platform Notes
    ///
    /// - **iOS/macOS**: Uses `PHImageManager` to load the photo/video data
    /// - **Android**: Uses `ContentResolver` to load from the content URI
    ///
    /// # Returns
    ///
    /// Returns the loaded [`Media`] item (Image, Video, or `LivePhoto`).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let selection = binding(Selected::new(0));
    /// let picker = MediaPicker::new().selection(selection.clone());
    ///
    /// // After user selects media...
    /// let media = selection.get().load().await;
    /// ```
    pub async fn load(self, env: &Environment) -> Media {
        let loader = env
            .get::<MediaLoader>()
            .expect("MediaLoader not found in environment");
        let (mut sender, receiver) = async_oneshot::oneshot();
        loader.load(
            self,
            Box::new(move |media| {
                let _ = sender.send(media);
            }),
        );
        receiver.await.expect("Failed to receive media")
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

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
use waterui_core::{
    AnyView, Binding, Computed, Environment, Signal, View, reactive::impl_constant,
};

use crate::Media;

/// Manager for presenting media picker and loading selected media.
/// Installed by native backends via FFI.
///
/// This trait should be implemented by platform-specific backends to provide
/// native media picker functionality.
pub trait CustomMediaPickerManager: 'static {
    /// Present the native media picker modal with the given filter.
    /// Returns the selected media ID via callback when user picks media.
    fn present(&self, filter: MediaFilter, callback: impl FnOnce(Selected) + 'static);

    /// Load media content for the given selection ID.
    /// Returns the loaded Media via callback.
    fn load(&self, selected: Selected, callback: impl FnOnce(Media) + 'static);
}

/// Type-erased `MediaPickerManager` stored in Environment.
#[derive(Clone)]
pub struct MediaPickerManager(Rc<dyn MediaPickerManagerImpl>);

trait MediaPickerManagerImpl: 'static {
    fn present(&self, filter: MediaFilter, callback: Box<dyn FnOnce(Selected)>);
    fn load(&self, selected: Selected, callback: Box<dyn FnOnce(Media)>);
}

impl<T: CustomMediaPickerManager> MediaPickerManagerImpl for T {
    fn present(&self, filter: MediaFilter, callback: Box<dyn FnOnce(Selected)>) {
        CustomMediaPickerManager::present(self, filter, callback);
    }

    fn load(&self, selected: Selected, callback: Box<dyn FnOnce(Media)>) {
        CustomMediaPickerManager::load(self, selected, callback);
    }
}

impl MediaPickerManager {
    /// Creates a new `MediaPickerManager` from any type implementing `CustomMediaPickerManager`.
    pub fn new<T: CustomMediaPickerManager>(manager: T) -> Self {
        Self(Rc::new(manager))
    }
}

impl Debug for MediaPickerManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPickerManager").finish()
    }
}

/// A media picker view that lets users select photos, videos, or live media.
///
/// `MediaPicker` renders as a button that, when clicked, presents the native
/// platform media picker. The selected media ID is written to the provided binding.
#[derive(Debug)]
pub struct MediaPicker {
    selection: Binding<Selected>,
    filter: Computed<MediaFilter>,
    label: AnyView,
}

impl MediaPicker {
    /// Creates a new `MediaPicker` with a selection binding.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let selection = binding(Selected::new(0));
    /// let picker = MediaPicker::new(&selection);
    /// ```
    #[must_use] 
    pub fn new(selection: &Binding<Selected>) -> Self {
        Self {
            selection: selection.clone(),
            filter: MediaFilter::Image.into_computed(),
            label: AnyView::new(waterui_text::text("Select Media")),
        }
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
        self.filter = filter.into_computed();
        self
    }

    /// Sets a custom label for the picker button.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// MediaPicker::new(&selection).label(text("Choose Photo"));
    /// ```
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.label = AnyView::new(label);
        self
    }
}

impl View for MediaPicker {
    fn body(self, env: &Environment) -> impl View {
        use waterui_controls::button;

        let selection = self.selection.clone();
        let filter = self.filter.clone();

        // Get manager from environment during view construction
        let manager = env
            .get::<MediaPickerManager>()
            .expect("MediaPickerManager not installed in environment")
            .clone();

        button(self.label).action(move || {
            let sel = selection.clone();
            manager.0.present(
                filter.get(),
                Box::new(move |selected| {
                    sel.set(selected);
                }),
            );
        })
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
    /// let picker = MediaPicker::new(&selection);
    ///
    /// // After user selects media...
    /// let media = selection.get().load(&env).await;
    /// ```
    pub async fn load(self, env: &Environment) -> Media {
        let manager = env
            .get::<MediaPickerManager>()
            .expect("MediaPickerManager not found in environment");
        let (mut sender, receiver) = async_oneshot::oneshot();
        manager.0.load(
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

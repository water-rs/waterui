//! # Media Picker
//!
//! This module provides media selection functionality through `MediaPicker`.
//!
//! ## Platform Support
//!
//! The `MediaPicker` is available on iOS, macOS, and Android platforms.

use std::fmt::Debug;

use alloc::rc::Rc;

use waterui_core::extract::Use;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Binding, Computed, Environment, Signal, View, reactive::impl_constant};
use waterui_text::{Text, text};

use crate::Media;

/// Manager for presenting media picker and loading selected media.
/// Installed by native backends via FFI.
///
/// This trait should be implemented by platform-specific backends to provide
/// native media picker functionality.
pub trait CustomMediaPickerManager: 'static {
    /// Present the native media picker modal with the given filter.
    /// Returns the selected media ID via callback when user picks media.
    fn present(&self, filter: MediaFilter, callback: impl FnOnce(SelectedId) + 'static);

    /// Load media content for the given selection ID.
    /// Returns the loaded Media via callback.
    fn load(&self, selected: SelectedId, callback: impl FnOnce(Media) + 'static);
}

/// Type-erased `MediaPickerManager` stored in Environment.
#[derive(Clone)]
pub struct MediaPickerManager(Rc<dyn MediaPickerManagerImpl>);

trait MediaPickerManagerImpl: 'static {
    fn present(&self, filter: MediaFilter, callback: Box<dyn FnOnce(SelectedId)>);
    fn load(&self, selected: SelectedId, callback: Box<dyn FnOnce(Media)>);
}

impl<T: CustomMediaPickerManager> MediaPickerManagerImpl for T {
    fn present(&self, filter: MediaFilter, callback: Box<dyn FnOnce(SelectedId)>) {
        CustomMediaPickerManager::present(self, filter, callback);
    }

    fn load(&self, selected: SelectedId, callback: Box<dyn FnOnce(Media)>) {
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
pub struct MediaPicker<Label> {
    selection: Binding<Option<Selected>>,
    filter: Computed<MediaFilter>,
    label: Label,
}

impl MediaPicker<Text> {
    /// Creates a new `MediaPicker` with a selection binding.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let selection = binding(Selected::new(0));
    /// let picker = MediaPicker::new(&selection);
    /// ```
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
    Label: View,
{
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
    pub fn label<NewLabel: View>(self, label: NewLabel) -> MediaPicker<NewLabel> {
        MediaPicker {
            selection: self.selection,
            filter: self.filter,
            label,
        }
    }
}

/// Unique identifier for selected media items.
pub type SelectedId = u32;

impl MediaPickerManager {
    /// Load media content for the given selection ID.
    pub fn load(&self, selected: SelectedId, callback: impl FnOnce(Media) + 'static) {
        self.0.load(selected, Box::new(callback));
    }

    /// Present the media picker with the specified filter.
    pub fn present(&self, filter: MediaFilter, callback: impl FnOnce(SelectedId) + 'static) {
        self.0.present(filter, Box::new(callback));
    }
}

impl<Label> View for MediaPicker<Label>
where
    Label: View,
{
    fn body(self, _env: &Environment) -> impl View {
        use waterui_controls::button;

        let selection = self.selection.clone();
        let filter = self.filter.clone();

        button(self.label).action(move |manager: Use<MediaPickerManager>| {
            let sel = selection.clone();
            manager.present(filter.get(), {
                let manager = manager.0.clone();
                Box::new(move |selected| {
                    sel.set(Some(Selected {
                        id: selected,
                        manager: manager.clone(),
                    }));
                })
            });
        })
    }
}

/// Represents a selected media item by its unique identifier.
#[derive(Debug, Clone)]
pub struct Selected {
    id: u32,
    manager: MediaPickerManager,
}

impl PartialEq for Selected {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl PartialOrd for Selected {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.id.partial_cmp(&other.id)
    }
}

impl Selected {
    /// Creates a new `Selected` with the given ID and no manager.
    ///
    /// This is typically used to create an initial/empty selection state.
    /// The manager will be populated when the user picks media via `MediaPicker`.
    #[allow(dead_code)]
    const fn new(id: u32, manager: MediaPickerManager) -> Self {
        Self { id, manager }
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
    /// # Panics
    ///
    /// Panics if the media loading operation fails or if the receiver channel is closed.
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
    pub async fn load(self) -> Media {
        let (mut sender, receiver) = async_oneshot::oneshot();
        self.manager.load(
            self.id,
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

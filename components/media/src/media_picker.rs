//! # Media Picker
//!
//! This module provides media selection functionality through `MediaPicker`.
//!
//! ## Platform Support
//!
//! The `MediaPicker` is available on iOS, macOS, and Android platforms.

use async_oneshot::oneshot;
use waterui_core::{Computed, configurable};

use crate::Media;

/// Configuration for the `MediaPicker` component.
#[derive(Debug)]
pub struct MediaPickerConfig {
    /// The items selected in the picker.
    pub selection: Computed<Selected>,
    /// A filter to apply to media selection.
    pub filter: Computed<MediaFilter>,
}

configurable!(
    #[doc = "A media picker view that lets users select photos, videos, or live media."]
    MediaPicker,
    MediaPickerConfig
);

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
    /// Returns the loaded [`Media`] item (Image, Video, or LivePhoto).
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
    pub async fn load(self) -> Media {
        let (mut sender, receiver) = oneshot::<Media>();
        let id = self.0;

        // Create the callback that native will call when media is loaded
        let callback = MediaLoadCallback::new(move |result: MediaLoadResult| {
            let media = result.into_media();
            // Ignore error if receiver was dropped
            let _ = sender.send(media);
        });

        // Call native to load the media
        unsafe {
            waterui_load_media(id, callback);
        }

        // Wait for result
        receiver.await.unwrap_or_else(|_| {
            // Channel closed without sending - return placeholder
            Media::Image(crate::Url::from(alloc::format!("media://cancelled/{id}")))
        })
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

// ============================================================================
// Media Loading FFI Infrastructure
// ============================================================================

/// Result of loading media from native platform.
///
/// For Live Photos / Motion Photos, both `url_ptr` (image) and `video_url_ptr` (video)
/// are populated. For regular images/videos, only `url_ptr` is used.
#[repr(C)]
pub struct MediaLoadResult {
    /// Pointer to UTF-8 encoded URL string (image URL for Live Photos).
    pub url_ptr: *const u8,
    /// Length of the URL string in bytes.
    pub url_len: usize,
    /// Pointer to UTF-8 encoded video URL (only for Live Photos).
    pub video_url_ptr: *const u8,
    /// Length of the video URL string in bytes.
    pub video_url_len: usize,
    /// Media type: 0 = Image, 1 = Video, 2 = LivePhoto.
    pub media_type: u8,
}

impl MediaLoadResult {
    /// Convert this FFI result into a Rust Media type.
    ///
    /// # Safety
    ///
    /// The `url_ptr` must point to valid UTF-8 bytes of length `url_len`.
    /// For Live Photos, `video_url_ptr` must also point to valid UTF-8 bytes.
    ///
    /// # Panics
    ///
    /// Panics if the URL data is not valid UTF-8 or cannot be parsed.
    fn into_media(self) -> Media {
        assert!(!self.url_ptr.is_null(), "MediaLoadResult: url_ptr is null");
        assert!(self.url_len > 0, "MediaLoadResult: url_len is 0");

        let url_str = unsafe {
            let url_bytes = core::slice::from_raw_parts(self.url_ptr, self.url_len);
            core::str::from_utf8(url_bytes).expect("MediaLoadResult: url is not valid UTF-8")
        };

        let url: crate::Url = url_str
            .parse()
            .expect("MediaLoadResult: failed to parse url");

        match self.media_type {
            0 => Media::Image(url),
            1 => Media::Video(url),
            2 => {
                // Live Photo: extract video URL
                assert!(
                    !self.video_url_ptr.is_null(),
                    "MediaLoadResult: video_url_ptr is null for LivePhoto"
                );
                assert!(
                    self.video_url_len > 0,
                    "MediaLoadResult: video_url_len is 0 for LivePhoto"
                );

                let video_url_str = unsafe {
                    let video_bytes =
                        core::slice::from_raw_parts(self.video_url_ptr, self.video_url_len);
                    core::str::from_utf8(video_bytes)
                        .expect("MediaLoadResult: video_url is not valid UTF-8")
                };

                let video_url: crate::Url = video_url_str
                    .parse()
                    .expect("MediaLoadResult: failed to parse video_url");

                Media::LivePhoto(crate::live::LivePhotoSource::new(url, video_url))
            }
            _ => panic!("MediaLoadResult: unknown media_type {}", self.media_type),
        }
    }
}

/// A callback for receiving loaded media from native code.
///
/// This is a C-compatible closure that native code calls with the result.
#[repr(C)]
pub struct MediaLoadCallback {
    /// Opaque pointer to the callback data.
    pub data: *mut (),
    /// Function to call with the result. This consumes the callback.
    pub call: unsafe extern "C" fn(*mut (), MediaLoadResult),
}

impl MediaLoadCallback {
    /// Creates a new callback from a Rust closure.
    pub fn new<F>(f: F) -> Self
    where
        F: FnOnce(MediaLoadResult) + 'static,
    {
        unsafe extern "C" fn call_impl<F2>(data: *mut (), result: MediaLoadResult)
        where
            F2: FnOnce(MediaLoadResult),
        {
            unsafe {
                let f = alloc::boxed::Box::from_raw(data as *mut F2);
                f(result);
            }
        }

        let data = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(f)) as *mut ();
        Self {
            data,
            call: call_impl::<F>,
        }
    }
}

unsafe impl Send for MediaLoadCallback {}

unsafe extern "C" {
    /// Native function to load media by selection ID.
    ///
    /// Native platforms must implement this function. When the media is loaded,
    /// native code must call the callback with the result.
    ///
    /// # Parameters
    ///
    /// - `id`: The selection ID from `MediaPicker`
    /// - `callback`: Callback to invoke when media is loaded
    ///
    /// # Implementation Notes
    ///
    /// - **iOS/macOS**: Use `PHImageManager` to load from Photos library
    /// - **Android**: Use `ContentResolver` to load from content URI
    fn waterui_load_media(id: u32, callback: MediaLoadCallback);
}

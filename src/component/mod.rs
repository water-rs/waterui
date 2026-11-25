//! UI components for `WaterUI`
//!
//! This module contains various UI components that can be used to build user interfaces.

#[doc(inline)]
pub use waterui_controls::*;

pub mod badge;
pub mod focus;
pub mod lazy;
#[cfg(feature = "link")]
pub mod link;
#[cfg(feature = "list")]
pub mod list;

#[cfg(any(feature = "list", feature = "table"))]
pub(crate) mod selection_support;

pub mod progress;
#[doc(inline)]
pub use progress::{Progress, ProgressStyle, loading, progress};

#[cfg(feature = "table")]
pub mod table;

#[doc(inline)]
pub use waterui_core::{AnyView, Dynamic, anyview, dynamic};

#[cfg(feature = "link")]
#[doc(inline)]
pub use link::{Link, link};
#[cfg(feature = "image")]
#[doc(inline)]
pub use media::{Image, MediaPicker, Photo, PhotoEvent, PickerStatus, photo};
#[cfg(all(feature = "image", feature = "video"))]
#[doc(inline)]
pub use media::{LivePhoto, LivePhotoEvent};
pub use text::{Text, text::text};
#[cfg(feature = "form")]
#[doc(inline)]
pub use waterui_form as form;
#[doc(inline)]
pub use waterui_layout::{
    scroll::{self, ScrollController, ScrollView, scroll},
    spacer::{self, Spacer, spacer, spacer_min},
    stack::{self, HStack, VStack, ZStack, hstack, vstack, zstack},
};
#[doc(inline)]
#[cfg(feature = "image")]
pub use waterui_media as media;
#[doc(inline)]
pub use waterui_text as text;
#[cfg(feature = "video")]
#[doc(inline)]
pub use waterui_video::{Video, VideoEvent, VideoPlayer, video_player};

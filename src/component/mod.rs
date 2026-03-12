//! UI components for `WaterUI`
//!
//! This module contains various UI components that can be used to build user interfaces.

#[doc(inline)]
pub use waterui_controls::*;

pub mod badge;
pub mod focus;
pub mod label;
pub mod lazy;
pub mod link;
pub mod list;

#[doc(inline)]
pub use label::{Label, label};

pub mod progress;
#[doc(inline)]
pub use progress::{Progress, loading, progress};

pub mod table;

#[doc(inline)]
pub use waterui_core::{AnyView, Dynamic, anyview, dynamic};

#[cfg(feature = "media")]
pub use media::*;
pub use text::{Text, text::text};
//pub use waterui_canvas as canvas;
#[doc(inline)]
pub use link::{Link, link};
#[doc(inline)]
pub use waterui_form as form;
#[doc(inline)]
pub use waterui_layout::{
    scroll::{self, ScrollView, scroll},
    spacer::{self, Spacer, spacer, spacer_min},
    stack::{self, HStack, VStack, ZStack, hstack, vstack, zstack},
};
#[doc(inline)]
#[cfg(feature = "media")]
pub use waterui_media as media;
#[doc(inline)]
pub use waterui_text as text;

//! UI components for `WaterUI`
//!
//! This module contains various UI components that can be used to build user interfaces.

pub mod button;
#[doc(inline)]
pub use button::{Button, button};

pub mod badge;
pub mod focu;
pub mod lazy;
pub mod list;

pub mod progress;
#[doc(inline)]
pub use progress::{Progress, loading, progress};

pub mod style;
pub mod table;

#[doc(inline)]
pub use waterui_core::components::*;

pub use media::*;
pub use text::{Text, text};
//pub use waterui_canvas as canvas;
#[doc(inline)]
pub use waterui_form as form;
#[doc(inline)]
pub use waterui_layout as layout;
#[doc(inline)]
pub use waterui_media as media;
#[doc(inline)]
pub use waterui_text as text;
#[doc(inline)]
pub use waterui_text::link;

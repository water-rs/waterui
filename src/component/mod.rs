//! UI components for WaterUI
//!
//! This module contains various UI components that can be used to build user interfaces.

pub mod button;
#[doc(inline)]
pub use button::{Button, button};

pub mod badge;
pub mod divder;
pub mod focu;

//pub mod list;

pub mod progress;
//pub mod views;
#[doc(inline)]
pub use progress::{Progress, loading, progress};

pub mod style;
//pub mod table;

#[doc(inline)]
pub use waterui_core::components::*;

pub use text::{Text, text};
pub use waterui_text as text;

pub use media::*;
pub use waterui_media as media;

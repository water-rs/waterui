//! `WaterUI` Controls Components
//! This crate provides a set of common UI controls for building user interfaces with `WaterUI`.
//!

#![no_std]
extern crate alloc;

pub mod menu;
pub use menu::{Menu, MenuItem};
pub mod slider;

pub use slider::Slider;
pub mod text_field;
pub use text_field::{TextField, field};
pub mod toggle;
pub use toggle::{Toggle, ToggleStyle, toggle};

pub mod stepper;
pub use stepper::{Stepper, stepper};

/// Button component and related utilities.
pub mod button;
pub use button::{Button, ButtonStyle, button};
/// Text editor component.
pub mod text_editor;
pub use text_editor::{RichTextEditor, RichTextField};

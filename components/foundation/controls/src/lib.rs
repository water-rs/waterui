//! `WaterUI` Controls Components
//! This crate provides a set of common UI controls for building user interfaces with `WaterUI`.
//!

#![no_std]
extern crate alloc;

#[cfg(test)]
pub(crate) fn init_test_executor() {
    waterui_testing::install_test_executor();
}

pub mod label;
pub use label::{IconPosition, IntoLabel, Label, LabelDisplayMode, label};
pub mod menu;
pub use menu::{
    Command, CommandExt, Menu, MenuBarView, MenuItem, MenuView, Shortcut, ShortcutModifiers,
};
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

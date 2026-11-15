//! `WaterUI` Controls Components
//! This crate provides a set of common UI controls for building user interfaces with `WaterUI`.
//!

#![no_std]
extern crate alloc;

#[cfg(test)]
pub(crate) fn init_test_executor() {
    waterui_testing::install_test_executor();
}

pub mod input;
pub mod label;
pub use label::{IconPosition, IntoLabel, Label, LabelDisplayMode, label};
#[cfg(feature = "menu")]
pub mod menu;
#[cfg(feature = "menu")]
pub use menu::{
    Command, CommandExt, CommandRole, Menu, MenuBarView, MenuItem, MenuView, NamedKey, Shortcut,
    ShortcutKey, ShortcutModifiers,
};
pub mod size;
pub use size::ControlSize;
#[cfg(feature = "slider")]
pub mod slider;

#[cfg(feature = "slider")]
pub use slider::{Slider, slider};
#[cfg(feature = "text-field")]
pub mod text_field;
#[cfg(feature = "text-field")]
pub use text_field::{TextField, field};
pub mod text_selection;
pub use text_selection::{CustomTextSelection, TextSelection};
#[cfg(feature = "toggle")]
pub mod toggle;
#[cfg(feature = "toggle")]
pub use toggle::{Toggle, ToggleStyle, toggle};

#[cfg(feature = "stepper")]
pub mod stepper;
#[cfg(feature = "stepper")]
pub use stepper::{Stepper, stepper};

/// Button component and related utilities.
pub mod button;
pub use button::{Button, ButtonStyle, button};

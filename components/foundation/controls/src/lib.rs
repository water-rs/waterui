//! `WaterUI` Controls Components
//! This crate provides a set of common UI controls for building user interfaces with `WaterUI`.
//!

#![no_std]
extern crate alloc;

#[cfg(test)]
pub(crate) fn init_test_executor() {
    waterui_testing::install_test_executor();
}

pub mod badge;
pub use badge::BadgeValue;
pub mod input;
pub mod label;
pub use label::{
    Built, ControlLabel, CustomLabel, ErasedLabel, IconLabel, IconPosition, IntoLabel, Label,
    LabelDisplayMode, LabelKind, SemanticLabel, TitleLabel, label,
};
pub mod slot;
pub use slot::{IconSlot, NoopSlot, SlotBuilder, StyleSlot};
#[cfg(feature = "menu")]
pub mod menu;
#[cfg(feature = "menu")]
pub use menu::{
    Command, CommandExt, Menu, MenuBarView, MenuItem, MenuView, NamedKey, Shortcut, ShortcutKey,
    ShortcutModifiers, ShortcutNamedKey,
};
pub mod size;
pub use size::ControlSize;
#[cfg(feature = "slider")]
pub mod slider;

#[cfg(feature = "slider")]
pub use slider::{Slider, SliderValue, slider};
#[cfg(feature = "text-field")]
pub mod text_field;
#[cfg(feature = "text-field")]
pub use text_field::{TextField, TextFieldValue, field};
pub mod text_selection;
pub use text_selection::{CustomTextSelection, TextSelection};
#[cfg(feature = "toggle")]
pub mod toggle;
#[cfg(feature = "toggle")]
pub use toggle::{
    CustomToggleStyle, IntoToggleStyle, Toggle, ToggleLabel, ToggleStyle, ToggleStyleConfiguration,
    toggle,
};

#[cfg(feature = "stepper")]
pub mod stepper;
#[cfg(feature = "stepper")]
pub use stepper::{Stepper, StepperValue, stepper};

/// Button component and related utilities.
pub mod button;
pub use button::{
    Button, ButtonAction, ButtonLabel, ButtonRole, ButtonStyle, ButtonStyleConfiguration,
    ButtonTrigger, CustomButtonStyle, IntoButtonStyle, PrimitiveButtonStyle,
    PrimitiveButtonStyleConfiguration, button,
};

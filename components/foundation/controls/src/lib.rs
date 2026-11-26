//! `WaterUI` Controls Components
//! This crate provides a set of common UI controls for building user interfaces with `WaterUI`.
//!

#![no_std]
extern crate alloc;

#[cfg(test)]
pub(crate) fn init_test_executor() {
    waterui_testing::install_test_executor();
}

pub mod anchored_overlay;
#[cfg(feature = "avatar")]
pub mod avatar;
pub mod badge;
pub use badge::{Badge, BadgeConfig, BadgeValue};
pub mod card;
pub use card::{Card, CardConfig, CardStyle, card};
#[cfg(feature = "menu")]
pub mod context_menu;
#[cfg(feature = "dialog")]
pub mod dialog;
pub mod disclosure;
pub use disclosure::{Disclosure, DisclosureConfig, disclosure};
pub mod divider;
pub use divider::Divider;
pub mod input;
pub mod label;
pub use label::{
    Built, ControlLabel, CustomLabel, ErasedLabel, IconLabel, IconPosition, IntoLabel, Label,
    LabelDisplayMode, LabelKind, SemanticLabel, TitleLabel, label,
};
#[cfg(feature = "link")]
pub mod link;
#[cfg(feature = "list")]
pub mod list;
pub mod progress;
pub use progress::{Progress, ProgressConfig, ProgressStyle, ProgressWithTotal, loading, progress};
pub mod secure;
#[cfg(feature = "list")]
mod selection_support;
pub mod sheet;
pub mod slot;
pub use slot::{IconSlot, NoopSlot, SlotBuilder};
#[cfg(feature = "menu")]
pub mod menu;
#[cfg(feature = "snackbar")]
pub mod snackbar;
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
#[cfg(feature = "table")]
pub mod table;
#[cfg(feature = "text-field")]
pub mod text_field;
#[cfg(feature = "text-field")]
pub use text_field::{TextField, TextFieldValue, field};
pub mod text_selection;
pub use text_selection::{CustomTextSelection, TextSelection};
pub mod theme;
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

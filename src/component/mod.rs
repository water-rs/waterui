//! UI components for `WaterUI`
//!
//! This module contains various UI components that can be used to build user interfaces.

#[doc(inline)]
pub use waterui_controls::{
    Badge, BadgeConfig, BadgeValue, Built, Button, ButtonAction, ButtonLabel, ButtonRole,
    ButtonStyle, ButtonStyleConfiguration, ButtonTrigger, ControlLabel, ControlSize,
    CustomButtonStyle, CustomLabel, CustomTextSelection, Disclosure, DisclosureConfig, ErasedLabel,
    IconLabel, IconPosition, IconSlot, IntoButtonStyle, IntoLabel, Label, LabelDisplayMode,
    LabelKind, NoopSlot, PrimitiveButtonStyle, PrimitiveButtonStyleConfiguration, Progress,
    ProgressConfig, ProgressStyle, ProgressWithTotal, SemanticLabel, Shortcut, ShortcutKey,
    ShortcutModifiers, ShortcutNamedKey, SlotBuilder, TextSelection, TitleLabel, button,
    disclosure, input, label, loading, shortcut, size, slot, text_selection,
};
#[cfg(feature = "menu")]
#[doc(inline)]
pub use waterui_controls::{
    Command, CommandExt, Menu, MenuBarView, MenuItem, MenuView, NamedKey, menu,
};
#[cfg(feature = "toggle")]
#[doc(inline)]
pub use waterui_controls::{
    CustomToggleStyle, IntoToggleStyle, Toggle, ToggleLabel, ToggleStyle, ToggleStyleConfiguration,
    toggle,
};
#[cfg(feature = "slider")]
#[doc(inline)]
pub use waterui_controls::{Slider, SliderValue, slider};
#[cfg(feature = "stepper")]
#[doc(inline)]
pub use waterui_controls::{Stepper, StepperValue, stepper};
#[cfg(feature = "text-field")]
#[doc(inline)]
pub use waterui_controls::{TextField, TextFieldValue, field, text_field};

pub mod badge;
pub mod focus;
#[cfg(feature = "link")]
#[doc(inline)]
pub use waterui_controls::link;
#[cfg(feature = "list")]
#[doc(inline)]
pub use waterui_controls::list;
#[cfg(feature = "table")]
#[doc(inline)]
pub use waterui_controls::table;

pub mod progress;
#[doc(inline)]
pub use progress::progress;

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

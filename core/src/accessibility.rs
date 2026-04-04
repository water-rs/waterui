//! Helpers for customizing accessibility metadata when the built-in
//! `WaterUI` defaults are not enough.
//!
//! `WaterUI` components ship with reasonable accessibility roles, labels, and
//! states by default. These types let you override the metadata when your
//! layout diverges from the default semantics (for example, when building a
//! composite widget or exposing platform-specific affordances). Prefer the
//! defaults whenever possible and use these helpers as the final step to ensure
//! assistive technologies convey the intended experience.

use nami::{Computed, impl_constant, signal::IntoComputed};
use waterui_str::Str;

use crate::metadata::MetadataKey;

/// Overrides the spoken label for a component when the default text is not
/// adequate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityLabel(Str);

impl MetadataKey for AccessibilityLabel {}

impl AccessibilityLabel {
    /// Creates a label announced by assistive technologies when the default
    /// `WaterUI` text would be misleading or absent.
    ///
    /// ```
    /// # use waterui_core::accessibility::AccessibilityLabel;
    /// let label = AccessibilityLabel::new("Delete draft");
    /// ```
    pub fn new(label: impl Into<Str>) -> Self {
        Self(label.into())
    }

    /// Returns the raw label string.
    #[must_use]
    pub const fn as_str(&self) -> &Str {
        &self.0
    }
}

/// Describes the semantic role of a component so assistive technology can
/// expose the right behavior and shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessibilityRole {
    Button,
    Link,
    Image,
    Text,
    Header,
    Footer,
    Navigation,
    Main,
    Search,
    Article,
    Section,
    List,
    ListItem,
    Checkbox,
    RadioButton,
    Switch,
    Slider,
    ProgressBar,
    Tab,
    TabList,
    TabPanel,
    Menu,
    MenuItem,
    MenuBar,
    MenuItemCheckbox,
    MenuItemRadio,
    Combobox,
    Option,
    Group,
}

impl MetadataKey for AccessibilityRole {}

/// Controls whether this view should participate in accessibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessibilityHidden(bool);

impl MetadataKey for AccessibilityHidden {}

impl AccessibilityHidden {
    /// Creates a hidden flag for accessibility.
    #[must_use]
    pub const fn new(hidden: bool) -> Self {
        Self(hidden)
    }

    /// Returns whether this view is hidden from assistive technologies.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.0
    }
}

/// Defines how this view should expose child semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum AccessibilityChildren {
    #[default]
    Automatic,
    ExcludeDescendants,
}

impl MetadataKey for AccessibilityChildren {}

impl AccessibilityChildren {
    /// Returns whether descendants should be excluded from accessibility output.
    #[must_use]
    pub const fn excludes_descendants(&self) -> bool {
        matches!(self, Self::ExcludeDescendants)
    }
}

/// Describes nuanced state transitions that assistive technologies use to keep
/// users in sync with complex widgets.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessibilityState {
    disabled: bool,
    selected: bool,
    checked: Option<bool>,
    expanded: Option<bool>,
    busy: bool,
    hidden: bool,
}

impl MetadataKey for AccessibilityState {}
impl_constant!(AccessibilityState);

impl AccessibilityState {
    /// Creates a default accessibility state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            disabled: false,
            selected: false,
            checked: None,
            expanded: None,
            busy: false,
            hidden: false,
        }
    }

    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    #[must_use]
    pub const fn checked(mut self, checked: Option<bool>) -> Self {
        self.checked = checked;
        self
    }

    #[must_use]
    pub const fn expanded(mut self, expanded: Option<bool>) -> Self {
        self.expanded = expanded;
        self
    }

    #[must_use]
    pub const fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    #[must_use]
    pub const fn is_selected(&self) -> bool {
        self.selected
    }

    #[must_use]
    pub const fn checked_state(&self) -> Option<bool> {
        self.checked
    }

    #[must_use]
    pub const fn expanded_state(&self) -> Option<bool> {
        self.expanded
    }

    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.busy
    }

    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }
}

/// Reactive accessibility state source for view modifiers that depend on signals.
#[derive(Debug, Clone)]
pub struct AccessibilityStateSignal(Computed<AccessibilityState>);

impl MetadataKey for AccessibilityStateSignal {}

impl AccessibilityStateSignal {
    /// Creates a new reactive accessibility state wrapper.
    #[must_use]
    pub fn new(state: impl IntoComputed<AccessibilityState>) -> Self {
        Self(state.into_computed())
    }

    /// Returns the computed accessibility state.
    #[must_use]
    pub fn state(&self) -> &Computed<AccessibilityState> {
        &self.0
    }
}

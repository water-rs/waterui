//! Helpers for customizing accessibility metadata when the built-in
//! `WaterUI` defaults are not enough.
//!
//! `WaterUI` components ship with reasonable accessibility roles, labels, and
//! states by default. These types let you override the metadata when your
//! layout diverges from the default semantics (for example, when building a
//! composite widget or exposing platform-specific affordances). Prefer the
//! defaults whenever possible and use these helpers as the final step to ensure
//! assistive technologies convey the intended experience.

use nami::{Computed, signal::IntoComputed};
use waterui_core::metadata::MetadataKey;
use waterui_str::Str;

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
    /// # use waterui::accessibility::AccessibilityLabel;
    /// let label = AccessibilityLabel::new("Delete draft");
    /// ```
    ///
    /// Pass short, action-oriented phrases that match what a user would read on
    /// screen. Reuse built-in labels when they already describe the control.
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
    /// Interactive control that triggers an immediate action.
    Button,
    /// Navigational link that moves focus to another view or page.
    Link,
    /// Standalone image, icon, or illustration.
    Image,
    /// Non-interactive block of textual content.
    Text,
    /// Heading that introduces the structure of surrounding content.
    Header,
    /// Content that provides complementary information near the bottom of a
    /// view.
    Footer,
    /// Main navigation landmark for switching sections or screens.
    Navigation,
    /// Primary content region of the current view.
    Main,
    /// Search region containing search inputs or results.
    Search,
    /// Article or long-form content with its own outline.
    Article,
    /// Section of related content within a larger structure.
    Section,
    /// Container for a vertical or horizontal list of items.
    List,
    /// Single entry within a list.
    ListItem,
    /// Checkbox that toggles between on/off or yes/no.
    Checkbox,
    /// Radio button that participates in a mutually-exclusive group.
    RadioButton,
    /// Switch control that represents a binary state.
    Switch,
    /// Range slider used for continuous or stepped values.
    Slider,
    /// Progress bar communicating task completion.
    ProgressBar,
    /// Individual tab that selects one panel at a time.
    Tab,
    /// List container holding interactive tabs.
    TabList,
    /// Panel displaying content associated with a tab.
    TabPanel,
    /// Menu container that groups menu items.
    Menu,
    /// Interactive command within a menu.
    MenuItem,
    /// Top-level menu bar containing multiple menus.
    MenuBar,
    /// Checkbox-like menu item for toggling options inside a menu.
    MenuItemCheckbox,
    /// Radio-button-like menu item for mutually exclusive menu choices.
    MenuItemRadio,
    /// Combo box presenting a text field with a list of options.
    Combobox,
    /// Individual option within a list or combo box.
    Option,
    /// Grouping container that provides context for nested items.
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
    /// Default backend behavior.
    #[default]
    Automatic,
    /// Keep this node semantic, but exclude semantics from descendants.
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
    /// Whether the control is disabled for interaction but remains visible.
    disabled: bool,
    /// Whether the control is the current selection within its group.
    selected: bool,
    /// Whether the control is checked, unchecked, or mixed.
    checked: Option<bool>,
    /// Whether the control's additional content is expanded or collapsed.
    expanded: Option<bool>,
    /// Whether the control represents a busy, loading, or indeterminate state.
    busy: bool,
    /// Whether the control should be hidden from assistive technologies.
    hidden: bool,
}

impl MetadataKey for AccessibilityState {}

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

    /// Sets disabled state.
    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sets selected state.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Sets checked state. `None` means not applicable.
    #[must_use]
    pub const fn checked(mut self, checked: Option<bool>) -> Self {
        self.checked = checked;
        self
    }

    /// Sets expanded/collapsed state. `None` means not applicable.
    #[must_use]
    pub const fn expanded(mut self, expanded: Option<bool>) -> Self {
        self.expanded = expanded;
        self
    }

    /// Sets busy state.
    #[must_use]
    pub const fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Sets hidden state.
    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Returns disabled state.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns selected state.
    #[must_use]
    pub const fn is_selected(&self) -> bool {
        self.selected
    }

    /// Returns checked state.
    #[must_use]
    pub const fn checked_state(&self) -> Option<bool> {
        self.checked
    }

    /// Returns expanded state.
    #[must_use]
    pub const fn expanded_state(&self) -> Option<bool> {
        self.expanded
    }

    /// Returns busy state.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.busy
    }

    /// Returns hidden state.
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

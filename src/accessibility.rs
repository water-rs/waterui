//! Helpers for customizing accessibility metadata when the built-in
//! `WaterUI` defaults are not enough.
//!
//! `WaterUI` components ship with reasonable accessibility roles, labels, and
//! states by default. These types let you override the metadata when your
//! layout diverges from the default semantics (for example, when building a
//! composite widget or exposing platform-specific affordances). Prefer the
//! defaults whenever possible and use these helpers as the final step to ensure
//! assistive technologies convey the intended experience.

use waterui_str::Str;

/// Overrides the spoken label for a component when the default text is not
/// adequate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityLabel(Str);

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

/// Describes nuanced state transitions that assistive technologies use to keep
/// users in sync with complex widgets.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
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

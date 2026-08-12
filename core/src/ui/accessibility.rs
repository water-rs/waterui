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
///
/// The label is reactive: a label derived from app state (`"3 unread
/// messages"`) stays current without rebuilding the subtree, matching
/// [`AccessibilityStateSignal`].
#[derive(Debug, Clone)]
pub struct AccessibilityLabel(Computed<Str>);

impl MetadataKey for AccessibilityLabel {}

impl AccessibilityLabel {
    /// Creates a label announced by assistive technologies when the default
    /// `WaterUI` text would be misleading or absent.
    ///
    /// Accepts a constant or any signal of [`Str`].
    ///
    /// ```
    /// # use waterui_core::accessibility::AccessibilityLabel;
    /// let label = AccessibilityLabel::new("Delete draft");
    /// ```
    pub fn new(label: impl IntoComputed<Str>) -> Self {
        Self(label.into_computed())
    }

    /// The reactive label signal.
    #[must_use]
    pub const fn signal(&self) -> &Computed<Str> {
        &self.0
    }
}

/// A stable, developer-facing identifier for locating this view in UI tests.
///
/// Identifiers are never exposed to end users or spoken by assistive
/// technologies — they exist purely for automation (`waterui-testing`
/// selectors, `XCUITest` `accessibilityIdentifier`, Android `UiAutomator` resource
/// matching). Keep them constant: a query key that changes with app state
/// defeats its purpose, so unlike [`AccessibilityLabel`] this is not a signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityIdentifier(Str);

impl MetadataKey for AccessibilityIdentifier {}

impl AccessibilityIdentifier {
    /// Creates a stable automation identifier.
    ///
    /// ```
    /// # use waterui_core::accessibility::AccessibilityIdentifier;
    /// let id = AccessibilityIdentifier::new("login.submit");
    /// ```
    pub fn new(identifier: impl Into<Str>) -> Self {
        Self(identifier.into())
    }

    /// The identifier string.
    #[must_use]
    pub const fn as_str(&self) -> &Str {
        &self.0
    }

    /// Consumes the metadata and returns the identifier string.
    #[must_use]
    pub fn into_str(self) -> Str {
        self.0
    }
}

/// Describes the semantic role of a component so assistive technology can
/// expose the right behavior and shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessibilityRole {
    /// Interactive control that triggers an action.
    Button,
    /// Interactive text or element that opens a destination.
    Link,
    /// Non-text visual content.
    Image,
    /// Plain readable text content.
    Text,
    /// Section heading.
    Header,
    /// Section footer.
    Footer,
    /// Landmark for primary site or app navigation.
    Navigation,
    /// Landmark for the main content region.
    Main,
    /// Landmark for search controls or results.
    Search,
    /// Self-contained article or post content.
    Article,
    /// Thematic content section.
    Section,
    /// Container for list items.
    List,
    /// Item within a list.
    ListItem,
    /// Toggleable checkbox control.
    Checkbox,
    /// Mutually exclusive radio button control.
    RadioButton,
    /// On/off switch control.
    Switch,
    /// Adjustable range control.
    Slider,
    /// Read-only progress indicator.
    ProgressBar,
    /// Individual tab selector.
    Tab,
    /// Container that owns a set of tabs.
    TabList,
    /// Content region paired with a tab.
    TabPanel,
    /// Popup or contextual menu.
    Menu,
    /// Action entry inside a menu.
    MenuItem,
    /// Horizontal menu bar container.
    MenuBar,
    /// Checkbox-style menu item.
    MenuItemCheckbox,
    /// Radio-style menu item.
    MenuItemRadio,
    /// Editable or pick-list combo box.
    Combobox,
    /// Selectable option within a list or combo box.
    Option,
    /// Logical grouping container.
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
    /// Let the backend choose the default child exposure behavior.
    Automatic,
    /// Hide descendants and expose only the parent node.
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

    /// Marks whether the element is currently disabled.
    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Marks whether the element is currently selected.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Sets the checkbox or tri-state checked value.
    #[must_use]
    pub const fn checked(mut self, checked: Option<bool>) -> Self {
        self.checked = checked;
        self
    }

    /// Sets the expanded or collapsed state when applicable.
    #[must_use]
    pub const fn expanded(mut self, expanded: Option<bool>) -> Self {
        self.expanded = expanded;
        self
    }

    /// Marks whether the element is busy processing work.
    #[must_use]
    pub const fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Marks whether the element should be hidden from accessibility output.
    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Returns whether the element is disabled.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns whether the element is selected.
    #[must_use]
    pub const fn is_selected(&self) -> bool {
        self.selected
    }

    /// Returns the checked state for checkbox-like roles.
    #[must_use]
    pub const fn checked_state(&self) -> Option<bool> {
        self.checked
    }

    /// Returns the expanded state for expandable roles.
    #[must_use]
    pub const fn expanded_state(&self) -> Option<bool> {
        self.expanded
    }

    /// Returns whether the element is marked busy.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.busy
    }

    /// Returns whether the element is hidden from accessibility output.
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
    pub const fn state(&self) -> &Computed<AccessibilityState> {
        &self.0
    }
}

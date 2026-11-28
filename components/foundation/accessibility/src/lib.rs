//! Helpers for customizing accessibility metadata when the built-in
//! `WaterUI` defaults are not enough.
//!
//! `WaterUI` components ship with reasonable accessibility roles, labels, and
//! states by default. These types let you override the metadata when your
//! layout diverges from the default semantics (for example, when building a
//! composite widget or exposing platform-specific affordances). Prefer the
//! defaults whenever possible and use these helpers as the final step to ensure
//! assistive technologies convey the intended experience.

#![no_std]

extern crate alloc;

use alloc::rc::Rc;
use core::fmt;

use nami::{Computed, Signal, SignalExt, impl_constant, signal::IntoComputed, zip::zip};
use suiteki::Str;

use waterui_core::layout::{PlacedGeometry, Rect};
use waterui_core::metadata::MetadataKey;
use waterui_core::{Environment, Metadata, OneOf2, View};

type TextResolverFn = dyn Fn(&Environment) -> Computed<Str>;

#[derive(Clone)]
enum TextResolver {
    Signal(Computed<Str>),
    Resolved(Rc<TextResolverFn>),
}

impl fmt::Debug for TextResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signal(signal) => f.debug_tuple("Signal").field(signal).finish(),
            Self::Resolved(_) => f.write_str("Resolved(..)"),
        }
    }
}

impl TextResolver {
    fn resolve(&self, env: &Environment) -> Computed<Str> {
        match self {
            Self::Signal(signal) => signal.clone(),
            Self::Resolved(resolver) => resolver(env),
        }
    }
}

/// Overrides the spoken label for a component when the default text is not
/// adequate.
///
/// The label is reactive and resolves against the surrounding [`Environment`]
/// so localized text tracks the active translation catalog and locale without
/// rebuilding the subtree, matching [`AccessibilityStateSignal`].
#[derive(Debug, Clone)]
pub struct AccessibilityLabel(TextResolver);

impl MetadataKey for AccessibilityLabel {}

impl AccessibilityLabel {
    /// Creates a label announced by assistive technologies when the default
    /// `WaterUI` text would be misleading or absent.
    ///
    /// Accepts a constant or any signal of [`Str`].
    ///
    /// ```
    /// # use waterui_accessibility::AccessibilityLabel;
    /// let label = AccessibilityLabel::new("Delete draft");
    /// ```
    pub fn new(label: impl IntoComputed<Str>) -> Self {
        Self(TextResolver::Signal(label.into_computed()))
    }

    /// Creates a label resolved against the surrounding [`Environment`] at
    /// realization time (for example, localized [`Text`](https://docs.rs/waterui-text)).
    pub fn resolved(resolver: impl Fn(&Environment) -> Computed<Str> + 'static) -> Self {
        Self(TextResolver::Resolved(Rc::new(resolver)))
    }

    /// Resolves the reactive label signal against `env`.
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<Str> {
        self.0.resolve(env)
    }
}

/// Carries the semantic value of a component beside its label.
///
/// The value is the node's own text content — what the component *says* —
/// rather than the name an application gives it. A formula's spoken
/// mathematics, a chart's summary, or a document's title all live here, so a
/// human-readable [`AccessibilityLabel`] like `"Euler's identity"` no longer
/// has to replace them. Assistive technologies announce it after the label,
/// matching `accessibilityValue` on `AppKit` and `aria-valuetext` on the web.
///
/// The value is reactive and resolves against the surrounding [`Environment`],
/// matching [`AccessibilityLabel`].
#[derive(Debug, Clone)]
pub struct AccessibilityValue(TextResolver);

impl MetadataKey for AccessibilityValue {}

impl AccessibilityValue {
    /// Creates a value announced by assistive technologies after the label.
    ///
    /// Accepts a constant or any signal of [`Str`].
    ///
    /// ```
    /// # use waterui_accessibility::AccessibilityValue;
    /// let value = AccessibilityValue::new("e raised to i pi plus one equals zero");
    /// ```
    pub fn new(value: impl IntoComputed<Str>) -> Self {
        Self(TextResolver::Signal(value.into_computed()))
    }

    /// Creates a value resolved against the surrounding [`Environment`] at
    /// realization time (for example, localized text).
    pub fn resolved(resolver: impl Fn(&Environment) -> Computed<Str> + 'static) -> Self {
        Self(TextResolver::Resolved(Rc::new(resolver)))
    }

    /// Resolves the reactive value signal against `env`.
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<Str> {
        self.0.resolve(env)
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
    /// # use waterui_accessibility::AccessibilityIdentifier;
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
    /// Document or section heading with its 1-based outline level (`1..=6`).
    Heading(u8),
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
    /// Modal dialog or alert surface.
    Dialog,
}

impl MetadataKey for AccessibilityRole {}

/// Gives `view` `role` unless the application already chose a role for it.
///
/// A component that draws itself into one opaque surface — a GPU texture, a
/// platform subview nothing can see into — publishes no accessibility nodes on
/// its own, so the whole region is missing from a screen reader unless the view
/// itself carries a role. Every realization of such a component owes the tree
/// the same node, and that node is this one.
///
/// Whatever the application said with `.a11y_role(...)` wins: an explicit role
/// in the environment passes the view straight through, because the application
/// knows what its surface is and the component only knows what it draws.
///
/// The role is attached as [`Metadata`], which every backend realizes.
///
/// No label is defaulted here. Only the application knows what the surface
/// shows, and an invented label reads worse than none; a component with
/// something true to say about its own contents attaches its own
/// [`AccessibilityLabel`] on top.
pub fn default_role<V: View>(
    env: &Environment,
    view: V,
    role: AccessibilityRole,
) -> OneOf2<V, Metadata<AccessibilityRole, V>> {
    if env.get::<AccessibilityRole>().is_some() {
        OneOf2::A(view)
    } else {
        OneOf2::B(Metadata::new(view, role))
    }
}

/// Controls whether this view should participate in accessibility.
///
/// Accepts a constant `bool` or any reactive boolean signal so toggling
/// hidden state updates assistive technologies without rebuilding the subtree.
#[derive(Debug, Clone)]
pub struct AccessibilityHidden(Computed<bool>);

impl MetadataKey for AccessibilityHidden {}

impl AccessibilityHidden {
    /// Creates a hidden flag for accessibility.
    #[must_use]
    pub fn new(hidden: impl IntoComputed<bool>) -> Self {
        Self(hidden.into_computed())
    }

    /// The reactive hidden signal.
    #[must_use]
    pub const fn signal(&self) -> &Computed<bool> {
        &self.0
    }

    /// Returns whether this view is hidden from assistive technologies.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.0.snapshot()
    }
}

/// Which rectangle the accessibility element of the view it names reports.
///
/// By default an element reports its geometry owner's response rectangle —
/// the view's own answer, positioned in the frame its parent assigned
/// (`docs/layout-spec.md` §2.4, §7.2). Name another source when the element
/// stands for something else: a canvas whose drawing fills its allocation, an
/// enlarged target, or a shape drawn inside the view.
///
/// The choice changes only accessibility bounds; hit testing, focus and
/// drawing keep their own geometry.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: the three geometry sources the layout contract defines (docs/layout-spec.md §7.2)"
)]
pub enum AccessibilityGeometry {
    /// The owner's response rectangle.
    #[default]
    Response,
    /// The whole frame the owner's parent assigned.
    Allocation,
    /// An explicit region in the owner's allocation-local coordinates.
    Region(Rect),
}

impl MetadataKey for AccessibilityGeometry {}

impl AccessibilityGeometry {
    /// The owner-local rectangle this source names, given the owner's
    /// retained geometry.
    #[must_use]
    pub const fn resolve(&self, geometry: &PlacedGeometry) -> Rect {
        match *self {
            Self::Response => geometry.response_rect,
            Self::Allocation => Rect::from_size(*geometry.allocation.size()),
            Self::Region(region) => region,
        }
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
    /// Merge descendants' labels in reading order and keep their actions on
    /// this element, hiding the merged descendants as separate elements.
    Combine,
}

/// The semantic checked state of a checkable control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed tri-state (unchecked, checked, mixed) that every backend maps onto its native checked state"
)]
pub enum AccessibilityChecked {
    /// The control is not checked.
    False,
    /// The control is checked.
    True,
    /// The control is in an indeterminate or mixed state.
    Mixed,
}

impl MetadataKey for AccessibilityChildren {}

impl AccessibilityChildren {
    /// Returns whether descendants should be excluded from accessibility output.
    #[must_use]
    pub const fn excludes_descendants(&self) -> bool {
        matches!(self, Self::ExcludeDescendants)
    }

    /// Returns whether descendants' labels and actions should be merged into
    /// this element.
    #[must_use]
    pub const fn combines_descendants(&self) -> bool {
        matches!(self, Self::Combine)
    }
}

/// Describes nuanced state transitions that assistive technologies use to keep
/// users in sync with complex widgets.
///
/// Every field is optional: a field left unset says nothing and inherits the
/// value an enclosing state scope set, so nested state modifiers — a
/// `.disabled(..)` inside a `.visible(..)` — combine instead of replacing one
/// another. [`inheriting`](Self::inheriting) is that merge: the inner scope's
/// set fields win and its unset fields take the outer scope's.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessibilityState {
    disabled: Option<bool>,
    selected: Option<bool>,
    checked: Option<AccessibilityChecked>,
    expanded: Option<bool>,
    busy: Option<bool>,
    hidden: Option<bool>,
}

impl MetadataKey for AccessibilityState {}
impl_constant!(AccessibilityState);

impl AccessibilityState {
    /// Creates a state that sets nothing, inheriting every field.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            disabled: None,
            selected: None,
            checked: None,
            expanded: None,
            busy: None,
            hidden: None,
        }
    }

    /// Marks whether the element is currently disabled.
    #[must_use]
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(disabled);
        self
    }

    /// Marks whether the element is currently selected.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Sets the checkbox or toggle checked value.
    #[must_use]
    pub const fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(if checked {
            AccessibilityChecked::True
        } else {
            AccessibilityChecked::False
        });
        self
    }

    /// Marks the element as having an indeterminate or mixed checked state.
    #[must_use]
    pub const fn mixed(mut self) -> Self {
        self.checked = Some(AccessibilityChecked::Mixed);
        self
    }

    /// Sets the expanded or collapsed state.
    #[must_use]
    pub const fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Marks whether the element is busy processing work.
    #[must_use]
    pub const fn busy(mut self, busy: bool) -> Self {
        self.busy = Some(busy);
        self
    }

    /// Marks whether the element should be hidden from accessibility output.
    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = Some(hidden);
        self
    }

    /// Merges this state over `outer`, the state of an enclosing scope: each
    /// field this state sets wins, and each field it leaves unset takes
    /// `outer`'s.
    #[must_use]
    pub const fn inheriting(self, outer: &Self) -> Self {
        Self {
            disabled: or(self.disabled, outer.disabled),
            selected: or(self.selected, outer.selected),
            checked: match self.checked {
                Some(checked) => Some(checked),
                None => outer.checked,
            },
            expanded: or(self.expanded, outer.expanded),
            busy: or(self.busy, outer.busy),
            hidden: or(self.hidden, outer.hidden),
        }
    }

    /// The disabled flag, or `None` when this state leaves it unset.
    #[must_use]
    pub const fn disabled_state(&self) -> Option<bool> {
        self.disabled
    }

    /// The selected flag, or `None` when this state leaves it unset.
    #[must_use]
    pub const fn selected_state(&self) -> Option<bool> {
        self.selected
    }

    /// Returns the checked state for checkbox-like roles.
    #[must_use]
    pub const fn checked_state(&self) -> Option<AccessibilityChecked> {
        self.checked
    }

    /// Returns the expanded state for expandable roles.
    #[must_use]
    pub const fn expanded_state(&self) -> Option<bool> {
        self.expanded
    }

    /// The busy flag, or `None` when this state leaves it unset.
    #[must_use]
    pub const fn busy_state(&self) -> Option<bool> {
        self.busy
    }

    /// The hidden flag, or `None` when this state leaves it unset.
    #[must_use]
    pub const fn hidden_state(&self) -> Option<bool> {
        self.hidden
    }

    /// Returns whether the element is disabled; an unset flag reads `false`.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        matches!(self.disabled, Some(true))
    }

    /// Returns whether the element is selected; an unset flag reads `false`.
    #[must_use]
    pub const fn is_selected(&self) -> bool {
        matches!(self.selected, Some(true))
    }

    /// Returns whether the element is marked busy; an unset flag reads `false`.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        matches!(self.busy, Some(true))
    }

    /// Returns whether the element is hidden from accessibility output; an
    /// unset flag reads `false`.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        matches!(self.hidden, Some(true))
    }
}

/// `Option::or` for `bool`, usable in a `const fn`.
const fn or(inner: Option<bool>, outer: Option<bool>) -> Option<bool> {
    match inner {
        Some(value) => Some(value),
        None => outer,
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

    /// Merges `local` over the state scope `env` inherits, reactively: the
    /// result follows both signals, and each field `local` leaves unset takes
    /// the enclosing scope's value ([`AccessibilityState::inheriting`]).
    #[must_use]
    pub fn resolve(env: &Environment, local: impl IntoComputed<AccessibilityState>) -> Self {
        let local = local.into_computed();
        match env.get::<Self>() {
            Some(inherited) => Self(
                zip(local, inherited.0.clone())
                    .map(|(local, inherited)| local.inheriting(&inherited))
                    .into_computed(),
            ),
            None => Self(local),
        }
    }

    /// Installs a state scope into `env`, merged over any scope already
    /// inherited from an enclosing state modifier.
    pub fn install(env: &mut Environment, local: impl IntoComputed<AccessibilityState>) {
        let merged = Self::resolve(env, local);
        env.insert(merged);
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessibilityRole, AccessibilityState, AccessibilityStateSignal, default_role};
    use nami::Signal;
    use waterui_core::{Environment, OneOf2};

    /// A surface with nothing said about it publishes the role it was given.
    #[test]
    fn an_undescribed_surface_is_given_the_default_role() {
        let view = default_role(&Environment::new(), (), AccessibilityRole::Group);

        let OneOf2::B(wrapper) = view else {
            panic!("the default role is attached as metadata");
        };
        assert_eq!(wrapper.value, AccessibilityRole::Group);
    }

    /// The application's own role wins, and nothing is attached over it.
    #[test]
    fn an_application_supplied_role_passes_the_view_through_untouched() {
        let mut env = Environment::new();
        env.insert(AccessibilityRole::Image);

        let view = default_role(&env, (), AccessibilityRole::Group);

        assert!(
            matches!(view, OneOf2::A(())),
            "a view whose role the application chose must not be wrapped again"
        );
    }

    /// Nested state scopes merge field by field: the inner scope's set fields
    /// win, its unset fields inherit, and the merge follows both signals.
    #[test]
    fn nested_state_scopes_merge_field_by_field() {
        let outer = nami::Binding::new(AccessibilityState::new().hidden(true).selected(true));
        let mut env = Environment::new();
        AccessibilityStateSignal::install(&mut env, outer.clone());
        AccessibilityStateSignal::install(
            &mut env,
            AccessibilityState::new().disabled(true).selected(false),
        );
        let merged = env
            .get::<AccessibilityStateSignal>()
            .expect("the inner scope installs a state signal")
            .state()
            .clone();

        assert_eq!(
            merged.snapshot(),
            AccessibilityState::new()
                .hidden(true)
                .selected(false)
                .disabled(true)
        );

        outer.set(AccessibilityState::new().busy(true));
        assert_eq!(
            merged.snapshot(),
            AccessibilityState::new()
                .busy(true)
                .selected(false)
                .disabled(true)
        );
    }

    #[test]
    fn accessibility_hidden_tracks_reactive_signal() {
        let hidden = nami::Binding::new(true);
        let metadata = super::AccessibilityHidden::new(hidden.clone());
        assert!(metadata.is_hidden());
        assert!(metadata.signal().snapshot());

        hidden.set(false);
        assert!(!metadata.is_hidden());
        assert!(!metadata.signal().snapshot());
    }

    #[test]
    fn accessibility_state_checked_and_expanded_accept_booleans() {
        let state = AccessibilityState::new().checked(true).expanded(false);
        assert_eq!(
            state.checked_state(),
            Some(super::AccessibilityChecked::True)
        );
        assert_eq!(state.expanded_state(), Some(false));
    }

    #[test]
    fn accessibility_children_predicates_match_variants() {
        use super::AccessibilityChildren;

        assert!(!AccessibilityChildren::Automatic.excludes_descendants());
        assert!(!AccessibilityChildren::Automatic.combines_descendants());
        assert!(AccessibilityChildren::ExcludeDescendants.excludes_descendants());
        assert!(!AccessibilityChildren::ExcludeDescendants.combines_descendants());
        assert!(!AccessibilityChildren::Combine.excludes_descendants());
        assert!(AccessibilityChildren::Combine.combines_descendants());
    }
}

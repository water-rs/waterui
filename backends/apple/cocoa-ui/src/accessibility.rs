//! Accessibility settings on a view: role, label, value, state.
//!
//! The platform trees (`UIAccessibility` and `NSAccessibility`) agree on the
//! concepts and disagree on the plumbing, so this module keeps one typed
//! [`Role`] and a small set of apply functions; what `UIKit` expresses as
//! trait bits and `AppKit` as role/value attributes is the same request
//! here.
//!
//! # Safety
//!
//! No unsafe code; every setter is a safe property call on the view.

/// What a view is, for assistive technologies.
///
/// The case set is the one the consuming renderer's accessibility
/// vocabulary uses; [`uikit_traits`] and [`appkit_role`] translate it into
/// each platform's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
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
    /// Modal dialog or alert surface.
    Dialog,
}

/// A checkbox's check state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checked {
    /// Not checked.
    Unchecked,
    /// Checked.
    Checked,
    /// Neither checked nor unchecked.
    Mixed,
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2_foundation::NSString;
    use objc2_ui_kit::{
        NSObjectUIAccessibility, UIAccessibilityTraitAdjustable, UIAccessibilityTraitButton,
        UIAccessibilityTraitHeader, UIAccessibilityTraitImage, UIAccessibilityTraitLink,
        UIAccessibilityTraitNotEnabled, UIAccessibilityTraitSearchField,
        UIAccessibilityTraitSelected, UIAccessibilityTraitStaticText,
        UIAccessibilityTraitUpdatesFrequently, UIAccessibilityTraits, UIView,
    };

    use super::{Checked, Role};

    /// The traits `set_role` manages: the union of everything any role can
    /// add, removed before the role's own are set.
    fn managed_traits() -> UIAccessibilityTraits {
        // SAFETY: the trait constants are immutable `u64`s published by
        // UIKit.
        unsafe {
            UIAccessibilityTraitButton
                | UIAccessibilityTraitLink
                | UIAccessibilityTraitImage
                | UIAccessibilityTraitStaticText
                | UIAccessibilityTraitHeader
                | UIAccessibilityTraitSearchField
                | UIAccessibilityTraitAdjustable
                | UIAccessibilityTraitUpdatesFrequently
        }
    }

    /// The traits `UIKit` gives this role, or `None` when the role has no
    /// `UIKit` equivalent.
    #[must_use]
    pub fn traits(role: Role) -> Option<UIAccessibilityTraits> {
        // SAFETY: the trait constants are immutable `u64`s published by
        // UIKit; reading them is safe.
        Some(unsafe {
            match role {
                Role::Button
                | Role::Checkbox
                | Role::RadioButton
                | Role::Switch
                | Role::Tab
                | Role::MenuItem
                | Role::MenuItemCheckbox
                | Role::MenuItemRadio
                | Role::Combobox => UIAccessibilityTraitButton,
                Role::Link => UIAccessibilityTraitLink,
                Role::Image => UIAccessibilityTraitImage,
                Role::Text | Role::Footer | Role::Article | Role::ListItem => {
                    UIAccessibilityTraitStaticText
                }
                Role::Header => UIAccessibilityTraitHeader,
                Role::Search => UIAccessibilityTraitSearchField,
                Role::Slider => UIAccessibilityTraitAdjustable,
                Role::ProgressBar => UIAccessibilityTraitUpdatesFrequently,
                Role::Navigation
                | Role::Main
                | Role::Section
                | Role::List
                | Role::TabList
                | Role::TabPanel
                | Role::Menu
                | Role::MenuBar
                | Role::Option
                | Role::Group => 0,
                Role::Dialog => return None,
            }
        })
    }

    /// Applies `role`'s traits to `view`, replacing any previously managed
    /// ones, and marks it an accessibility element.
    pub fn set_role(view: &UIView, role: Role) {
        let mtm = objc2::MainThreadMarker::from(view);
        view.setIsAccessibilityElement(true, mtm);
        let Some(role_traits) = traits(role) else {
            return;
        };
        let mut current = view.accessibilityTraits(mtm);
        current &= !managed_traits();
        current |= role_traits;
        view.setAccessibilityTraits(current, mtm);
    }

    /// Marks `view` an accessibility element and gives it `label`.
    pub fn set_label(view: &UIView, label: &NSString) {
        let mtm = objc2::MainThreadMarker::from(view);
        view.setIsAccessibilityElement(true, mtm);
        view.setAccessibilityLabel(Some(label), mtm);
    }

    /// Marks `view` an accessibility element and gives it `value`.
    pub fn set_value(view: &UIView, value: &NSString) {
        let mtm = objc2::MainThreadMarker::from(view);
        view.setIsAccessibilityElement(true, mtm);
        view.setAccessibilityValue(Some(value), mtm);
    }

    /// Gives `view` the stable automation identifier UI tests locate it
    /// by — `accessibilityIdentifier`.
    pub fn set_identifier(view: &UIView, identifier: &NSString) {
        // SAFETY: `setAccessibilityIdentifier:` is `NSObject`'s
        // `UIAccessibilityIdentification` method, available on every view.
        unsafe { () = objc2::msg_send![view, setAccessibilityIdentifier: identifier] }
    }

    /// Hides or unhides `view` and its descendants.
    pub fn set_hidden(view: &UIView, hidden: bool) {
        view.setAccessibilityElementsHidden(hidden, objc2::MainThreadMarker::from(view));
    }

    /// Whether `view` and its descendants are hidden.
    #[must_use]
    pub fn hidden(view: &UIView) -> bool {
        view.accessibilityElementsHidden(objc2::MainThreadMarker::from(view))
    }

    /// Marks `view` an accessibility element with no accessible
    /// descendants.
    pub fn exclude_children(view: &UIView) {
        let mtm = objc2::MainThreadMarker::from(view);
        view.setIsAccessibilityElement(true, mtm);
        view.setAccessibilityElementsHidden(true, mtm);
    }

    /// The current accessibility value, if any.
    #[must_use]
    pub fn value(view: &UIView) -> Option<objc2::rc::Retained<NSString>> {
        view.accessibilityValue(objc2::MainThreadMarker::from(view))
    }

    /// The current accessibility hint, if any.
    #[must_use]
    pub fn hint(view: &UIView) -> Option<objc2::rc::Retained<NSString>> {
        view.accessibilityHint(objc2::MainThreadMarker::from(view))
    }

    /// Applies the dynamic accessibility state.
    ///
    /// `disabled`, `selected`, `busy` toggle traits and the hint; `checked`
    /// announces a value. When `checked` is `None` the value goes back to
    /// `original_value`, but only once a check was actually announced —
    /// a value an application set otherwise survives.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    pub fn apply_state(
        view: &UIView,
        disabled: bool,
        selected: bool,
        checked: Option<Checked>,
        checked_value_written: bool,
        original_value: Option<&NSString>,
        busy: bool,
        original_hint: Option<&NSString>,
    ) {
        let mtm = objc2::MainThreadMarker::from(view);
        view.setIsAccessibilityElement(true, mtm);
        let mut traits = view.accessibilityTraits(mtm);
        // SAFETY: the trait constants are immutable `u64`s published by
        // UIKit; reading them is safe.
        let (not_enabled, selected_trait) =
            unsafe { (UIAccessibilityTraitNotEnabled, UIAccessibilityTraitSelected) };
        traits = if disabled {
            traits | not_enabled
        } else {
            traits & !not_enabled
        };
        traits = if selected {
            traits | selected_trait
        } else {
            traits & !selected_trait
        };
        view.setAccessibilityTraits(traits, mtm);
        if let Some(checked) = checked {
            let text = match checked {
                Checked::Unchecked => "Unchecked",
                Checked::Checked => "Checked",
                Checked::Mixed => "Mixed",
            };
            view.setAccessibilityValue(Some(&NSString::from_str(text)), mtm);
        } else if checked_value_written {
            view.setAccessibilityValue(original_value, mtm);
        }
        let busy_hint = NSString::from_str("Busy");
        view.setAccessibilityHint(
            if busy {
                Some(&*busy_hint)
            } else {
                original_hint
            },
            mtm,
        );
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2_app_kit::{
        NSAccessibility, NSAccessibilityButtonRole, NSAccessibilityCheckBoxRole,
        NSAccessibilityComboBoxRole, NSAccessibilityGroupRole, NSAccessibilityImageRole,
        NSAccessibilityLinkRole, NSAccessibilityListRole, NSAccessibilityMenuBarRole,
        NSAccessibilityMenuItemRole, NSAccessibilityMenuRole, NSAccessibilityProgressIndicatorRole,
        NSAccessibilityRadioButtonRole, NSAccessibilityRole, NSAccessibilitySliderRole,
        NSAccessibilityStaticTextRole, NSAccessibilityTabGroupRole,
    };
    use objc2_foundation::{NSArray, NSNumber, NSString};

    use super::{Checked, Role};

    /// The `NSAccessibility` role this role is announced as, or `None` when
    /// `AppKit` has no equivalent.
    ///
    /// # Safety
    ///
    /// The `extern` role constants are immutable `NSString`s published by
    /// `AppKit`; reading them is safe.
    #[must_use]
    pub fn role(role: Role) -> Option<&'static NSAccessibilityRole> {
        // SAFETY: reading an immutable string constant exported by AppKit.
        Some(unsafe {
            match role {
                Role::Button | Role::Tab => NSAccessibilityButtonRole,
                Role::Link => NSAccessibilityLinkRole,
                Role::Image => NSAccessibilityImageRole,
                Role::Text => NSAccessibilityStaticTextRole,
                Role::List => NSAccessibilityListRole,
                Role::Checkbox | Role::MenuItemCheckbox => NSAccessibilityCheckBoxRole,
                Role::RadioButton | Role::MenuItemRadio => NSAccessibilityRadioButtonRole,
                Role::Slider => NSAccessibilitySliderRole,
                Role::ProgressBar => NSAccessibilityProgressIndicatorRole,
                Role::TabList => NSAccessibilityTabGroupRole,
                Role::Menu => NSAccessibilityMenuRole,
                Role::MenuItem => NSAccessibilityMenuItemRole,
                Role::MenuBar => NSAccessibilityMenuBarRole,
                Role::Combobox => NSAccessibilityComboBoxRole,
                Role::Header
                | Role::Footer
                | Role::Navigation
                | Role::Main
                | Role::Search
                | Role::Article
                | Role::Section
                | Role::ListItem
                | Role::Switch
                | Role::TabPanel
                | Role::Option
                | Role::Group => NSAccessibilityGroupRole,
                Role::Dialog => return None,
            }
        })
    }

    /// Applies `role` to `element` and marks it an accessibility element.
    pub fn set_role(element: &ProtocolObject<dyn NSAccessibility>, role: Role) {
        element.setAccessibilityElement(true);
        if let Some(role) = self::role(role) {
            element.setAccessibilityRole(Some(role));
        }
    }

    /// Marks `element` an accessibility element and gives it `label`.
    pub fn set_label(element: &ProtocolObject<dyn NSAccessibility>, label: &NSString) {
        element.setAccessibilityElement(true);
        element.setAccessibilityLabel(Some(label));
    }

    /// Marks `element` an accessibility element and gives it `value`.
    pub fn set_value(element: &ProtocolObject<dyn NSAccessibility>, value: &NSString) {
        element.setAccessibilityElement(true);
        // SAFETY: `NSString` is a valid property-list object.
        unsafe { element.setAccessibilityValue(Some(AsRef::<AnyObject>::as_ref(value))) };
    }

    /// Gives `element` the stable automation identifier UI tests locate
    /// it by — `accessibilityIdentifier`.
    pub fn set_identifier(element: &ProtocolObject<dyn NSAccessibility>, identifier: &NSString) {
        element.setAccessibilityIdentifier(Some(identifier));
    }

    /// Hides or unhides `element`: hiding removes it from the
    /// accessibility tree.
    pub fn set_hidden(element: &ProtocolObject<dyn NSAccessibility>, hidden: bool) {
        element.setAccessibilityElement(!hidden);
    }

    /// Marks `element` an accessibility element with no accessible
    /// children.
    pub fn exclude_children(element: &ProtocolObject<dyn NSAccessibility>) {
        element.setAccessibilityElement(true);
        // SAFETY: an empty `NSArray` is a valid children list.
        unsafe { element.setAccessibilityChildren(Some(&NSArray::new())) };
    }

    /// Whether `element` is in the accessibility tree.
    #[must_use]
    pub fn is_element(element: &ProtocolObject<dyn NSAccessibility>) -> bool {
        element.isAccessibilityElement()
    }

    /// Applies the dynamic accessibility state: enabled, selected, checked,
    /// expanded, busy.
    pub fn apply_state(
        element: &ProtocolObject<dyn NSAccessibility>,
        disabled: bool,
        selected: bool,
        checked: Option<Checked>,
        expanded: Option<bool>,
        busy: bool,
    ) {
        element.setAccessibilityElement(true);
        element.setAccessibilityEnabled(!disabled);
        element.setAccessibilitySelected(selected);
        if let Some(checked) = checked {
            let (value, description) = match checked {
                Checked::Unchecked => (0, "Unchecked"),
                Checked::Checked => (1, "Checked"),
                Checked::Mixed => (-1, "Mixed"),
            };
            let number = NSNumber::new_i32(value);
            // SAFETY: `NSNumber` is a valid property-list object.
            unsafe { element.setAccessibilityValue(Some(AsRef::<AnyObject>::as_ref(&*number))) };
            element.setAccessibilityValueDescription(Some(&NSString::from_str(description)));
        }
        if let Some(expanded) = expanded {
            element.setAccessibilityExpanded(expanded);
        }
        let busy_label = NSString::from_str("Busy");
        element.setAccessibilityHelp(if busy { Some(&*busy_label) } else { None });
    }
}

pub use imp::*;

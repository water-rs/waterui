//! Accessibility settings on a view: role, label, value, state, frame.
//!
//! The platform trees (`UIAccessibility` and `NSAccessibility`) agree on the
//! concepts and disagree on the plumbing, so this module keeps one typed
//! [`Role`] and a small set of apply functions; what `UIKit` expresses as
//! trait bits and `AppKit` as role/value attributes is the same request
//! here.
//!
//! # Safety
//!
//! The attribute setters are safe property calls on the view. The frame
//! provider projects a live rect through each platform's own query hook —
//! `accessibilityFrameBlock` on `UIKit`, an `accessibilityFrame` override on
//! `AppKit`, installed on the view's own (cocoa-ui-owned) class — so the
//! reported frame is computed when assistive technology asks for it, not
//! stored stale.

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

/// The accessibility frame `view` should report, in `view`'s own
/// coordinate space.
///
/// [`set_frame_provider`] projects it to the screen space the platform
/// API reports. `None` falls back to the view's own frame — the answer
/// `NSView`/`UIView` gives without a provider.
pub type FrameProvider = std::rc::Rc<dyn Fn(&crate::PlatformView) -> Option<crate::Rect>>;

/// Detaches the provider [`set_frame_provider`] installed.
///
/// The view then reports its own frame again. `AppKit` keeps the
/// class-level override installed — a provider-less instance dispatches
/// to the class's original implementation, which is exactly what it
/// answered before.
#[derive(Debug)]
#[must_use = "the provider stays installed only while the guard lives"]
pub struct FrameProviderGuard {
    inner: imp::GuardState,
}

impl Drop for FrameProviderGuard {
    fn drop(&mut self) {
        imp::detach(&mut self.inner);
    }
}

/// Installs `provider` as `view`'s accessibility-frame source.
///
/// The frame is asked for — not stored: the provider runs when assistive
/// technology reads the frame, so the reported rect tracks the view's
/// live geometry (scroll, window moves, ancestor layout) instead of
/// going stale the moment either moves. `view` keeps its own frame
/// either way: the provider only rewrites what the accessibility APIs
/// report.
///
/// The caller owns the geometry. This is a projection hook, not a source
/// of truth: the rect it is fed should be the view's resolved response
/// rectangle as its layout engine positioned it — never a rect inferred
/// from a descendant's bounds or a re-measured size the backend centered
/// itself.
pub fn set_frame_provider(
    view: &crate::PlatformView,
    provider: impl Fn(&crate::PlatformView) -> Option<crate::Rect> + 'static,
) -> FrameProviderGuard {
    FrameProviderGuard {
        inner: imp::attach(view, std::rc::Rc::new(provider)),
    }
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

    /// The installed provider: the block `UIKit` copied into the property,
    /// plus the view it was installed on — needed to clear the property
    /// when the guard drops.
    pub struct GuardState {
        view: objc2::rc::Weak<UIView>,
        _block: block2::RcBlock<dyn Fn() -> objc2_core_foundation::CGRect>,
    }

    impl core::fmt::Debug for GuardState {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("GuardState").finish_non_exhaustive()
        }
    }

    /// Installs `provider` as `view`'s `accessibilityFrameBlock`: `UIKit`
    /// calls the block every time the frame is read, so the answer tracks
    /// the view's live geometry. `None` from the provider reports the
    /// view's own frame — the same conversion `UIKit`'s default performs.
    pub fn attach(view: &UIView, provider: super::FrameProvider) -> GuardState {
        let weak = objc2::rc::Weak::new(view);
        let block = block2::RcBlock::new(move || -> objc2_core_foundation::CGRect {
            let Some(view) = weak.load() else {
                return objc2_core_foundation::CGRect::ZERO;
            };
            let rect = provider(&view).unwrap_or_else(|| crate::view::bounds(&view));
            // `UIAccessibilityConvertFrameToScreenCoordinates` converts a
            // rect in `view`'s coordinate space to screen coordinates.
            objc2_ui_kit::UIAccessibilityConvertFrameToScreenCoordinates(rect.into(), &view)
        });
        let mtm = objc2::MainThreadMarker::from(view);
        // SAFETY: `setAccessibilityFrameBlock:` copies the block into the
        // view's property; `RcBlock::as_ptr` lends it a valid one.
        unsafe { view.setAccessibilityFrameBlock(block2::RcBlock::as_ptr(&block), mtm) };
        GuardState {
            view: objc2::rc::Weak::new(view),
            _block: block,
        }
    }

    /// Clears the property the block was copied into, returning the view
    /// to its own reported frame.
    pub fn detach(state: &mut GuardState) {
        let Some(view) = state.view.load() else {
            return;
        };
        let mtm = objc2::MainThreadMarker::from(&*view);
        // SAFETY: clearing the property releases the copied block.
        unsafe {
            view.setAccessibilityFrameBlock(core::ptr::null_mut(), mtm);
        }
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
        NSAccessibilityStaticTextRole, NSAccessibilityTabGroupRole, NSView,
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

    /// The provider registered for a view, keyed by its address.
    type Registry = std::collections::HashMap<usize, super::FrameProvider>;

    std::thread_local! {
        /// Live frame providers. `AppKit` gives a view no frame hook of
        /// its own — `accessibilityFrame` is a stored property on
        /// `NSAccessibilityElement` but a *computed* accessor on
        /// `NSView` — so the only way a view answers with live geometry
        /// is for the method to consult a registry keyed by the
        /// instance's address. Views are main-thread objects and every
        /// reader is an accessibility call on that thread, so a
        /// thread-local map needs no synchronization.
        static PROVIDERS: core::cell::RefCell<Registry> =
            core::cell::RefCell::new(std::collections::HashMap::new());
        /// The `accessibilityFrame` implementation each overridden class
        /// answered with before the override landed — the dispatch target
        /// for every provider-less instance of that class.
        static ORIGINALS: core::cell::RefCell<
            std::collections::HashMap<*const objc2::runtime::AnyClass, objc2::runtime::Imp>,
        > = core::cell::RefCell::new(std::collections::HashMap::new());
    }

    /// The `accessibilityFrame` implementation installed on classes a
    /// provider was attached to. A provider's answer wins; an instance
    /// with none dispatches to the implementation the class answered with
    /// before the override — which `ORIGINALS` recorded, since the method
    /// is usually inherited rather than the class's own.
    ///
    /// # Safety
    ///
    /// Installed through `class_replaceMethod` only under the
    /// `accessibilityFrame` selector and the type encoding the runtime
    /// already associates with it — same signature, same contract. The
    /// function only ever runs as that method: `this` is a live `NSView`
    /// (providers attach to `PlatformView` subclasses only) and `_cmd` is
    /// the `accessibilityFrame` selector.
    unsafe extern "C-unwind" fn accessibility_frame(
        this: *mut objc2::runtime::AnyObject,
        cmd: objc2::runtime::Sel,
    ) -> objc2_foundation::NSRect {
        crate::callback::guarded("accessibility frame provider", || {
            let provider = PROVIDERS.with(|map| map.borrow().get(&(this as usize)).cloned());
            if let Some(provider) = provider {
                // SAFETY: per the function's safety note, `this` is a live
                // `NSView`.
                let view: &NSView = unsafe { &*(this.cast::<NSView>()) };
                if let Some(rect) = provider(view) {
                    // `NSAccessibilityFrameInView` converts a rect in
                    // `view`'s coordinate space to screen coordinates.
                    return objc2_app_kit::NSAccessibilityFrameInView(view, rect.into());
                }
            }
            // The override is inherited: an instance of a subclass of an
            // overridden class reaches this IMP too, and its class has no
            // entry of its own. Walk the chain to the nearest class the
            // override landed on — its recorded implementation is exactly
            // what this instance would have dispatched to.
            // SAFETY: `this` is a live object per the function's safety
            // note, so `class()` reads a live isa.
            let mut class = unsafe { (*this).class() };
            let original = loop {
                let found = ORIGINALS.with(|map| {
                    map.borrow()
                        .get(&(core::ptr::from_ref::<objc2::runtime::AnyClass>(class)))
                        .copied()
                });
                if let Some(imp) = found {
                    break Some(imp);
                }
                let Some(superclass) = class.superclass() else {
                    break None;
                };
                class = superclass;
            };
            original.map_or(objc2_foundation::NSRect::ZERO, |imp| {
                // SAFETY: `imp` is the `accessibilityFrame` implementation
                // this instance's class dispatched to before the override —
                // same signature, still the inherited answer for
                // provider-less instances.
                let imp: unsafe extern "C-unwind" fn(
                    *mut objc2::runtime::AnyObject,
                    objc2::runtime::Sel,
                ) -> objc2_foundation::NSRect = unsafe { core::mem::transmute(imp) };
                // SAFETY: the call targets the method's own receiver and
                // selector, which the transmuted signature expects.
                unsafe { imp(this, cmd) }
            })
        })
    }

    /// The guard's key into [`PROVIDERS`]: the view's address.
    pub type GuardState = usize;

    /// Installs `provider` as `view`'s `accessibilityFrame` source: the
    /// class override lands once, the per-instance registration is what
    /// the override consults.
    pub fn attach(view: &NSView, provider: super::FrameProvider) -> GuardState {
        ensure_override(view);
        let key = core::ptr::from_ref::<NSView>(view) as usize;
        PROVIDERS.with(|map| {
            map.borrow_mut().insert(key, provider);
        });
        key
    }

    /// Replaces `view`'s class's `accessibilityFrame` with the dispatching
    /// implementation, recording what the class answered before. Only the
    /// concrete class is touched: `class_replaceMethod` adds the method to
    /// the class itself when the selector is inherited, so siblings and
    /// superclasses keep their own implementation.
    fn ensure_override(view: &NSView) {
        let class = view.class();
        let key = core::ptr::from_ref::<objc2::runtime::AnyClass>(class);
        if ORIGINALS.with(|map| map.borrow().contains_key(&key)) {
            return;
        }
        let sel = objc2::sel!(accessibilityFrame);
        let method = class
            .instance_method(sel)
            .expect("NSObject implements accessibilityFrame");
        let original = method.implementation();
        // When `class` already inherits the override from an ancestor —
        // the resolved IMP is ours — no replacement is needed: the
        // inherited IMP consults `PROVIDERS` for this instance's address,
        // and its no-provider walk reaches the ancestor's original. Adding
        // a method here would only record our own IMP as the "original"
        // and recurse.
        if original as usize == accessibility_frame as *const () as usize {
            return;
        }
        // SAFETY: `method_getTypeEncoding` reads a live method's encoding —
        // the signature `accessibility_frame` is registered under, keeping
        // the ABI identical to the method it replaces.
        let types = unsafe { objc2::ffi::method_getTypeEncoding(method) };
        // SAFETY: `class` is a live class object — the concrete class of
        // `view`, a cocoa-ui-defined `NSView` subclass — and the IMP has
        // the signature the selector and encoding declare. The transmute
        // only widens the typed pointer to the runtime's untyped `Imp`.
        let imp: objc2::runtime::Imp = unsafe {
            core::mem::transmute::<
                unsafe extern "C-unwind" fn(
                    *mut objc2::runtime::AnyObject,
                    objc2::runtime::Sel,
                ) -> objc2_foundation::NSRect,
                objc2::runtime::Imp,
            >(accessibility_frame)
        };
        // SAFETY: `key` is the live class object resolved above, `sel` and
        // `types` describe `accessibilityFrame` exactly, and `imp` carries
        // that signature — the replacement is ABI-identical.
        unsafe {
            objc2::ffi::class_replaceMethod(key.cast_mut(), sel, imp, types);
        }
        ORIGINALS.with(|map| map.borrow_mut().insert(key, original));
    }

    /// Drops `view`'s provider; its class keeps the override, whose
    /// no-provider path is what the class answered all along.
    pub fn detach(key: &mut GuardState) {
        PROVIDERS.with(|map| {
            map.borrow_mut().remove(key);
        });
    }
}

pub use imp::*;

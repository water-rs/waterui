//! The `accessibility` metadata family:
//! `IgnorableMetadata<Accessibility{Label,Value,Role,Hidden,Children,
//! State,StateSignal}>` wrapped around a child.
//!
//! Mirrors `WuiAccessibilityMetadata`: every property lands on the
//! content's platform view, marked an accessibility element where the
//! Swift classes did. The leaf is the child's own — no wrapper — so a
//! chain of metadata applies to the innermost view, what Swift's
//! `accessibilityTarget` drill reached.
//!
//! A leaf's view reports the geometry its placement committed as its
//! `accessibilityFrame` — the geometry owner's response rectangle
//! (`docs/layout-spec.md` §7.2) rather than the frame the parent assigned:
//! a `view.size(8, 8).a11y_label("…")` under a window-filling overlay is
//! assigned the window but answers 8×8, and announces the 8×8.
//! `a11y_geometry` names the allocation or an explicit region of that
//! committed geometry instead.
//!
//! `AccessibilityState` and `AccessibilityStateSignal` share one applier:
//! on `UIKit`, `checked` writes a spoken value the first time it arrives
//! and restores the original value only after it was ever written —
//! `wroteCheckedValue`; `busy` swaps the hint for "Busy" and restores the
//! original otherwise; `hidden` restores `accessibilityElementsHidden`.
//! On `AppKit`, `expanded` is applied only when present and `hidden`
//! restores `isAccessibilityElement`.

use cocoa_ui::accessibility::{self as a11y, Role};
use cocoa_ui::response;
use cocoa_ui::{PlatformView, view};
use objc2_foundation::NSString;
use waterui::accessibility::{
    AccessibilityChecked, AccessibilityChildren, AccessibilityGeometry, AccessibilityHidden,
    AccessibilityLabel, AccessibilityRole, AccessibilityState, AccessibilityStateSignal,
    AccessibilityValue,
};
use waterui_core::IgnorableMetadata;

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;
use crate::proposal::in_view;

#[cfg(target_os = "macos")]
mod target {
    use cocoa_ui::PlatformView;
    use cocoa_ui::objc2_app_kit::NSAccessibility;
    use objc2::runtime::ProtocolObject;

    /// The platform accessibility protocol face of a view — the receiver
    /// every `AppKit` accessibility call takes.
    pub(super) fn of(view: &PlatformView) -> &ProtocolObject<dyn NSAccessibility> {
        ProtocolObject::from_ref(view)
    }
}

#[cfg(target_os = "ios")]
mod target {
    use cocoa_ui::PlatformView;

    /// On `UIKit` the accessibility receiver is the view itself.
    pub(super) const fn of(view: &PlatformView) -> &PlatformView {
        view
    }
}

/// Names the rectangle `leaf`'s accessibility element reports — the
/// allocation or an explicit region of its committed geometry instead of
/// the response rectangle.
///
/// # Panics
///
/// When `leaf`'s platform view carries no response geometry — every leaf
/// view class carries one.
fn name_geometry(leaf: &NativeLeaf, source: AccessibilityGeometry) {
    let slot = response::of(leaf.view()).expect("every leaf view class carries response geometry");
    let geometry = leaf.geometry();
    slot.set_accessibility_frame_handler(move |view| {
        geometry
            .get()
            .map(|placed| in_view(view, &placed, source.resolve(&placed)))
    });
}

/// The waterui checked state as a kit `Checked`.
const fn checked(state: AccessibilityChecked) -> a11y::Checked {
    match state {
        AccessibilityChecked::False => a11y::Checked::Unchecked,
        AccessibilityChecked::True => a11y::Checked::Checked,
        AccessibilityChecked::Mixed => a11y::Checked::Mixed,
    }
}

/// The waterui role as a kit `Role` — the enums line up member for member.
const fn role(role: &AccessibilityRole) -> Role {
    match role {
        AccessibilityRole::Button => Role::Button,
        AccessibilityRole::Link => Role::Link,
        AccessibilityRole::Image => Role::Image,
        AccessibilityRole::Text => Role::Text,
        AccessibilityRole::Header => Role::Header,
        AccessibilityRole::Footer => Role::Footer,
        AccessibilityRole::Navigation => Role::Navigation,
        AccessibilityRole::Main => Role::Main,
        AccessibilityRole::Search => Role::Search,
        AccessibilityRole::Article => Role::Article,
        AccessibilityRole::Section => Role::Section,
        AccessibilityRole::List => Role::List,
        AccessibilityRole::ListItem => Role::ListItem,
        AccessibilityRole::Checkbox => Role::Checkbox,
        AccessibilityRole::RadioButton => Role::RadioButton,
        AccessibilityRole::Switch => Role::Switch,
        AccessibilityRole::Slider => Role::Slider,
        AccessibilityRole::ProgressBar => Role::ProgressBar,
        AccessibilityRole::Tab => Role::Tab,
        AccessibilityRole::TabList => Role::TabList,
        AccessibilityRole::TabPanel => Role::TabPanel,
        AccessibilityRole::Menu => Role::Menu,
        AccessibilityRole::MenuItem => Role::MenuItem,
        AccessibilityRole::MenuBar => Role::MenuBar,
        AccessibilityRole::MenuItemCheckbox => Role::MenuItemCheckbox,
        AccessibilityRole::MenuItemRadio => Role::MenuItemRadio,
        AccessibilityRole::Combobox => Role::Combobox,
        AccessibilityRole::Option => Role::Option,
        AccessibilityRole::Dialog => Role::Dialog,
        _ => Role::Group,
    }
}

/// Applies an `AccessibilityState` to the view — `applyState`.
#[cfg(target_os = "ios")]
fn apply_state(
    target: &PlatformView,
    state: &AccessibilityState,
    checked_value_written: bool,
    original_value: Option<&NSString>,
    original_hint: Option<&NSString>,
) {
    a11y::apply_state(
        target,
        state.is_disabled(),
        state.is_selected(),
        state.checked_state().map(checked),
        checked_value_written,
        original_value,
        state.is_busy(),
        original_hint,
    );
}

/// Applies an `AccessibilityState` to the view — `applyState`.
#[cfg(target_os = "macos")]
fn apply_state(target: &PlatformView, state: &AccessibilityState) {
    a11y::apply_state(
        target::of(target),
        state.is_disabled(),
        state.is_selected(),
        state.checked_state().map(checked),
        state.expanded_state(),
        state.is_busy(),
    );
}

/// Applies `hidden` — `apply(hidden:)`: unhiding restores the value the
/// channel held before the state was bound.
#[cfg(target_os = "ios")]
fn apply_hidden(target: &PlatformView, hidden: bool, original: bool) {
    a11y::set_hidden(target, if hidden { true } else { original });
}

/// Applies `hidden` — `apply(hidden:)`: unhiding restores `isAccessibilityElement`.
#[cfg(target_os = "macos")]
fn apply_hidden(target: &PlatformView, hidden: bool, original: bool) {
    a11y::set_hidden(target::of(target), if hidden { true } else { !original });
}

/// Shared bind for `AccessibilityState` (constant) and
/// `AccessibilityStateSignal` (reactive): each delivery applies the whole
/// state plus the `hidden` channel's restore semantics.
fn install_state(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<IgnorableMetadata<AccessibilityState>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        leaf.keep(target.clone());
        #[cfg(target_os = "ios")]
        {
            use alloc::rc::Rc;
            use core::cell::{Cell, RefCell};
            let state = Rc::new(RefCell::new(metadata.value));
            let wrote_checked = Cell::new(false);
            let original_value = a11y::value(&target);
            let original_hint = a11y::hint(&target);
            let original_hidden = a11y::hidden(&target);
            let apply = {
                let state = Rc::clone(&state);
                let target = target.clone();
                move || {
                    let state = state.borrow();
                    apply_state(
                        &target,
                        &state,
                        wrote_checked.get(),
                        original_value.as_deref(),
                        original_hint.as_deref(),
                    );
                    if state.checked_state().is_some() {
                        wrote_checked.set(true);
                    }
                }
            };
            apply();
            apply_hidden(&target, state.borrow().is_hidden(), original_hidden);
            leaf.keep(state);
        }
        #[cfg(target_os = "macos")]
        {
            let original_element = a11y::is_element(target::of(&target));
            apply_state(&target, &metadata.value);
            apply_hidden(&target, metadata.value.is_hidden(), original_element);
        }
        leaf
    });

    install_state_signal(dispatcher);
}

/// `AccessibilityStateSignal`: the reactive channel of the state pair.
fn install_state_signal(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<IgnorableMetadata<AccessibilityStateSignal>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        leaf.keep(target.clone());
        #[cfg(target_os = "ios")]
        {
            use alloc::rc::Rc;
            use core::cell::{Cell, RefCell};
            use waterui::reactive::Signal;
            let state = Rc::new(RefCell::new(metadata.value.state().snapshot()));
            let wrote_checked = Rc::new(Cell::new(false));
            let original_value = a11y::value(&target);
            let original_hint = a11y::hint(&target);
            let original_hidden = a11y::hidden(&target);
            leaf.watch(metadata.value.state(), {
                let state = Rc::clone(&state);
                let target = target.clone();
                let wrote_checked = Rc::clone(&wrote_checked);
                let original_value = original_value.clone();
                let original_hint = original_hint.clone();
                move |wctx| {
                    let next = wctx.value().clone();
                    *state.borrow_mut() = next.clone();
                    apply_state(
                        &target,
                        &next,
                        wrote_checked.get(),
                        original_value.as_deref(),
                        original_hint.as_deref(),
                    );
                    if next.checked_state().is_some() {
                        wrote_checked.set(true);
                    }
                    apply_hidden(&target, next.is_hidden(), original_hidden);
                }
            });
            // Initial application, same as a watcher's first delivery.
            let initial = state.borrow().clone();
            apply_state(
                &target,
                &initial,
                wrote_checked.get(),
                original_value.as_deref(),
                original_hint.as_deref(),
            );
            if initial.checked_state().is_some() {
                wrote_checked.set(true);
            }
            apply_hidden(&target, initial.is_hidden(), original_hidden);
            leaf.keep(state);
        }
        #[cfg(target_os = "macos")]
        {
            let original_element = a11y::is_element(target::of(&target));
            leaf.bind(metadata.value.state(), move |state| {
                apply_state(&target, &state);
                apply_hidden(&target, state.is_hidden(), original_element);
            });
        }
        leaf
    });
}

/// Installs the accessibility metadata handlers on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<IgnorableMetadata<AccessibilityLabel>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        leaf.keep(target.clone());
        leaf.bind(metadata.value.signal(), move |label| {
            a11y::set_label(target::of(&target), &NSString::from_str(&label));
        });
        leaf
    });

    dispatcher.register_view::<IgnorableMetadata<AccessibilityValue>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        leaf.keep(target.clone());
        leaf.bind(metadata.value.signal(), move |value| {
            a11y::set_value(target::of(&target), &NSString::from_str(&value));
        });
        leaf
    });

    dispatcher.register_view::<IgnorableMetadata<AccessibilityRole>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        a11y::set_role(target::of(&target), role(&metadata.value));
        leaf.keep(target);
        leaf
    });

    dispatcher.register_view::<IgnorableMetadata<AccessibilityGeometry>>(|metadata, ctx| {
        let leaf = ctx.render(metadata.content);
        name_geometry(&leaf, metadata.value);
        leaf
    });

    dispatcher.register_view::<IgnorableMetadata<AccessibilityHidden>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        a11y::set_hidden(target::of(&target), metadata.value.is_hidden());
        leaf.keep(target);
        leaf
    });

    dispatcher.register_view::<IgnorableMetadata<AccessibilityChildren>>(|metadata, ctx| {
        let mut leaf = ctx.render(metadata.content);
        let target = view::retain_base(leaf.view());
        if metadata.value.excludes_descendants() {
            a11y::exclude_children(target::of(&target));
        }
        leaf.keep(target);
        leaf
    });

    install_state(dispatcher);
}

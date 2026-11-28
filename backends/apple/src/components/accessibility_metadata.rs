//! The `accessibility` metadata family:
//! `Metadata<Accessibility{Label,Value,Role,Hidden,Children,
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
//! `AccessibilityState` and `AccessibilityStateSignal` share one applier.
//! Nested state scopes merge field by field through the environment, and
//! only the innermost scope on a leaf applies the merged state. On `UIKit`,
//! `checked` writes a spoken value the first time it arrives and restores
//! the original value only after it was ever written — `wroteCheckedValue`;
//! `busy` swaps the hint for "Busy" and restores the original otherwise;
//! `hidden` restores `accessibilityElementsHidden`.
//! On `AppKit`, `expanded` is applied only when present and `hidden`
//! restores `isAccessibilityElement`.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use cocoa_ui::accessibility::{self as a11y, Role};
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;
use cocoa_ui::{PlatformView, Rect, response, view};
use nami::Computed;
use objc2_foundation::NSString;
use waterui_accessibility::{
    AccessibilityAdjustable, AccessibilityChecked, AccessibilityChildren,
    AccessibilityCustomActions, AccessibilityGeometry, AccessibilityHidden, AccessibilityHint,
    AccessibilityLabel, AccessibilityRole, AccessibilityState, AccessibilityStateSignal,
    AccessibilityValue,
};
use waterui_core::handler::ReportExtraction as _;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};
use waterui_core::metadata::MetadataKey;
use waterui_core::{Environment, Metadata, Signal, SignalExt};
use waterui_interaction::interaction::Selected;

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::proposal::{self, in_view};

#[cfg(target_os = "macos")]
pub mod target {
    use cocoa_ui::PlatformView;
    use cocoa_ui::objc2_app_kit::NSAccessibility;
    use objc2::runtime::ProtocolObject;

    /// The platform accessibility protocol face of a view — the receiver
    /// every `AppKit` accessibility call takes.
    pub fn of(view: &PlatformView) -> &ProtocolObject<dyn NSAccessibility> {
        ProtocolObject::from_ref(view)
    }
}

#[cfg(target_os = "ios")]
pub mod target {
    use cocoa_ui::PlatformView;

    /// On `UIKit` the accessibility receiver is the view itself.
    pub const fn of(view: &PlatformView) -> &PlatformView {
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
        AccessibilityRole::Heading(level) => Role::Heading(*level),
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

/// Composes an explicit accessibility hint with the `busy` state flag so
/// setting either one never clobbers the other.
fn compose_hint(hint: Option<&str>, busy: bool) -> Option<String> {
    let hint = hint.filter(|text| !text.is_empty());
    match (busy, hint) {
        (true, Some(hint)) => Some(format!("Busy, {hint}")),
        (true, None) => Some(String::from("Busy")),
        (false, Some(hint)) => Some(String::from(hint)),
        (false, None) => None,
    }
}

/// Applies an `AccessibilityState` to the view — `applyState`.
#[cfg(target_os = "ios")]
#[allow(clippy::too_many_arguments)]
fn apply_state(
    target: &PlatformView,
    state: &AccessibilityState,
    checked_value_written: bool,
    original_value: Option<&NSString>,
    original_hint: Option<&NSString>,
    manage_hint: bool,
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
        manage_hint,
    );
}

/// Applies an `AccessibilityState` to the view — `applyState`.
#[cfg(target_os = "macos")]
fn apply_state(
    target: &PlatformView,
    state: &AccessibilityState,
    original_help: Option<&NSString>,
    manage_help: bool,
) {
    a11y::apply_state(
        target::of(target),
        state.is_disabled(),
        state.is_selected(),
        state.checked_state().map(checked),
        state.expanded_state(),
        state.is_busy(),
        original_help,
        manage_help,
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
fn apply_hidden(target: &PlatformView, hidden: bool, original_hidden: bool) {
    a11y::set_hidden(target::of(target), hidden || original_hidden);
}

#[cfg(target_os = "ios")]
fn read_original_hidden(target: &PlatformView) -> bool {
    a11y::hidden(target::of(target))
}

#[cfg(target_os = "macos")]
fn read_original_hidden(target: &PlatformView) -> bool {
    !a11y::is_element(target::of(target))
}

/// Shared bind for `AccessibilityState` (constant) and
/// `AccessibilityStateSignal` (reactive).
///
/// The scope merges `local` over the state an enclosing scope installed and
/// renders its content under the merged scope, so the innermost scope on a
/// leaf holds every enclosing scope's fields and is the one that applies:
/// an enclosing scope around the same leaf applies nothing over it
/// ([`NativeLeaf::claim_accessibility_state`]). Each delivery applies the
/// whole merged state plus the `hidden` channel's restore semantics.
fn render_state(
    content: Held,
    local: Computed<AccessibilityState>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mut env = ctx.env().clone();
    AccessibilityStateSignal::install(&mut env, local);
    let merged = env
        .get::<AccessibilityStateSignal>()
        .expect("install inserts the merged state scope")
        .state()
        .clone();
    let mut leaf = ctx.with_env(&env).realize(content);
    let target = view::retain_base(leaf.view());
    let has_scoped_hint = env.get::<AccessibilityHint>().cloned();
    if let Some(hint) = has_scoped_hint
        && leaf.claim_accessibility_hint()
    {
        bind_composed_hint(&mut leaf, target.clone(), &hint, Some(&merged), &env);
    }
    leaf.keep(env);
    if !leaf.claim_accessibility_state() {
        return leaf;
    }
    let manage_hint = leaf.claim_accessibility_hint();
    leaf.keep(target.clone());
    #[cfg(target_os = "ios")]
    {
        use core::cell::Cell;
        let wrote_checked = Cell::new(false);
        let original_value = a11y::value(&target);
        let original_hint = a11y::hint(&target);
        let original_hidden = read_original_hidden(&target);
        leaf.bind(&merged, move |state| {
            apply_state(
                &target,
                &state,
                wrote_checked.get(),
                original_value.as_deref(),
                original_hint.as_deref(),
                manage_hint,
            );
            if state.checked_state().is_some() {
                wrote_checked.set(true);
            }
            apply_hidden(&target, state.is_hidden(), original_hidden);
        });
    }
    #[cfg(target_os = "macos")]
    {
        let original_help = a11y::hint(target::of(&target));
        let original_hidden = read_original_hidden(&target);
        leaf.bind(&merged, move |state| {
            apply_state(&target, &state, original_help.as_deref(), manage_hint);
            apply_hidden(&target, state.is_hidden(), original_hidden);
        });
    }
    leaf
}

/// Binds an explicit `AccessibilityHint` (optionally composed with a scoped
/// `AccessibilityStateSignal`'s `busy` flag) onto `target`.
fn bind_composed_hint(
    leaf: &mut NativeLeaf,
    target: cocoa_ui::Retained<PlatformView>,
    hint: &AccessibilityHint,
    state: Option<&Computed<AccessibilityState>>,
    env: &Environment,
) {
    leaf.keep(target.clone());
    let hint_signal = hint.resolve(env);
    if let Some(state_signal) = state {
        let combined = nami::zip::zip(hint_signal, state_signal.clone());
        leaf.bind(&combined, move |(hint_text, state)| {
            let composed = compose_hint(Some(hint_text.as_str()), state.is_busy());
            let ns = composed.as_deref().map(NSString::from_str);
            a11y::set_hint(target::of(&target), ns.as_deref());
        });
    } else {
        leaf.bind(&hint_signal, move |hint_text| {
            let composed = compose_hint(Some(hint_text.as_str()), false);
            let ns = composed.as_deref().map(NSString::from_str);
            a11y::set_hint(target::of(&target), ns.as_deref());
        });
    }
}

fn scope_env<K: MetadataKey + Clone + 'static>(ctx: &RenderContext<'_>, value: &K) -> Environment {
    let mut env = ctx.env().clone();
    env.insert(value.clone());
    env
}

/// Strips nearest-consumer accessibility metadata at container/control child
/// boundaries so an enclosing `.a11y_label(...)` / `.a11y_role(...)` /
/// `.a11y_hidden(...)` never leaks into descendant views.
pub fn child_env(env: &Environment) -> Environment {
    let mut child = env.clone();
    child.remove::<AccessibilityLabel>();
    child.remove::<AccessibilityValue>();
    child.remove::<AccessibilityHint>();
    child.remove::<AccessibilityCustomActions>();
    child.remove::<AccessibilityAdjustable>();
    child.remove::<AccessibilityRole>();
    child.remove::<AccessibilityHidden>();
    child.remove::<AccessibilityChildren>();
    child.remove::<AccessibilityGeometry>();
    child
}

/// Registers the constant and reactive state handlers on the shared bind.
pub fn realize_accessibility_state_metadata(
    metadata: Metadata<AccessibilityState, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mut env = ctx.env().clone();
    if metadata.value.is_hidden() {
        env.insert(AccessibilityHidden::new(true));
    }
    render_state(
        metadata.content,
        Computed::constant(metadata.value),
        &ctx.with_env(&env),
    )
}

pub fn realize_accessibility_state_signal_metadata(
    metadata: Metadata<AccessibilityStateSignal, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    render_state(metadata.content, metadata.value.state().clone(), ctx)
}

/// The accessibility metadata realizations.
pub fn realize_accessibility_label_metadata(
    metadata: Metadata<AccessibilityLabel, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    let target = view::retain_base(leaf.view());
    leaf.keep(target.clone());
    let signal = metadata.value.resolve(&scoped);
    leaf.bind(&signal, move |label| {
        a11y::set_label(target::of(&target), &NSString::from_str(&label));
    });
    leaf.keep(scoped);
    leaf
}

pub fn realize_accessibility_value_metadata(
    metadata: Metadata<AccessibilityValue, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    let target = view::retain_base(leaf.view());
    leaf.keep(target.clone());
    let signal = metadata.value.resolve(&scoped);
    leaf.bind(&signal, move |value| {
        a11y::set_value(target::of(&target), &NSString::from_str(&value));
    });
    leaf.keep(scoped);
    leaf
}

pub fn realize_accessibility_hint_metadata(
    metadata: Metadata<AccessibilityHint, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    if leaf.claim_accessibility_hint() {
        let target = view::retain_base(leaf.view());
        let state_signal = scoped
            .get::<AccessibilityStateSignal>()
            .map(|scope| scope.state().clone());
        bind_composed_hint(
            &mut leaf,
            target,
            &metadata.value,
            state_signal.as_ref(),
            &scoped,
        );
    }
    leaf.keep(scoped);
    leaf
}

pub fn realize_accessibility_custom_actions_metadata(
    metadata: Metadata<AccessibilityCustomActions, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mut scoped = ctx.env().clone();
    AccessibilityCustomActions::install(&mut scoped, &metadata.value);
    let merged = scoped
        .get::<AccessibilityCustomActions>()
        .expect("install inserts merged custom actions")
        .clone();
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    let handler_env = leaf.handler_env(&scoped);
    if leaf.claim_accessibility_custom_actions() {
        let target = view::retain_base(leaf.view());
        leaf.keep(target.clone());
        let entries: Rc<Vec<_>> = Rc::new(
            merged
                .actions()
                .iter()
                .map(|action| {
                    let name_signal = action.resolve_name(&handler_env);
                    let action = action.clone();
                    let action_env = handler_env.clone();
                    let callback: Rc<dyn Fn() -> bool> =
                        Rc::new(move || action.invoke(&action_env).report_extraction().is_some());
                    (name_signal, callback)
                })
                .collect(),
        );
        let rebuild: Rc<dyn Fn()> = {
            let entries = Rc::clone(&entries);
            Rc::new(move || {
                let resolved: Vec<(String, Rc<dyn Fn() -> bool>)> = entries
                    .iter()
                    .map(|(signal, callback)| {
                        (
                            String::from(signal.snapshot().as_str()),
                            Rc::clone(callback),
                        )
                    })
                    .collect();
                a11y::set_custom_actions(target::of(&target), &resolved);
            })
        };
        rebuild();
        for (signal, _) in entries.iter() {
            let rebuild = Rc::clone(&rebuild);
            leaf.watch(signal, move |_| rebuild());
        }
    }
    leaf.set_handler_env(handler_env);
    leaf.keep(scoped);
    leaf
}

/// Transparent wrapper state for `.a11y_adjustable(...)` when its wrapped leaf
/// is not already a [`HostView`].
struct AdjustableHostState {
    child: Mounted,
}

impl core::fmt::Debug for AdjustableHostState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AdjustableHostState")
            .finish_non_exhaustive()
    }
}

struct AdjustableHostSubView {
    state: Rc<RefCell<AdjustableHostState>>,
}

impl core::fmt::Debug for AdjustableHostSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AdjustableHostSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for AdjustableHostSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.state
            .borrow()
            .child
            .layout()
            .resolve_extent(bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.borrow().child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.borrow().child.layout().is_empty()
    }
}

fn install_adjustable_on_host(
    host: &HostView,
    adjustable: AccessibilityAdjustable,
    action_env: Environment,
) {
    let inc_action = adjustable.clone();
    let inc_env = action_env.clone();
    let dec_action = adjustable;
    let dec_env = action_env;

    #[cfg(target_os = "macos")]
    {
        use cocoa_ui::objc2_app_kit::NSAccessibility as _;
        host.set_accessibility_adjustable_handlers(
            move |_| inc_action.increment(&inc_env).report_extraction().is_some(),
            move |_| dec_action.decrement(&dec_env).report_extraction().is_some(),
        );
        let host_view: &PlatformView = host;
        target::of(host_view).setAccessibilityElement(true);
    }
    #[cfg(target_os = "ios")]
    {
        host.set_accessibility_adjustable_handlers(
            move |_| {
                inc_action.increment(&inc_env).report_extraction();
            },
            move |_| {
                dec_action.decrement(&dec_env).report_extraction();
            },
        );
        a11y::set_adjustable(host, true);
    }
}

pub fn realize_accessibility_adjustable_metadata(
    metadata: Metadata<AccessibilityAdjustable, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    let handler_env = leaf.handler_env(&scoped);
    if let Some(host) = leaf.view().downcast_ref::<HostView>() {
        install_adjustable_on_host(host, metadata.value, handler_env.clone());
        leaf.set_handler_env(handler_env);
        leaf.keep(scoped);
        return leaf;
    }

    // Wrap non-HostView leaves (for example a bare `Label`) in a transparent
    // `HostView` that owns the increment/decrement overrides and forwards
    // layout, primary content, and unset accessibility properties to the child.
    let host = HostView::new(ctx.mtm(), Rect::ZERO);
    let host_view: &PlatformView = &host;
    let clip_shape = leaf.clip_shape_handle();
    let mounted = leaf.mount(host_view);
    view::set_translates_autoresizing(mounted.view(), true);
    let state = Rc::new(RefCell::new(AdjustableHostState { child: mounted }));

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host| {
            let bounds = view::bounds(host);
            let child = state.borrow();
            view::set_frame(child.child.view(), bounds);
        }
    });
    crate::primary_content::forward(&host, state.borrow().child.view());
    host.set_delegates_accessibility_to_primary(true);
    #[cfg(target_os = "ios")]
    host.set_combines_accessibility_children(true);

    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.borrow().child.view(), selected);
        }
    });

    install_adjustable_on_host(&host, metadata.value, handler_env.clone());

    let mut wrapper = NativeLeaf::new(
        host_view,
        AdjustableHostSubView {
            state: Rc::clone(&state),
        },
    );
    wrapper.set_clip_shape_handle(clip_shape);
    wrapper.set_handler_env(handler_env);
    wrapper.keep(sink_guard);
    wrapper.keep(state);
    wrapper.keep(scoped);
    wrapper
}

pub fn realize_accessibility_role_metadata(
    metadata: Metadata<AccessibilityRole, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    let target = view::retain_base(leaf.view());
    a11y::set_role(target::of(&target), role(&metadata.value));
    leaf.keep(target);
    leaf.keep(scoped);
    leaf
}

pub fn realize_accessibility_geometry_metadata(
    metadata: Metadata<AccessibilityGeometry, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    name_geometry(&leaf, metadata.value);
    leaf.keep(scoped);
    leaf
}

pub fn realize_accessibility_hidden_metadata(
    metadata: Metadata<AccessibilityHidden, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    let target = view::retain_base(leaf.view());
    let original_hidden = read_original_hidden(&target);
    leaf.keep(target.clone());
    leaf.bind(metadata.value.signal(), move |hidden| {
        apply_hidden(&target, hidden, original_hidden);
    });
    leaf.keep(scoped);
    leaf
}

pub fn realize_accessibility_children_metadata(
    metadata: Metadata<AccessibilityChildren, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let scoped = scope_env(ctx, &metadata.value);
    let mut leaf = ctx.with_env(&scoped).realize(metadata.content);
    let target = view::retain_base(leaf.view());
    if metadata.value.excludes_descendants() {
        #[cfg(target_os = "macos")]
        a11y::exclude_children(target::of(&target));
        #[cfg(target_os = "ios")]
        {
            use cocoa_ui::objc2_ui_kit::NSObjectUIAccessibility as _;
            target.setIsAccessibilityElement(true, ctx.mtm());
            for subview in view::subviews(&target) {
                view::hide_from_accessibility(&subview);
            }
        }
    }
    leaf.keep(target);
    leaf.keep(scoped);
    leaf
}

/// `.selected(...)`: the control it modifies is announced as selected while
/// the signal reads `true`, through the same state scope as
/// [`AccessibilityState`].
pub fn realize_selected_metadata(
    metadata: Metadata<Selected, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let Selected(selected) = metadata.value;
    let state = SignalExt::map(&selected, |selected| {
        AccessibilityState::new().selected(selected)
    });
    render_state(metadata.content, Computed::new(state), ctx)
}

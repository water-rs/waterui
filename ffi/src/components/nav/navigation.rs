use alloc::boxed::Box;
use nami::SignalExt as _;

use crate::action::WuiAction;
use crate::array::WuiArray;
use crate::closure::{ForeignCallbackContext, WuiFn};
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiAnyView, WuiEnv};
use waterui_core::Str;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_graphics::color::ResolvedColor;
use waterui_navigation::tab::{NativeTabStyle, Tab, Tabs};
use waterui_navigation::{
    Bar, ColumnWidth, CustomNavigationController, NativeNavigationSplitStyle,
    NativeNavigationTransition, NavigationController, NavigationDestinationState, NavigationSearch,
    NavigationSplitColumnVisibility, NavigationSplitLayout, NavigationStack,
    NavigationTitleDisplayMode, NavigationToolbarItem, NavigationToolbarPlacement,
    NavigationTransaction, NavigationView, split::NavigationSplitDetailBuilder,
};
use waterui_text::styled::StyledStr;

into_ffi! {
    NavigationView,
    pub struct WuiNavigationView {
        bar: WuiBar,
        content: *mut WuiAnyView,
        state: WuiNavigationDestinationState,
    }
}

#[repr(C)]
pub struct WuiNavigationDestinationState {
    pub pop_enabled: *mut WuiComputed<bool>,
    pub pop_attempted: *mut WuiAction,
    pub appear: *mut WuiAction,
    pub disappear: *mut WuiAction,
    pub pop: *mut WuiAction,
}

impl IntoFFI for NavigationDestinationState {
    type FFI = WuiNavigationDestinationState;

    fn into_ffi(self) -> Self::FFI {
        WuiNavigationDestinationState {
            pop_enabled: self.pop_enabled.into_ffi(),
            pop_attempted: self
                .pop_attempted
                .map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
            appear: self.appear.map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
            disappear: self
                .disappear
                .map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
            pop: self.pop.map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
        }
    }
}

pub struct WuiNavigationLink {
    pub label: *mut WuiAnyView,
    pub destination: *mut WuiFn<*mut WuiAnyView>,
}

#[repr(C)]
pub struct WuiNavigationSearch {
    pub text: *mut WuiBinding<Str>,
    pub prompt: *mut WuiComputed<StyledStr>,
}

impl IntoFFI for NavigationSearch {
    type FFI = WuiNavigationSearch;

    fn into_ffi(self) -> Self::FFI {
        WuiNavigationSearch {
            text: self.text.into_ffi(),
            prompt: self.prompt.into_config_without_env().content.into_ffi(),
        }
    }
}

#[repr(C)]
pub struct WuiOptionalNavigationSearch {
    pub has_value: bool,
    pub value: WuiNavigationSearch,
}

impl IntoFFI for Option<NavigationSearch> {
    type FFI = WuiOptionalNavigationSearch;

    fn into_ffi(self) -> Self::FFI {
        self.map_or_else(
            || WuiOptionalNavigationSearch {
                has_value: false,
                value: WuiNavigationSearch {
                    text: core::ptr::null_mut(),
                    prompt: core::ptr::null_mut(),
                },
            },
            |search| WuiOptionalNavigationSearch {
                has_value: true,
                value: search.into_ffi(),
            },
        )
    }
}

/// The display mode for the navigation bar title (FFI-compatible).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiNavigationTitleDisplayMode {
    /// System decides based on context.
    Automatic = 0,
    /// Always use inline (small) title.
    Inline = 1,
    /// Always use large title.
    Large = 2,
}

impl IntoFFI for NavigationTitleDisplayMode {
    type FFI = WuiNavigationTitleDisplayMode;
    fn into_ffi(self) -> Self::FFI {
        match self {
            NavigationTitleDisplayMode::Automatic => WuiNavigationTitleDisplayMode::Automatic,
            NavigationTitleDisplayMode::Inline => WuiNavigationTitleDisplayMode::Inline,
            NavigationTitleDisplayMode::Large => WuiNavigationTitleDisplayMode::Large,
        }
    }
}

#[repr(C)]
pub struct WuiBar {
    pub title: *mut WuiAnyView,
    pub subtitle: *mut WuiAnyView,
    pub toolbar: WuiArray<WuiNavigationToolbarItem>,
    pub search: WuiOptionalNavigationSearch,
    pub color: *mut WuiComputed<ResolvedColor>,
    pub hidden: *mut WuiComputed<bool>,
    pub display_mode: WuiNavigationTitleDisplayMode,
}

impl IntoFFI for Bar {
    type FFI = WuiBar;

    fn into_ffi(self) -> Self::FFI {
        let color = match (self.color, self.resolved_color) {
            (None, _) => core::ptr::null_mut(),
            (Some(_), Some(color)) => color.into_ffi(),
            (Some(_), None) => {
                panic!("NavigationView must resolve its bar color with an Environment before FFI")
            }
        };
        WuiBar {
            title: self.title.into_ffi(),
            subtitle: self.subtitle.into_ffi(),
            toolbar: self.toolbar.items.into_ffi(),
            search: self.search.into_ffi(),
            color,
            hidden: self.hidden.into_ffi(),
            display_mode: self.display_mode.into_ffi(),
        }
    }
}

/// Semantic native toolbar placement.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiNavigationToolbarPlacement {
    Principal = 0,
    PrimaryAction = 1,
    SecondaryAction = 2,
    Confirmation = 3,
    Cancellation = 4,
    BottomBar = 5,
    Status = 6,
    TopBarLeading = 7,
    TopBarTrailing = 8,
}

impl From<NavigationToolbarPlacement> for WuiNavigationToolbarPlacement {
    fn from(value: NavigationToolbarPlacement) -> Self {
        match value {
            NavigationToolbarPlacement::Principal => Self::Principal,
            NavigationToolbarPlacement::PrimaryAction => Self::PrimaryAction,
            NavigationToolbarPlacement::SecondaryAction => Self::SecondaryAction,
            NavigationToolbarPlacement::Confirmation => Self::Confirmation,
            NavigationToolbarPlacement::Cancellation => Self::Cancellation,
            NavigationToolbarPlacement::BottomBar => Self::BottomBar,
            NavigationToolbarPlacement::Status => Self::Status,
            NavigationToolbarPlacement::TopBarLeading => Self::TopBarLeading,
            NavigationToolbarPlacement::TopBarTrailing => Self::TopBarTrailing,
        }
    }
}

#[repr(C)]
pub struct WuiNavigationToolbarItem {
    pub placement: WuiNavigationToolbarPlacement,
    pub content: *mut WuiAnyView,
}

impl IntoFFI for NavigationToolbarItem {
    type FFI = WuiNavigationToolbarItem;

    fn into_ffi(self) -> Self::FFI {
        WuiNavigationToolbarItem {
            placement: self.placement.into(),
            content: self.content.into_ffi(),
        }
    }
}

// FFI view bindings for navigation components
ffi_view!(NavigationView, WuiNavigationView, navigation_view);

/// FFI struct for NavigationStack<(),()>
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiNavigationTransitionKind {
    Automatic = 0,
    Fade = 1,
    Zoom = 2,
    None = 3,
    Custom = 4,
}

#[repr(C)]
pub struct WuiNavigationTransition {
    pub kind: WuiNavigationTransitionKind,
    pub source_id: i32,
}

impl IntoFFI for NativeNavigationTransition {
    type FFI = WuiNavigationTransition;

    fn into_ffi(self) -> Self::FFI {
        match self {
            NativeNavigationTransition::Automatic => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Automatic,
                source_id: 0,
            },
            NativeNavigationTransition::Fade => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Fade,
                source_id: 0,
            },
            NativeNavigationTransition::Zoom(source) => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Zoom,
                source_id: i32::from(source),
            },
            NativeNavigationTransition::None => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::None,
                source_id: 0,
            },
            NativeNavigationTransition::Custom => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Custom,
                source_id: 0,
            },
        }
    }
}

/// FFI struct for NavigationStack<(),()>
#[repr(C)]
pub struct WuiNavigationStack {
    /// The root view of the navigation stack.
    pub root: *mut WuiAnyView,
    /// Transition style used for push/pop operations.
    pub transition: WuiNavigationTransition,
}

impl IntoFFI for NavigationStack<(), ()> {
    type FFI = WuiNavigationStack;
    fn into_ffi(self) -> Self::FFI {
        let transition = self.transition_style().native().into_ffi();
        WuiNavigationStack {
            root: self.into_inner().into_ffi(),
            transition,
        }
    }
}

ffi_view!(NavigationStack<(),()>, WuiNavigationStack, navigation_stack);

unsafe fn resolve_navigation_stack_root(
    root: *mut WuiAnyView,
    env: *const WuiEnv,
) -> *mut WuiAnyView {
    let root: waterui::AnyView = unsafe { crate::IntoRust::into_rust(root) };
    let env = unsafe { crate::borrow_ffi(env) };
    let root = waterui_navigation::resolve_navigation_root(root, &env.0);
    waterui::AnyView::new(waterui_core::Native::new(root)).into_ffi()
}

/// Resolves a stack root after the native backend installs its controller.
///
/// # Safety
///
/// - `root` must be the owning root pointer returned by
///   `waterui_force_as_navigation_stack` and is consumed exactly once.
/// - `env` must be the live controller-scoped environment for this stack.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_stack_root(
    root: *mut WuiAnyView,
    env: *const WuiEnv,
) -> *mut WuiAnyView {
    unsafe { resolve_navigation_stack_root(root, env) }
}

/// Resolves an Android stack root after the backend controller is installed.
///
/// # Safety
///
/// `root_ptr` must own an unresolved stack root and `env_ptr` must point to its
/// live controller-scoped environment.
#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_navigationStackRoot<
    'local,
>(
    _env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    root_ptr: crate::jni::jlong,
    env_ptr: crate::jni::jlong,
) -> crate::jni::jlong {
    use crate::jni::convert::{jlong_to_ptr, jlong_to_ptr_mut};

    let root = unsafe { jlong_to_ptr_mut(root_ptr) };
    let env = unsafe { jlong_to_ptr(env_ptr) };
    let resolved = unsafe { resolve_navigation_stack_root(root, env) };
    resolved as crate::jni::jlong
}

#[repr(C)]
pub struct WuiNavigationSplitLayout {
    /// Sidebar content.
    pub sidebar: *mut WuiAnyView,
    /// Placeholder content for empty regular-width selection.
    pub placeholder: *mut WuiAnyView,
    /// Primary selection encoded as i32 (0 means no selection).
    pub primary_selection: *mut WuiBinding<i32>,
    /// Optional resolver for the middle column in a three-column split.
    pub content: *mut WuiNavigationSplitDetail,
    /// Optional secondary selection encoded as i32 (0 means no selection).
    pub secondary_selection: *mut WuiBinding<i32>,
    /// Resolver handle for building detail content from a selected id.
    pub detail: *mut WuiNavigationSplitDetail,
    /// Reactive requested column visibility.
    pub column_visibility: *mut WuiComputed<i32>,
    /// Native resizable sidebar width constraints.
    pub sidebar_width: WuiNavigationColumnWidth,
    /// Native split style.
    pub style: WuiNavigationSplitStyle,
}

#[repr(C)]
pub struct WuiNavigationColumnWidth {
    pub min: f32,
    pub ideal: f32,
    pub max: f32,
}

impl From<ColumnWidth> for WuiNavigationColumnWidth {
    fn from(value: ColumnWidth) -> Self {
        Self {
            min: value.min(),
            ideal: value.ideal(),
            max: value.max(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiNavigationSplitStyle {
    Automatic = 0,
    Balanced = 1,
    ProminentDetail = 2,
}

impl From<NativeNavigationSplitStyle> for WuiNavigationSplitStyle {
    fn from(value: NativeNavigationSplitStyle) -> Self {
        match value {
            NativeNavigationSplitStyle::Automatic => Self::Automatic,
            NativeNavigationSplitStyle::Balanced => Self::Balanced,
            NativeNavigationSplitStyle::ProminentDetail => Self::ProminentDetail,
        }
    }
}

#[repr(C)]
pub struct WuiNavigationSplitDetail {
    _private: [u8; 0],
}

impl crate::IntoFFI for NavigationSplitDetailBuilder {
    type FFI = *mut WuiNavigationSplitDetail;

    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(self)) as *mut WuiNavigationSplitDetail
    }
}

impl crate::IntoRust for *mut WuiNavigationSplitDetail {
    type Rust = NavigationSplitDetailBuilder;

    unsafe fn into_rust(self) -> Self::Rust {
        unsafe { *Box::from_raw(self as *mut NavigationSplitDetailBuilder) }
    }
}

/// Releases a navigation split-detail handle.
///
/// # Safety
///
/// `value` must be a valid, owning `WuiNavigationSplitDetail` handle that has
/// not already been dropped; it must not be used after this call.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_split_navigation_detail(
    value: *mut WuiNavigationSplitDetail,
) {
    let _ = unsafe { crate::IntoRust::into_rust(value) };
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
/// Releases an Android split-detail navigation handle.
///
/// # Safety
///
/// `ptr` must be a valid owning `WuiNavigationSplitDetail` handle and must not
/// be used after this call.
pub unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropSplitNavigationDetail<
    'local,
>(
    _env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    ptr: crate::jni::jlong,
) {
    use crate::jni::convert::jlong_to_ptr_mut;
    let ptr: *mut WuiNavigationSplitDetail = unsafe { jlong_to_ptr_mut(ptr) };
    let _ = unsafe { crate::IntoRust::into_rust(ptr) };
}

/// Resolves the active detail navigation view for a selected split identifier.
///
/// # Safety
///
/// - `handler` must be a valid pointer to a `WuiNavigationSplitDetail`.
/// - `selected` must encode a valid non-zero split selection id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_split_navigation_detail_content(
    handler: *mut WuiNavigationSplitDetail,
    selected: crate::id::WuiId,
    env: *const WuiEnv,
) -> WuiNavigationView {
    let handler = unsafe { crate::borrow_ffi(handler as *const NavigationSplitDetailBuilder) };
    let selected = unsafe { crate::IntoRust::into_rust(selected) };
    let env = unsafe { crate::borrow_ffi(env) };
    let mut view = handler.build(selected);
    view.resolve_native_fields(&env.0);
    IntoFFI::into_ffi(view)
}

impl IntoFFI for NavigationSplitLayout {
    type FFI = WuiNavigationSplitLayout;

    fn into_ffi(self) -> Self::FFI {
        let (
            sidebar,
            placeholder,
            primary_selection,
            content,
            secondary_selection,
            detail,
            column_visibility,
            sidebar_width,
            style,
        ) = self.into_parts();
        let primary_selection = optional_id_binding(&primary_selection).into_ffi();
        let secondary_selection = secondary_selection
            .as_ref()
            .map_or(core::ptr::null_mut(), |selection| {
                optional_id_binding(selection).into_ffi()
            });
        let column_visibility = column_visibility
            .map(|visibility| match visibility {
                NavigationSplitColumnVisibility::Automatic => 0,
                NavigationSplitColumnVisibility::All => 1,
                NavigationSplitColumnVisibility::DoubleColumn => 2,
                NavigationSplitColumnVisibility::DetailOnly => 3,
            })
            .computed()
            .into_ffi();

        WuiNavigationSplitLayout {
            sidebar: sidebar.build().into_ffi(),
            placeholder: placeholder.build().into_ffi(),
            primary_selection,
            content: content.map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
            secondary_selection,
            detail: detail.into_ffi(),
            column_visibility,
            sidebar_width: sidebar_width.into(),
            style: style.into(),
        }
    }
}

fn optional_id_binding(selection: &nami::Binding<Option<Id>>) -> WuiBinding<i32> {
    WuiBinding(nami::Binding::mapping(
        selection,
        |value| value.map(i32::from).unwrap_or(0),
        |binding, value| {
            binding.set(core::num::NonZeroI32::new(value).map(waterui_core::id::Id::from));
        },
    ))
}

ffi_view!(
    NavigationSplitLayout,
    WuiNavigationSplitLayout,
    split_navigation_container
);

/// Native adaptive tab style.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiTabStyle {
    Automatic = 0,
    TabBar = 1,
    Sidebar = 2,
}

impl From<NativeTabStyle> for WuiTabStyle {
    fn from(style: NativeTabStyle) -> Self {
        match style {
            NativeTabStyle::Automatic => Self::Automatic,
            NativeTabStyle::TabBar => Self::TabBar,
            NativeTabStyle::Sidebar => Self::Sidebar,
        }
    }
}

#[repr(C)]
pub struct WuiTabs {
    /// The currently selected tab identifier.
    pub selection: *mut WuiBinding<Id>,

    /// The collection of tabs to display.
    pub tabs: WuiArray<WuiTab>,

    /// Native adaptive tab style.
    pub style: WuiTabStyle,
}

opaque!(WuiTabContent, AnyViewBuilder<NavigationView>, tab_content);

#[repr(C)]
pub struct WuiTab {
    /// The unique identifier for the tab (raw u64 for FFI compatibility).
    pub id: u64,

    /// Pointer to the tab's label view.
    pub label: *mut WuiAnyView,

    /// Pointer to the tab's content view.
    pub content: *mut WuiTabContent,

    /// Optional reactive badge count.
    pub badge: *mut WuiComputed<i32>,

    /// Reactive enabled state.
    pub enabled: *mut WuiComputed<bool>,
}

/// Creates a navigation view from tab content.
///
/// # Safety
///
/// This function is unsafe because:
/// - `handler` must be a valid, non-null pointer to a `WuiTabContent`
/// - Both pointers must remain valid for the duration of the function call
/// - The caller must ensure proper memory management of the returned view
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_tab_content(
    handler: *mut WuiTabContent,
    env: *const WuiEnv,
) -> WuiNavigationView {
    unsafe {
        let handler = crate::borrow_ffi(handler);
        let env = crate::borrow_ffi(env);
        let mut view = handler.build();
        view.resolve_native_fields(&env.0);
        IntoFFI::into_ffi(view)
    }
}

impl IntoFFI for Tab<Id> {
    type FFI = WuiTab;
    fn into_ffi(self) -> Self::FFI {
        let id_i32 = i32::from(self.id);
        let id = u64::try_from(id_i32)
            .expect("tab id must be positive when converting to FFI u64 identifier");
        WuiTab {
            id,
            label: self.label.into_ffi(),
            content: self.content.into_ffi(),
            badge: self.badge.map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
            enabled: self.enabled.into_ffi(),
        }
    }
}

impl IntoFFI for Tabs {
    type FFI = WuiTabs;
    fn into_ffi(self) -> Self::FFI {
        WuiTabs {
            selection: self.selection.into_ffi(),
            tabs: self.tabs.into_ffi(),
            style: self.style.into(),
        }
    }
}

// FFI view binding for Tabs
ffi_view!(Tabs, WuiTabs, tabs);

// =============================================================================
// Navigation Controller FFI
// =============================================================================

struct ForeignNavigationController {
    context: ForeignCallbackContext,
    apply: unsafe extern "C" fn(*mut (), WuiNavigationTransaction),
}

/// One atomic navigation stack mutation passed to a native backend.
#[repr(C)]
pub struct WuiNavigationTransaction {
    pub id: u64,
    pub retained_prefix: usize,
    pub removed: usize,
    pub inserted: WuiArray<WuiNavigationView>,
}

// SAFETY: ForeignNavigationController is only accessed from the UI thread.
unsafe impl Send for ForeignNavigationController {}

impl CustomNavigationController for ForeignNavigationController {
    fn apply(&mut self, transaction: NavigationTransaction) {
        let (id, retained_prefix, removed, inserted) = transaction.into_parts();
        let inserted = inserted
            .into_iter()
            .map(|builder| builder.build())
            .map(IntoFFI::into_ffi)
            .collect::<alloc::vec::Vec<_>>();
        unsafe {
            (self.apply)(
                self.context.data(),
                WuiNavigationTransaction {
                    id,
                    retained_prefix,
                    removed,
                    inserted: WuiArray::new(inserted),
                },
            )
        }
    }
}

/// Installs native navigation callbacks into an environment.
///
/// # Safety
///
/// - `env` must be a valid, exclusively borrowed environment pointer.
/// - `data` must remain valid until `drop_context` releases it.
/// - All callback function pointers must be valid and safe to call
/// - `drop_context` must release `data` exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_navigation_controller(
    env: *mut WuiEnv,
    context: *mut (),
    apply: unsafe extern "C" fn(*mut (), WuiNavigationTransaction),
    drop_context: unsafe extern "C" fn(*mut ()),
) {
    let env = unsafe { crate::borrow_ffi_mut(env) };
    let controller = ForeignNavigationController {
        context: unsafe { ForeignCallbackContext::new(context, drop_context) },
        apply,
    };
    env.insert(NavigationController::new(controller));
}

/// Checks if a navigation controller is installed in the environment.
///
/// Returns true if a NavigationController is available, false otherwise.
/// Use this to determine whether to show a back button in navigation views.
///
/// # Safety
///
/// - `env` must be a valid pointer to a `WuiEnv`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_has_navigation_controller(env: *const WuiEnv) -> bool {
    // SAFETY: Caller guarantees pointer is valid
    unsafe {
        let env = crate::borrow_ffi(env);
        env.get::<NavigationController>().is_some()
    }
}

/// Requests a user-initiated pop from the Rust navigation state.
///
/// If no NavigationController is installed in the environment, this function does nothing.
///
/// # Safety
///
/// - `env` must be a valid pointer to a `WuiEnv`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_pop(env: *const WuiEnv) {
    // SAFETY: Caller guarantees pointer is valid
    unsafe {
        let env = crate::borrow_ffi(env);
        if let Some(controller) = env.get::<NavigationController>() {
            controller.request_pop(1);
        }
    }
}

/// Commits a pop already completed interactively by the native container.
///
/// # Safety
///
/// `env` must point to a live environment containing the controller for the
/// native stack that completed the pop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_complete_native_pop(env: *const WuiEnv, count: usize) {
    let env = unsafe { crate::borrow_ffi(env) };
    let controller = env
        .get::<NavigationController>()
        .expect("native navigation pop completed without an installed controller");
    controller.complete_native_pop(count);
}

/// Acknowledges successful completion of a native navigation transaction.
///
/// Stale acknowledgements are ignored because a newer transaction owns the
/// native stack projection.
///
/// # Safety
///
/// `env` must point to the live environment for the reporting stack.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_transition_completed(
    env: *const WuiEnv,
    id: u64,
) -> bool {
    let env = unsafe { crate::borrow_ffi(env) };
    let controller = env
        .get::<NavigationController>()
        .expect("navigation transition completed without an installed controller");
    controller.transition_completed(id)
}

/// Acknowledges cancellation of a native navigation transaction.
///
/// # Safety
///
/// `env` must point to the live environment for the reporting stack.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_transition_cancelled(
    env: *const WuiEnv,
    id: u64,
) -> bool {
    let env = unsafe { crate::borrow_ffi(env) };
    let controller = env
        .get::<NavigationController>()
        .expect("navigation transition cancelled without an installed controller");
    controller.transition_cancelled(id)
}

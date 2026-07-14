use alloc::boxed::Box;

use crate::array::WuiArray;
use crate::closure::{ForeignCallbackContext, WuiFn};
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiAnyView, WuiEnv};
use waterui_core::Str;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_graphics::color::ResolvedColor;
use waterui_navigation::tab::{Tab, TabPosition, Tabs};
use waterui_navigation::{
    Bar, CustomNavigationController, NavigationController, NavigationSearch, NavigationSplitLayout,
    NavigationStack, NavigationTitleDisplayMode, NavigationTransition, NavigationView,
    split::NavigationSplitDetailBuilder,
};
use waterui_text::styled::StyledStr;

into_ffi! {
    NavigationView,
    pub struct WuiNavigationView {
        bar: WuiBar,
        content: *mut WuiAnyView,
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
    pub leading: *mut WuiAnyView,
    pub trailing: *mut WuiAnyView,
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
            leading: self.leading.into_ffi(),
            trailing: self.trailing.into_ffi(),
            search: self.search.into_ffi(),
            color,
            hidden: self.hidden.into_ffi(),
            display_mode: self.display_mode.into_ffi(),
        }
    }
}

// FFI view bindings for navigation components
ffi_view!(NavigationView, WuiNavigationView, navigation_view);

/// FFI struct for NavigationStack<(),()>
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiNavigationTransition {
    PushPop = 0,
    Fade = 1,
    None = 2,
}

impl IntoFFI for NavigationTransition {
    type FFI = WuiNavigationTransition;

    fn into_ffi(self) -> Self::FFI {
        match self {
            NavigationTransition::PushPop => WuiNavigationTransition::PushPop,
            NavigationTransition::Fade => WuiNavigationTransition::Fade,
            NavigationTransition::None => WuiNavigationTransition::None,
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
        let transition = self.transition_style().into_ffi();
        WuiNavigationStack {
            root: self.into_inner().into_ffi(),
            transition,
        }
    }
}

ffi_view!(NavigationStack<(),()>, WuiNavigationStack, navigation_stack);

#[repr(C)]
pub struct WuiNavigationSplitLayout {
    /// Sidebar content.
    pub sidebar: *mut WuiAnyView,
    /// Placeholder content for empty regular-width selection.
    pub placeholder: *mut WuiAnyView,
    /// The currently selected detail identifier encoded as i32 (0 means no selection).
    pub selection: *mut WuiBinding<i32>,
    /// Resolver handle for building detail content from a selected id.
    pub detail: *mut WuiNavigationSplitDetail,
    /// Preferred sidebar width in logical points.
    pub sidebar_width: f32,
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
        let (sidebar, placeholder, selection, detail, sidebar_width) = self.into_parts();
        let selection = WuiBinding(nami::Binding::mapping(
            &selection,
            |value| value.map(i32::from).unwrap_or(0),
            |binding, value| {
                binding.set(core::num::NonZeroI32::new(value).map(waterui_core::id::Id::from));
            },
        ))
        .into_ffi();

        WuiNavigationSplitLayout {
            sidebar: sidebar.build().into_ffi(),
            placeholder: placeholder.build().into_ffi(),
            selection,
            detail: detail.into_ffi(),
            sidebar_width,
        }
    }
}

ffi_view!(
    NavigationSplitLayout,
    WuiNavigationSplitLayout,
    split_navigation_container
);

/// Position of the tab bar within the tab container.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiTabPosition {
    /// Tab bar is positioned at the top of the container.
    Top = 0,
    /// Tab bar is positioned at the bottom of the container.
    Bottom = 1,
}

impl From<TabPosition> for WuiTabPosition {
    fn from(pos: TabPosition) -> Self {
        match pos {
            TabPosition::Top => WuiTabPosition::Top,
            TabPosition::Bottom => WuiTabPosition::Bottom,
        }
    }
}

#[repr(C)]
pub struct WuiTabs {
    /// The currently selected tab identifier.
    pub selection: *mut WuiBinding<Id>,

    /// The collection of tabs to display.
    pub tabs: WuiArray<WuiTab>,

    /// Position of the tab bar (top or bottom).
    pub position: WuiTabPosition,
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
        let id_i32 = i32::from(self.label.tag);
        let id = u64::try_from(id_i32)
            .expect("tab id must be positive when converting to FFI u64 identifier");
        WuiTab {
            id,
            label: self.label.content.into_ffi(),
            content: self.content.into_ffi(),
        }
    }
}

impl IntoFFI for Tabs {
    type FFI = WuiTabs;
    fn into_ffi(self) -> Self::FFI {
        WuiTabs {
            selection: self.selection.into_ffi(),
            tabs: self.tabs.into_ffi(),
            position: self.position.into(),
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
    push: unsafe extern "C" fn(*mut (), WuiNavigationView),
    pop: unsafe extern "C" fn(*mut ()),
}

// SAFETY: ForeignNavigationController is only accessed from the UI thread.
unsafe impl Send for ForeignNavigationController {}

impl CustomNavigationController for ForeignNavigationController {
    fn push(&mut self, content: NavigationView) {
        let ffi_view = content.into_ffi();
        unsafe { (self.push)(self.context.data(), ffi_view) }
    }

    fn pop(&mut self) {
        unsafe { (self.pop)(self.context.data()) }
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
    push: unsafe extern "C" fn(*mut (), WuiNavigationView),
    pop: unsafe extern "C" fn(*mut ()),
    drop_context: unsafe extern "C" fn(*mut ()),
) {
    let env = unsafe { crate::borrow_ffi_mut(env) };
    let controller = ForeignNavigationController {
        context: unsafe { ForeignCallbackContext::new(context, drop_context) },
        push,
        pop,
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

/// Pops the top view from the navigation stack.
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
            controller.pop();
        }
    }
}

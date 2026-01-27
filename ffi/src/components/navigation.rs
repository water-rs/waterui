use alloc::boxed::Box;

use crate::array::WuiArray;
use crate::closure::WuiFn;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiAnyView, WuiEnv};
use waterui::Color;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_navigation::tab::{Tab, TabPosition, Tabs};
use waterui_navigation::{
    Bar, CustomNavigationController, NavigationController, NavigationStack,
    NavigationTitleDisplayMode, NavigationView,
};

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

into_ffi! {Bar,
    pub struct WuiBar {
        title: *mut WuiAnyView,
        color: *mut WuiComputed<Color>,
        hidden: *mut WuiComputed<bool>,
        display_mode: WuiNavigationTitleDisplayMode,
    }
}

// FFI view bindings for navigation components
ffi_view!(NavigationView, WuiNavigationView, navigation_view);

/// FFI struct for NavigationStack<(),()>
#[repr(C)]
pub struct WuiNavigationStack {
    /// The root view of the navigation stack.
    pub root: *mut WuiAnyView,
}

impl IntoFFI for NavigationStack<(), ()> {
    type FFI = WuiNavigationStack;
    fn into_ffi(self) -> Self::FFI {
        WuiNavigationStack {
            root: self.into_inner().into_ffi(),
        }
    }
}

ffi_view!(NavigationStack<(),()>, WuiNavigationStack, navigation_stack);

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
pub unsafe extern "C" fn waterui_tab_content(handler: *mut WuiTabContent) -> WuiNavigationView {
    unsafe {
        let view = (&*handler).build();
        IntoFFI::into_ffi(view)
    }
}

impl IntoFFI for Tab<Id> {
    type FFI = WuiTab;
    fn into_ffi(self) -> Self::FFI {
        WuiTab {
            id: i32::from(self.label.tag) as u64,
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

/// FFI-compatible navigation controller that bridges native push/pop callbacks to Rust.
///
/// Native backends create this controller with callback function pointers, then install
/// it into the environment. When Rust views call `NavigationController::push()` or `pop()`,
/// those calls are forwarded to the native callbacks.
#[repr(C)]
pub struct WuiNavigationController {
    /// Opaque data pointer passed to all callbacks (typically a pointer to native controller).
    data: *mut (),
    /// Callback invoked when a view is pushed onto the navigation stack.
    push: unsafe extern "C" fn(*mut (), WuiNavigationView),
    /// Callback invoked when popping the top view from the navigation stack.
    pop: unsafe extern "C" fn(*mut ()),
    /// Callback invoked when the controller is dropped (for cleanup).
    drop: unsafe extern "C" fn(*mut ()),
}

// SAFETY: WuiNavigationController is only accessed from main thread
unsafe impl Send for WuiNavigationController {}

impl CustomNavigationController for WuiNavigationController {
    fn push(&mut self, content: NavigationView) {
        let ffi_view = content.into_ffi();
        // SAFETY: Caller guarantees data and push callback are valid
        unsafe { (self.push)(self.data, ffi_view) }
    }

    fn pop(&mut self) {
        // SAFETY: Caller guarantees data and pop callback are valid
        unsafe { (self.pop)(self.data) }
    }
}

impl Drop for WuiNavigationController {
    fn drop(&mut self) {
        // SAFETY: Caller guarantees data and drop callback are valid
        unsafe { (self.drop)(self.data) }
    }
}

/// Creates a new navigation controller from native callbacks.
///
/// # Arguments
///
/// * `data` - Opaque pointer passed to all callbacks (typically pointer to native controller)
/// * `push` - Callback invoked when pushing a view onto the navigation stack
/// * `pop` - Callback invoked when popping the top view
/// * `drop` - Callback invoked when the controller is destroyed (for cleanup)
///
/// # Safety
///
/// - `data` must remain valid for the lifetime of the returned controller
/// - All callback function pointers must be valid and safe to call
/// - The `drop` callback must properly clean up resources associated with `data`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_controller_new(
    data: *mut (),
    push: unsafe extern "C" fn(*mut (), WuiNavigationView),
    pop: unsafe extern "C" fn(*mut ()),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut WuiNavigationController {
    Box::into_raw(Box::new(WuiNavigationController {
        data,
        push,
        pop,
        drop,
    }))
}

/// Installs a navigation controller into the environment.
///
/// After calling this function, views rendered with this environment can extract
/// the `NavigationController` and use it to push/pop navigation views.
///
/// # Safety
///
/// - `env` must be a valid pointer to a `WuiEnv`
/// - `controller` must be a valid pointer returned by `waterui_navigation_controller_new`
/// - `controller` is consumed by this function and must not be used afterward
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_navigation_controller(
    env: *mut WuiEnv,
    controller: *mut WuiNavigationController,
) {
    // SAFETY: Caller guarantees pointers are valid
    unsafe {
        let controller = *Box::from_raw(controller);
        // NavigationController::new wraps in Rc<RefCell<...>> internally
        let rust_controller = NavigationController::new(controller);
        (*env).insert(rust_controller);
    }
}

/// Drops a navigation controller.
///
/// # Safety
///
/// - `controller` must be a valid pointer returned by `waterui_navigation_controller_new`
/// - `controller` must not have been previously dropped or consumed
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_navigation_controller(
    controller: *mut WuiNavigationController,
) {
    // SAFETY: Caller guarantees pointer is valid and not previously dropped
    unsafe {
        drop(Box::from_raw(controller));
    }
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
    if env.is_null() {
        return false;
    }
    // SAFETY: Caller guarantees pointer is valid
    unsafe { (*env).get::<NavigationController>().is_some() }
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
    if env.is_null() {
        return;
    }
    // SAFETY: Caller guarantees pointer is valid
    unsafe {
        if let Some(controller) = (*env).get::<NavigationController>() {
            controller.pop();
        }
    }
}

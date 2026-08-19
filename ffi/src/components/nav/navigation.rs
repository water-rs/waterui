use alloc::boxed::Box;
use nami::SignalExt as _;

use crate::WuiSystemIcon;
use crate::action::WuiAction;
use crate::array::WuiArray;
use crate::closure::ForeignCallbackContext;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiAnyView, WuiEnv};
use waterui_core::Str;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_graphics::color::ResolvedColor;
use waterui_navigation::tab::{NativeTabStyle, Tab, TabIcon, TabsLayout};
use waterui_navigation::{
    Bar, ColumnWidth, CustomNavigationController, NativeNavigationSplitStyle,
    NativeNavigationTransition, NavigationController, NavigationDestinationState,
    NavigationLinkHint, NavigationSearch, NavigationSplitColumnVisibility, NavigationSplitLayout,
    NavigationStack, NavigationTitleDisplayMode, NavigationToolbarItem,
    NavigationToolbarPlacement, NavigationTransaction, NavigationView, ToolbarItemIcon,
    split::NavigationSplitDetailBuilder,
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

/// FFI representation of a navigation destination's lifecycle and pop state.
#[repr(C)]
#[derive(Debug)]
pub struct WuiNavigationDestinationState {
    /// Reactive signal reporting whether an interactive pop gesture is allowed.
    pub pop_enabled: *mut WuiComputed<bool>,
    /// Action invoked when the user attempts an interactive pop.
    pub pop_attempted: *mut WuiAction,
    /// Action invoked when the destination appears on screen.
    pub appear: *mut WuiAction,
    /// Action invoked when the destination disappears from screen.
    pub disappear: *mut WuiAction,
    /// Action that programmatically pops this destination.
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

/// FFI-safe representation of `IgnorableMetadata<NavigationLinkHint>`.
///
/// A navigation link renders as a plain button; this marker is what lets a
/// platform that draws a destination-following affordance around the row a
/// link sits in — the iOS disclosure chevron — recognize one. Renderers with
/// no such affordance ignore it and fall through to the content.
#[repr(C)]
#[derive(Debug)]
pub struct WuiIgnorableMetadataNavigationLinkHint {
    /// The link content wrapped by this marker.
    pub content: *mut WuiAnyView,
}

impl IntoFFI for waterui_core::IgnorableMetadata<NavigationLinkHint> {
    type FFI = WuiIgnorableMetadataNavigationLinkHint;

    fn into_ffi(self) -> Self::FFI {
        WuiIgnorableMetadataNavigationLinkHint {
            content: self.content.into_ffi(),
        }
    }
}

ffi_ignorable_metadata!(
    NavigationLinkHint,
    WuiIgnorableMetadataNavigationLinkHint,
    navigation_link_hint
);

/// FFI representation of a navigation bar's search field configuration.
#[repr(C)]
#[derive(Debug)]
pub struct WuiNavigationSearch {
    /// Reactive binding to the current search query text.
    pub text: *mut WuiBinding<Str>,
    /// Reactive computed placeholder text shown when the query is empty.
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

/// FFI representation of an optional navigation search configuration.
///
/// Used in place of a nullable pointer because `WuiNavigationSearch` embeds
/// value fields rather than being independently heap-allocated.
#[repr(C)]
#[derive(Debug)]
pub struct WuiOptionalNavigationSearch {
    /// `true` if `value` holds a real search configuration.
    pub has_value: bool,
    /// The search configuration, meaningful only when `has_value` is `true`.
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
    /// Always use a medium title, between inline and large.
    Medium = 2,
    /// Always use large title.
    Large = 3,
}

impl IntoFFI for NavigationTitleDisplayMode {
    type FFI = WuiNavigationTitleDisplayMode;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Automatic => WuiNavigationTitleDisplayMode::Automatic,
            Self::Inline => WuiNavigationTitleDisplayMode::Inline,
            Self::Medium => WuiNavigationTitleDisplayMode::Medium,
            Self::Large => WuiNavigationTitleDisplayMode::Large,
        }
    }
}

/// FFI representation of a `NavigationView`'s bar: title, toolbar, search,
/// and appearance.
#[repr(C)]
#[derive(Debug)]
pub struct WuiBar {
    /// The bar's title view.
    pub title: *mut WuiAnyView,
    /// The bar's subtitle view.
    pub subtitle: *mut WuiAnyView,
    /// Toolbar items placed around the bar, keyed by placement.
    pub toolbar: WuiArray<WuiNavigationToolbarItem>,
    /// The bar's optional search field configuration.
    pub search: WuiOptionalNavigationSearch,
    /// Reactive computed bar tint color, resolved against the environment.
    pub color: *mut WuiComputed<ResolvedColor>,
    /// Reactive signal controlling whether the bar is hidden.
    pub hidden: *mut WuiComputed<bool>,
    /// The title's display mode (automatic, inline, or large).
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
    /// The default, platform-chosen placement.
    Principal = 0,
    /// The primary action slot (e.g., a trailing "Save" or "Done" button).
    PrimaryAction = 1,
    /// The secondary action slot (e.g., a leading "Edit" button).
    SecondaryAction = 2,
    /// A confirmation action within a modal presentation.
    Confirmation = 3,
    /// A cancellation action within a modal presentation.
    Cancellation = 4,
    /// The bottom toolbar bar.
    BottomBar = 5,
    /// A status item, typically centered.
    Status = 6,
    /// The leading slot of the top bar.
    TopBarLeading = 7,
    /// The trailing slot of the top bar.
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

/// FFI representation of a single toolbar item and its placement.
#[repr(C)]
#[derive(Debug)]
pub struct WuiNavigationToolbarItem {
    /// Where this item is placed within the bar.
    pub placement: WuiNavigationToolbarPlacement,
    /// The item's content view.
    pub content: *mut WuiAnyView,
    /// The item's name, or null when it has no semantic label.
    ///
    /// The platforms disagree about how much of a toolbar action to draw: a Mac
    /// shows the icon alone and keeps the name for the overflow menu, its
    /// tooltip and assistive technology, while a phone's navigation bar
    /// commonly shows the text. The name and the icon therefore travel apart
    /// from the content view, so each backend can draw what its chrome calls
    /// for rather than placing a view it cannot interpret.
    pub title: *mut WuiComputed<StyledStr>,
    /// The item's icon as a platform symbol, or null.
    ///
    /// A backend that knows the symbol should prefer this: it renders at the
    /// size and weight the platform's own chrome calls for.
    pub system_icon: *mut WuiSystemIcon,
    /// The item's icon as a view to render, or null.
    ///
    /// Set when the icon is not a platform symbol — a packaged icon set, say.
    pub icon: *mut WuiAnyView,
}

impl IntoFFI for NavigationToolbarItem {
    type FFI = WuiNavigationToolbarItem;

    fn into_ffi(self) -> Self::FFI {
        let (system_icon, icon) = match self.icon {
            Some(ToolbarItemIcon::System(icon)) => (
                Box::into_raw(Box::new(icon.into_ffi())),
                core::ptr::null_mut(),
            ),
            Some(ToolbarItemIcon::View(view)) => (core::ptr::null_mut(), view.build().into_ffi()),
            None => (core::ptr::null_mut(), core::ptr::null_mut()),
        };
        WuiNavigationToolbarItem {
            placement: self.placement.into(),
            content: self.content.into_ffi(),
            title: self
                .title
                .map_or(core::ptr::null_mut(), |title| title.content().into_ffi()),
            system_icon,
            icon,
        }
    }
}

// FFI view bindings for navigation components
ffi_view!(NavigationView, WuiNavigationView, navigation_view);

/// FFI representation of the kind of transition used for a navigation push/pop.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiNavigationTransitionKind {
    /// The platform-default transition.
    Automatic = 0,
    /// A cross-fade transition.
    Fade = 1,
    /// A zoom transition anchored at `WuiNavigationTransition::source_id`.
    Zoom = 2,
    /// No transition; the destination appears instantly.
    None = 3,
    /// A caller-supplied custom transition.
    Custom = 4,
}

/// FFI representation of a navigation push/pop transition and its source anchor.
#[repr(C)]
#[derive(Debug)]
pub struct WuiNavigationTransition {
    /// The kind of transition to perform.
    pub kind: WuiNavigationTransitionKind,
    /// The zoom-transition source view identifier, meaningful only when
    /// `kind` is `WuiNavigationTransitionKind::Zoom`.
    pub source_id: i32,
}

impl IntoFFI for NativeNavigationTransition {
    type FFI = WuiNavigationTransition;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Automatic => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Automatic,
                source_id: 0,
            },
            Self::Fade => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Fade,
                source_id: 0,
            },
            Self::Zoom(source) => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Zoom,
                source_id: i32::from(source),
            },
            Self::None => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::None,
                source_id: 0,
            },
            Self::Custom => WuiNavigationTransition {
                kind: WuiNavigationTransitionKind::Custom,
                source_id: 0,
            },
        }
    }
}

/// FFI struct for `NavigationStack`<(),()>
#[repr(C)]
#[derive(Debug)]
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
    // SAFETY: the caller contract makes `root` an owning view handle consumed here.
    let root: waterui::AnyView = unsafe { crate::IntoRust::into_rust(root) };
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
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
    // SAFETY: this entry point shares its callee's contract — `root` is an owning
    // handle and `env` a valid handle alive for the call.
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

/// FFI representation of a `NavigationSplitLayout`'s columns, selections, and
/// resolvers.
#[repr(C)]
#[derive(Debug)]
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

/// FFI representation of a resizable sidebar's width constraints.
#[repr(C)]
#[derive(Debug)]
pub struct WuiNavigationColumnWidth {
    /// Minimum allowed sidebar width.
    pub min: f32,
    /// Preferred (default) sidebar width.
    pub ideal: f32,
    /// Maximum allowed sidebar width.
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

/// FFI representation of a `NavigationSplitLayout`'s native presentation style.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiNavigationSplitStyle {
    /// The platform-default split style.
    Automatic = 0,
    /// Sidebar and detail columns share space evenly.
    Balanced = 1,
    /// The detail column is given prominence over the sidebar.
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

/// Opaque handle to a Rust-owned resolver for split-view detail content.
///
/// The resolver builds detail content for a selected split identifier. Native
/// backends only pass this pointer back into the FFI functions below; they
/// never inspect its contents.
#[repr(C)]
#[derive(Debug)]
pub struct WuiNavigationSplitDetail {
    _private: [u8; 0],
}

impl crate::IntoFFI for NavigationSplitDetailBuilder {
    type FFI = *mut WuiNavigationSplitDetail;

    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(self)).cast::<WuiNavigationSplitDetail>()
    }
}

impl crate::IntoRust for *mut WuiNavigationSplitDetail {
    type Rust = NavigationSplitDetailBuilder;

    unsafe fn into_rust(self) -> Self::Rust {
        // SAFETY: `IntoRust::into_rust` requires an owning pointer from the matching
        // `into_ffi`, which boxed this builder.
        unsafe { *Box::from_raw(self.cast::<NavigationSplitDetailBuilder>()) }
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
    // SAFETY: the caller contract makes `value` an owning handle, dropped once here.
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
    // SAFETY: the caller contract requires `handler` to be a valid builder handle
    // alive for this call; it is only borrowed.
    let handler = unsafe { crate::borrow_ffi(handler as *const NavigationSplitDetailBuilder) };
    // SAFETY: the caller contract makes `selected` an owning handle consumed here.
    let selected = unsafe { crate::IntoRust::into_rust(selected) };
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
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
        |value| value.map_or(0, i32::from),
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
    /// The platform-default tab presentation.
    Automatic = 0,
    /// A bottom tab bar.
    TabBar = 1,
    /// A sidebar of tabs.
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

/// FFI representation of the `Tabs` component.
#[repr(C)]
#[derive(Debug)]
pub struct WuiTabs {
    /// The currently selected tab identifier.
    pub selection: *mut WuiBinding<Id>,

    /// The collection of tabs to display.
    pub tabs: WuiArray<WuiTab>,

    /// Native adaptive tab style.
    pub style: WuiTabStyle,
}

opaque!(WuiTabContent, AnyViewBuilder<NavigationView>, tab_content);

/// FFI representation of a single tab within a `Tabs` component.
#[repr(C)]
#[derive(Debug)]
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

    /// The tab's icon as a platform symbol, or null.
    ///
    /// A backend that knows the symbol should prefer this: it renders at the
    /// size and weight the platform's own chrome calls for.
    pub system_icon: *mut WuiSystemIcon,

    /// The tab's icon as a view to render, or null.
    ///
    /// Set when the icon is not a platform symbol — a packaged icon set, say.
    /// A backend whose tab item takes an image has to rasterize this itself.
    pub icon: *mut WuiAnyView,
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
    // SAFETY: the caller contract requires both handles to be valid and alive for
    // this call; both are only borrowed.
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
        let (system_icon, icon) = match self.icon {
            Some(TabIcon::System(icon)) => (
                Box::into_raw(Box::new(icon.into_ffi())),
                core::ptr::null_mut(),
            ),
            Some(TabIcon::View(view)) => (core::ptr::null_mut(), view.build().into_ffi()),
            None => (core::ptr::null_mut(), core::ptr::null_mut()),
        };
        let id_i32 = i32::from(self.id);
        let id = u64::try_from(id_i32)
            .expect("tab id must be positive when converting to FFI u64 identifier");
        WuiTab {
            id,
            label: self.label.into_ffi(),
            content: self.content.into_ffi(),
            badge: self.badge.map_or(core::ptr::null_mut(), IntoFFI::into_ffi),
            enabled: self.enabled.into_ffi(),
            system_icon,
            icon,
        }
    }
}

impl IntoFFI for TabsLayout {
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
ffi_view!(TabsLayout, WuiTabs, tabs);

// =============================================================================
// Navigation Controller FFI
// =============================================================================

struct ForeignNavigationController {
    context: ForeignCallbackContext,
    apply: unsafe extern "C" fn(*mut (), WuiNavigationTransaction),
}

/// One atomic navigation stack mutation passed to a native backend.
#[repr(C)]
#[derive(Debug)]
pub struct WuiNavigationTransaction {
    /// Monotonically increasing identifier used to acknowledge this
    /// transaction via `waterui_navigation_transition_completed` /
    /// `waterui_navigation_transition_cancelled`.
    pub id: u64,
    /// Number of existing stack entries, counted from the root, that are
    /// retained unchanged by this transaction.
    pub retained_prefix: usize,
    /// Number of existing stack entries removed after the retained prefix.
    pub removed: usize,
    /// The views to insert after the retained prefix, replacing any removed entries.
    pub inserted: WuiArray<WuiNavigationView>,
}

#[expect(
    clippy::non_send_fields_in_send_ty,
    reason = "the foreign callback context is created, invoked, and dropped on the platform main thread; `Send` is asserted only to satisfy the controller trait bound"
)]
// SAFETY: the foreign callback context is created, invoked, and dropped on the
// platform main thread, so it never crosses a thread boundary.
unsafe impl Send for ForeignNavigationController {}

impl CustomNavigationController for ForeignNavigationController {
    fn apply(&mut self, transaction: NavigationTransaction) {
        let (id, retained_prefix, removed, inserted) = transaction.into_parts();
        let inserted = inserted
            .into_iter()
            .map(|builder| builder.build())
            .map(IntoFFI::into_ffi)
            .collect::<alloc::vec::Vec<_>>();
        // SAFETY: `apply` and the context were registered together by the backend, and
        // the `ForeignCallbackContext` keeps the data alive for as long as `self`.
        unsafe {
            (self.apply)(
                self.context.data(),
                WuiNavigationTransaction {
                    id,
                    retained_prefix,
                    removed,
                    inserted: WuiArray::new(inserted),
                },
            );
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
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };
    let controller = ForeignNavigationController {
        // SAFETY: the caller contract requires `context` and `drop_context` to be one
        // registration from the backend.
        context: unsafe { ForeignCallbackContext::new(context, drop_context) },
        apply,
    };
    env.insert(NavigationController::new(controller));
}

/// Checks if a navigation controller is installed in the environment.
///
/// Returns true if a `NavigationController` is available, false otherwise.
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
/// If no `NavigationController` is installed in the environment, this function does nothing.
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
/// # Panics
///
/// Panics if no `NavigationController` is installed in the environment.
///
/// # Safety
///
/// `env` must point to a live environment containing the controller for the
/// native stack that completed the pop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_complete_native_pop(env: *const WuiEnv, count: usize) {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
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
/// # Panics
///
/// Panics if no `NavigationController` is installed in the environment.
///
/// # Safety
///
/// `env` must point to the live environment for the reporting stack.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_transition_completed(
    env: *const WuiEnv,
    id: u64,
) -> bool {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    let controller = env
        .get::<NavigationController>()
        .expect("navigation transition completed without an installed controller");
    controller.transition_completed(id)
}

/// Acknowledges cancellation of a native navigation transaction.
///
/// # Panics
///
/// Panics if no `NavigationController` is installed in the environment.
///
/// # Safety
///
/// `env` must point to the live environment for the reporting stack.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_navigation_transition_cancelled(
    env: *const WuiEnv,
    id: u64,
) -> bool {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    let controller = env
        .get::<NavigationController>()
        .expect("navigation transition cancelled without an installed controller");
    controller.transition_cancelled(id)
}

//! Header-free embedding ABI. Handles and callbacks are main-thread owned.

use crate::contract::{KeepAlive, Mounted};
use cocoa_ui::{MainThreadMarker, PlatformView, Retained};
#[cfg(target_os = "ios")]
use objc2::Message;
use std::ffi::CStr;
use std::{
    ffi::{c_char, c_void},
    rc::Rc,
};

use waterui::{Environment, app::App, reactive::Binding};
use waterui_core::layout::{LayoutDirection, ProposalSize};
use waterui_locale::Locale;

/// The six platform text styles a `SwiftUI` host resolves and the ABI packs
/// as 18 `f64` triples — `size`, `weight`, `line_height` per style, in this
/// order.
const HOST_FONT_STYLES: [cocoa_ui::system_font::TextStyle; 6] = [
    cocoa_ui::system_font::TextStyle::Body,
    cocoa_ui::system_font::TextStyle::Title1,
    cocoa_ui::system_font::TextStyle::Headline,
    cocoa_ui::system_font::TextStyle::Subheadline,
    cocoa_ui::system_font::TextStyle::Caption1,
    cocoa_ui::system_font::TextStyle::Footnote,
];

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// Process services shared explicitly by embedded application instances.
#[derive(Debug)]
pub struct Runtime {
    env: Environment,
    fonts: std::cell::RefCell<crate::fonts::FontRegistrations>,
}

impl Runtime {
    /// A runtime over a minimal environment, for trials driving [`mount`]
    /// directly — `runtime_create`'s startup and GPU preparation stay out
    /// of the harness.
    #[cfg(all(target_os = "macos", feature = "native-test"))]
    #[must_use]
    pub fn for_test() -> Self {
        let mut env = Environment::new();
        env.insert(crate::first_paint::FirstPaint::default());
        waterui::text::install_system_font_collection(&mut env);
        Self {
            env,
            fonts: std::cell::RefCell::default(),
        }
    }
}

/// A host's declared presentation environment for one mount — a `SwiftUI`
/// representable's snapshot rather than the platform ambient.
///
/// `fonts` is the six resolved metrics in [`HOST_FONT_STYLES`] order:
/// `SwiftUI`'s own `Font.resolve(in:)` against the host's
/// `fontResolutionContext`, so the sizes, weights and line heights are the
/// host's, including its content-size category — never a synthesized
/// scale table on top of the ambient platform font.
#[derive(Debug, Clone)]
#[expect(
    clippy::exhaustive_structs,
    reason = "closed: the declared host snapshot is scheme, direction, locale and the six fonts"
)]
pub struct HostEnvironment {
    /// The host's `SwiftUI` color scheme.
    pub scheme: cocoa_ui::ColorScheme,
    /// The host's layout direction.
    pub direction: LayoutDirection,
    /// The host's locale, BCP-47 — scoped to the mount; the process's own
    /// regional settings stay the embedder's.
    pub locale: Locale,
    /// Resolved metrics in [`HOST_FONT_STYLES`] order.
    pub fonts: [cocoa_ui::system_font::FontMetrics; 6],
}

impl HostEnvironment {
    /// Reads one ABI snapshot: `dark`/`rtl` from the host's trait
    /// environment, `locale` as a NUL-terminated BCP-47 string, and
    /// `HOST_FONT_STYLES.len() * 3` `f64`s from `fonts` — one
    /// `(size, weight, line_height)` triple per style, in that order.
    ///
    /// # Panics
    /// Panics when `locale` is not valid UTF-8 or a valid BCP-47 tag.
    ///
    /// # Safety
    /// `locale` is a live NUL-terminated string and `fonts` points to
    /// eighteen readable, initialized `f64`s for the duration of the call.
    #[must_use]
    pub unsafe fn from_abi(
        dark: bool,
        rtl: bool,
        locale: *const c_char,
        fonts: *const f64,
    ) -> Self {
        // SAFETY: the caller contract supplies a live NUL-terminated string.
        let locale = unsafe { CStr::from_ptr(locale) }
            .to_str()
            .expect("the host locale is valid UTF-8")
            .parse::<Locale>()
            .expect("the host locale is a valid BCP-47 tag");
        // SAFETY: the caller contract supplies the eighteen readable doubles.
        let slice = unsafe { std::slice::from_raw_parts(fonts, HOST_FONT_STYLES.len() * 3) };
        // `HOST_FONT_STYLES` is the index contract: triple `i` carries the
        // metrics of style `i`.
        let fonts = std::array::from_fn(|index| cocoa_ui::system_font::FontMetrics {
            size: slice[index * 3],
            weight: slice[index * 3 + 1],
            line_height: slice[index * 3 + 2],
        });
        Self {
            scheme: if dark {
                cocoa_ui::ColorScheme::Dark
            } else {
                cocoa_ui::ColorScheme::Light
            },
            direction: if rtl {
                LayoutDirection::RightToLeft
            } else {
                LayoutDirection::LeftToRight
            },
            locale,
            fonts,
        }
    }
}

/// The signals a `HostEnvironment`-scoped mount keeps: the theme slots the
/// host re-feeds and the direction and locale bindings it overrides.
struct HostScope {
    signals: crate::theme::ThemeSignals,
    direction: Binding<LayoutDirection>,
    locale: Binding<Locale>,
}

impl HostScope {
    /// Installs the host's declared environment before the app body runs:
    /// theme slots under the host snapshot, the direction binding, and the
    /// host locale as an explicit `Binding` — every regional extractor
    /// reads bindings first, so formatters follow the host's locale while
    /// `regional::set_locale_tag` and the platform's own preferences stay
    /// the embedder's to manage.
    fn install(env: &mut Environment, host: &HostEnvironment) -> Self {
        let signals = crate::theme::install(env, host.scheme);
        crate::theme::refresh_host(&signals, host.scheme, &host.fonts);
        let direction = waterui::reactive::binding(host.direction);
        env.insert(direction.clone());
        env.insert(host.locale.clone());
        env.insert(waterui_locale::regional::current_settings().with_locale(&host.locale));
        let locale = waterui::reactive::binding(host.locale.clone());
        env.insert(locale.clone());
        Self {
            signals,
            direction,
            locale,
        }
    }
}

impl core::fmt::Debug for HostScope {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HostScope")
            .field("signals", &self.signals)
            .finish_non_exhaustive()
    }
}

/// An attached application instance. Dropping it detaches the entire subtree.
pub struct Mount {
    /// The mounted subtree. Declared first because Rust drops fields in
    /// declaration order: `Mounted` must detach and destruct while `root`,
    /// the external host, the controller and the scene registrations are
    /// still retained — `removeFromSuperview` cannot run inside a parent
    /// already in native teardown. `NativeLeaf` expresses the same inverse
    /// ownership ordering: child state before the platform view.
    content: Rc<Mounted>,
    /// The scoped host signals, when the mount carries a declared
    /// `SwiftUI` environment — `None` under the ambient platform.
    host_scope: Option<HostScope>,
    root: Retained<HostView>,
    _host: Retained<PlatformView>,
    _keepalive: KeepAlive,
    #[cfg(target_os = "ios")]
    controller: Retained<cocoa_ui::uikit::ViewController>,
    #[cfg(target_os = "ios")]
    scenes: crate::scene_registry::Registration<crate::windows::Scenes>,
}

impl core::fmt::Debug for Mount {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mount")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        // Clear the root's handler slots first so a callback the removal
        // itself delivers — a window/superview move, a final layout pass —
        // finds `None` and does nothing by construction.
        self.root.clear_handlers();
        #[cfg(target_os = "ios")]
        cocoa_ui::uikit::view_controller::will_move_to_parent(&self.controller);
        cocoa_ui::view::remove_from_superview(&self.root);
        #[cfg(target_os = "ios")]
        cocoa_ui::uikit::view_controller::remove_from_parent(&self.controller);
    }
}

pub(crate) fn install_services(env: &mut Environment) {
    #[cfg(feature = "map")]
    {
        use waterui_core::view::ViewConfiguration;
        env.insert_hook::<waterui_map::MapConfig, _>(|_env, config| config.render());
    }
    crate::windows::install_manager(env);
    #[cfg(feature = "view_renderer")]
    crate::components::view_renderer::install_service(env);
}

/// Initializes process services and returns one owned runtime through `ready`.
///
/// # Panics
/// Panics off the main thread or if native runtime initialization fails.
///
/// # Safety
/// Call once per process, on the main thread. `context` and `ready` must
/// remain valid until the callback, which runs exactly once on that thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_runtime_create(
    context: *mut c_void,
    ready: unsafe extern "C" fn(*mut c_void, *mut c_void),
) {
    MainThreadMarker::new().expect("embedding initialization runs on the main thread");
    let inspector = crate::startup::initialize();
    let mut env = Environment::new();
    env.insert(crate::first_paint::FirstPaint::default());
    #[cfg(target_os = "ios")]
    env.insert(crate::scene_registry::SceneRegistry::<crate::windows::Scenes>::default());
    waterui::inspector::install(&mut env, inspector);
    waterui::text::install_system_font_collection(&mut env);
    let mut runtime = Box::new(Runtime {
        env,
        fonts: std::cell::RefCell::default(),
    });
    // SAFETY: the owning box moves into the completion closure, keeping the
    // environment alive throughout GPU setup. The host owns it afterwards.
    unsafe {
        crate::gpu_runtime::prepare(&raw mut runtime.env, move || {
            ready(context, Box::into_raw(runtime).cast());
        });
    }
}

/// Releases a runtime after its last mount is destroyed.
///
/// # Panics
/// Panics off the main thread.
///
/// # Safety
/// `runtime` is an owned result of `runtime_create`, consumed once on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_runtime_drop(runtime: *mut c_void) {
    MainThreadMarker::new().expect("runtime destruction runs on the main thread");
    // SAFETY: this consumes the box transferred by runtime_create exactly once.
    drop(unsafe { Box::from_raw(runtime.cast::<Runtime>()) });
}

/// Constructs and mounts the app supplied by `export_app!`.
///
/// # Panics
/// Panics off the main thread, if the app declares no root window, or if
/// native rendering or resource initialization fails. On iOS, also panics
/// when the host does not belong to a view controller.
///
/// # Safety
/// `runtime` is borrowed from `runtime_create`; `host` is a live platform
/// view on the main thread. Paths are live NUL-terminated UTF-8 strings.
/// On iOS the host belongs to a view controller. The returned mount is owned.
/// Registered font files must remain in place until process exit, including
/// when another package reuses the same font data after this mount is dropped.
pub unsafe fn mount(
    runtime: *const c_void,
    host: *mut c_void,
    assets: *const c_char,
    fonts: *const c_char,
    host_environment: Option<HostEnvironment>,
    app: impl FnOnce(Environment) -> App,
) -> *mut c_void {
    let mtm = MainThreadMarker::new().expect("mount runs on the main thread");
    // SAFETY: the runtime and host are borrowed live objects under the caller contract.
    let runtime = unsafe { &*runtime.cast::<Runtime>() };
    // SAFETY: host points to a platform view and is retained for the mount lifetime.
    let host = unsafe { &*host.cast::<PlatformView>() };
    let mut env = runtime.env.clone();
    #[cfg(target_os = "ios")]
    let scenes = runtime
        .env
        .get::<crate::scene_registry::SceneRegistry<crate::windows::Scenes>>()
        .expect("runtime owns scene routing")
        .register(crate::windows::Scenes::default());
    // SAFETY: both paths remain readable for this call; from_host copies them.
    let resources = unsafe { crate::resources::from_host(assets, fonts) };
    runtime.fonts.borrow_mut().register_bundle_fonts(&resources);
    env.insert(resources);
    crate::dispatch::install(&mut env);
    install_services(&mut env);

    let mut keepalive = KeepAlive::default();
    #[cfg(target_os = "macos")]
    let (root, host_scope) =
        prepare_mount_root(&mut env, host, &mut keepalive, mtm, host_environment);
    #[cfg(target_os = "ios")]
    let (controller, root, host_scope) =
        prepare_mount_root(&mut env, host, &mut keepalive, mtm, host_environment);
    let parts = app(env).into_parts();
    // An embedded application's lifetime belongs to its host — iOS kills the
    // process without notice — so the termination machine is never started.
    let _ = parts.termination;
    #[allow(unused_mut)]
    let mut env = parts.env;
    #[cfg(feature = "webview")]
    crate::components::webview::install_service(&mut env);
    let mut windows = parts.windows.into_iter();
    let declaration = windows
        .next()
        .expect("an embedded app must declare a root window");

    #[cfg(target_os = "ios")]
    {
        let parent = cocoa_ui::uikit::view_controller::enclosing_controller(host)
            .expect("the embedding host belongs to a view controller");
        cocoa_ui::uikit::view_controller::add_child(&parent, &controller);
    }
    cocoa_ui::view::set_frame(&root, cocoa_ui::view::bounds(host));
    cocoa_ui::view::set_autoresizing_flexible_size(&root);
    cocoa_ui::view::add_subview(host, &root);
    #[cfg(target_os = "ios")]
    cocoa_ui::uikit::view_controller::did_move_to_parent(&controller);
    let content = mount_content(&root, declaration.build_content(), &env, &mut keepalive);
    #[cfg(target_os = "macos")]
    {
        crate::windows::bind_background(
            &mut keepalive,
            &root,
            &declaration.resolved_background(&env),
            &env,
            mtm,
        );
        for window in windows {
            keepalive.keep(crate::windows::realize(window, &env, mtm));
        }
        keepalive.keep(parts.menu_bar);
    }
    #[cfg(target_os = "ios")]
    {
        crate::windows::bind_background(
            &mut keepalive,
            &root,
            &declaration.resolved_background(&env),
            &env,
            mtm,
        );
        let declarations = std::iter::once(declaration).chain(windows).collect();
        crate::windows::declare(&scenes.state, declarations, &env, 1, mtm);
        keepalive.keep(parts.menu_bar);
    }
    cocoa_ui::view::layout_immediately(&root);
    crate::first_paint::mark(&root, &env);
    keepalive.keep(env);
    Box::into_raw(Box::new(Mount {
        host_scope,
        root,
        _host: cocoa_ui::view::retain_base(host),
        content,
        _keepalive: keepalive,
        #[cfg(target_os = "ios")]
        controller,
        #[cfg(target_os = "ios")]
        scenes,
    }))
    .cast()
}

/// Builds the root view and, for a scoped mount, the host-signal scope:
/// the ambient path installs the platform's own theme and locale observers,
/// while a declared `HostEnvironment` writes its slots before the app body
/// runs so the first frame draws under the host's snapshot.
#[cfg(target_os = "macos")]
fn prepare_mount_root(
    env: &mut Environment,
    host: &PlatformView,
    keepalive: &mut KeepAlive,
    mtm: MainThreadMarker,
    host_environment: Option<HostEnvironment>,
) -> (Retained<HostView>, Option<HostScope>) {
    let root = create_root(env, host, keepalive, mtm, host_environment.is_none());
    install_scoped_or_ambient(env, mtm, keepalive, host_environment.is_none());
    let host_scope = host_environment.map(|host_env| HostScope::install(env, &host_env));
    (root, host_scope)
}

/// The `UIKit` realization of [`prepare_mount_root`]: the controller's own
/// theme observer under the ambient path.
#[cfg(target_os = "ios")]
fn prepare_mount_root(
    env: &mut Environment,
    host: &PlatformView,
    keepalive: &mut KeepAlive,
    mtm: MainThreadMarker,
    host_environment: Option<HostEnvironment>,
) -> (
    Retained<cocoa_ui::uikit::ViewController>,
    Retained<HostView>,
    Option<HostScope>,
) {
    let controller =
        cocoa_ui::uikit::ViewController::new(mtm, HostView::new(mtm, cocoa_ui::view::bounds(host)));
    if host_environment.is_none() {
        crate::theme::install_controller(env, &controller, keepalive);
    }
    install_scoped_or_ambient(env, mtm, keepalive, host_environment.is_none());
    let host_scope = host_environment.map(|host_env| HostScope::install(env, &host_env));
    let root = controller.host_view().retain();
    (controller, root, host_scope)
}

/// A scoped mount never runs the ambient locale installer: it publishes
/// through the process-global `set_locale_tag`, which is the embedder's to
/// own. The host's locale enters as an explicit `Binding` in
/// `HostScope::install`.
fn install_scoped_or_ambient(
    env: &mut Environment,
    mtm: MainThreadMarker,
    keepalive: &mut KeepAlive,
    ambient: bool,
) {
    if ambient {
        keepalive.keep(crate::locale::install(env, mtm));
    }
}

#[cfg(target_os = "macos")]
fn create_root(
    env: &mut Environment,
    host: &PlatformView,
    keepalive: &mut KeepAlive,
    mtm: MainThreadMarker,
    ambient: bool,
) -> Retained<HostView> {
    // A host-scoped mount keeps no ambient observer: the host's declared
    // environment owns its slots, so the platform's effective appearance
    // cannot clobber them. The ambient path keeps the platform as source
    // of truth — the host view's `appearance` or the chain above it.
    if ambient {
        let theme = Rc::new(crate::theme::install(
            env,
            cocoa_ui::appkit::effective_color_scheme(host),
        ));
        let observed = theme.clone();
        keepalive.keep(cocoa_ui::appkit::observe_effective_appearance(
            host,
            move |scheme| {
                crate::theme::refresh(&observed, scheme);
            },
        ));
        keepalive.keep(theme);
    }
    HostView::new(mtm, cocoa_ui::view::bounds(host))
}

pub(crate) fn mount_content(
    root: &HostView,
    view: waterui::AnyView,
    env: &Environment,
    keepalive: &mut KeepAlive,
) -> Rc<Mounted> {
    let leaf = crate::dispatch::render(view, env);
    let content = Rc::new(leaf.mount(root));
    crate::primary_content::forward(root, content.view());
    let placed = Rc::clone(&content);
    crate::inspector::install(root, env, keepalive);
    root.set_layout_handler(move |root| {
        let frame = crate::native_layout::LayoutContext::of(root).content_frame(placed.view());
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the layout contract uses f32 points"
        )]
        let proposal = waterui_core::layout::ProposalSize::new(
            Some(frame.size.width as f32),
            Some(frame.size.height as f32),
        );
        // The root fills platform geometry, which sits on the device-pixel
        // grid, so its own layout origin discarded nothing.
        crate::proposal::place(
            placed.view(),
            frame,
            Some(proposal),
            cocoa_ui::view::SubpixelOffset::ZERO,
        );
        // Placement changes the child's bounds after the host's own layout.
        // Complete that child's pending layout before this placement finishes.
        cocoa_ui::view::layout_immediately(placed.view());
    });
    content
}

/// Detaches and destroys one mounted instance.
///
/// # Panics
/// Panics off the main thread.
///
/// # Safety
/// `mount` is the owned result of `waterui_apple_mount`, consumed once on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_mount_drop(mount: *mut c_void) {
    MainThreadMarker::new().expect("unmount runs on the main thread");
    // SAFETY: the caller transfers the live mount box exactly once.
    drop(unsafe { Box::from_raw(mount.cast::<Mount>()) });
}

/// Re-evaluates presentation visibility for everything mounted under this
/// instance.
///
/// An embedding host calls this after mutating an ancestor of its mounted
/// root — hiding or showing it, changing its alpha, its clipping bounds,
/// its transform, or reparenting it inside the same window. WaterUI-owned
/// layout, scroll and scene transitions reach surfaces on their own;
/// ancestors outside the `CocoaUi` classes publish nothing a descendant
/// can observe, so the host must report those mutations itself.
///
/// # Panics
/// Panics off the main thread.
///
/// # Safety
/// `mount` is a live mount handle borrowed on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_update_visibility(mount: *const c_void) {
    MainThreadMarker::new().expect("visibility refresh runs on the main thread");
    // SAFETY: the caller borrows a live mount for this call. Emitting on
    // the mounted root wakes every surface watch that subscribed to it —
    // ancestor subscriptions cover the whole mounted tree.
    unsafe { &*mount.cast::<Mount>() }
        .root
        .visibility_emitter()
        .emit();
}

/// Pushes a host's declared environment onto a scoped mount: the same
/// snapshot shape `mount` takes, so a `SwiftUI` environment change re-feeds
/// the theme and direction without remounting.
///
/// # Panics
/// Panics off the main thread, or on a mount created without a host
/// environment — the ambient path owns its own observers.
///
/// # Safety
/// `mount` is a live mount handle borrowed on the main thread, created with
/// a host environment. `host_fonts` points to eighteen readable, initialized
/// `f64`s — six `(size, weight, line_height)` triples in
/// [`HostEnvironment`]'s style order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_update_environment(
    mount: *const c_void,
    dark: bool,
    rtl: bool,
    locale: *const c_char,
    host_fonts: *const f64,
) {
    MainThreadMarker::new().expect("environment updates run on the main thread");
    // SAFETY: the caller borrows a live mount and passes a live locale
    // string and eighteen readable doubles per the documented ABI.
    let mount = unsafe { &*mount.cast::<Mount>() };
    let scope = mount
        .host_scope
        .as_ref()
        .expect("environment updates require a mount created with a host environment");
    // SAFETY: `locale` and `host_fonts` follow `from_abi`'s contract.
    let host = unsafe { HostEnvironment::from_abi(dark, rtl, locale, host_fonts) };
    crate::theme::refresh_host(&scope.signals, host.scheme, &host.fonts);
    scope.direction.set(host.direction);
    scope.locale.set(host.locale);
    crate::measure_memo::invalidate();
    mount.root.set_needs_layout();
}

/// Measures the mounted content under the host's size proposal, writing
/// `width` and `height` doubles to `result`.
///
/// `has_width`/`has_height` carry the proposal's axis presence: a host
/// proposal's `nil` axis is unbounded (`None` to the layout), while an
/// explicit `0.0` or `.infinity` reaches it as that exact value.
///
/// # Panics
/// Panics off the main thread.
///
/// # Safety
/// `mount` is a live mount handle borrowed on the main thread. `result`
/// points to two writable `f64`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_measure(
    mount: *const c_void,
    width: f64,
    height: f64,
    has_width: bool,
    has_height: bool,
    result: *mut f64,
) {
    MainThreadMarker::new().expect("measuring runs on the main thread");
    // SAFETY: the caller borrows a live mount and passes a writable
    // two-element `f64` buffer.
    let mount = unsafe { &*mount.cast::<Mount>() };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; the proposal's f64 fits"
    )]
    let proposal = ProposalSize::new(
        has_width.then_some(width as f32),
        has_height.then_some(height as f32),
    );
    let measured = mount.content.layout().measure(proposal);
    // SAFETY: `result` is a live two-element buffer per the caller contract.
    unsafe {
        result.write(f64::from(measured.size.width));
        result.add(1).write(f64::from(measured.size.height));
    }
}

/// Returns the mount's scene route, valid only in its owning runtime.
///
/// # Panics
/// Panics off the main thread.
///
/// # Safety
/// `mount` is a live mount handle, borrowed on the main thread.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_mount_scene_id(mount: *const c_void) -> u64 {
    MainThreadMarker::new().expect("scene routing runs on the main thread");
    // SAFETY: the caller borrows a live mount for this call.
    unsafe { &*mount.cast::<Mount>() }.scenes.id
}

/// Connects an embedding host's scene to the identified mount.
/// Returns an owned `UIWindow`, or null when the mount has been destroyed.
///
/// # Panics
/// Panics off the main thread or if the scene's native content cannot render.
///
/// # Safety
/// `runtime` and `scene` are live runtime and `UIWindowScene` pointers, borrowed
/// on the main thread. The host consumes the returned Objective-C retain.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_scene_connect(
    runtime: *const c_void,
    mount_id: u64,
    scene: *mut c_void,
) -> *mut c_void {
    let mtm = MainThreadMarker::new().expect("scene connection runs on the main thread");
    // SAFETY: both pointers are live borrowed objects under the caller contract.
    let runtime = unsafe { &*runtime.cast::<Runtime>() };
    // SAFETY: the host passes its live UIKit scene.
    let scene = unsafe { &*scene.cast::<cocoa_ui::objc2_ui_kit::UIWindowScene>() };
    let registry = runtime
        .env
        .get::<crate::scene_registry::SceneRegistry<crate::windows::Scenes>>()
        .expect("runtime owns scene routing");
    let Some(scenes) = registry.get(mount_id) else {
        return std::ptr::null_mut();
    };
    Retained::into_raw(crate::windows::connect_embedded(&scenes, scene, mtm)).cast()
}

/// Releases a disconnected scene's content and subscriptions.
///
/// # Panics
/// Panics off the main thread.
///
/// # Safety
/// `runtime` and `scene` are live borrowed objects on the main thread.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_apple_scene_disconnect(
    runtime: *const c_void,
    mount_id: u64,
    scene: *mut c_void,
) {
    MainThreadMarker::new().expect("scene disconnection runs on the main thread");
    // SAFETY: the host borrows the live runtime and UIKit scene for this call.
    let runtime = unsafe { &*runtime.cast::<Runtime>() };
    // SAFETY: the host passes its live UIKit scene.
    let scene = unsafe { &*scene.cast::<cocoa_ui::objc2_ui_kit::UIWindowScene>() };
    let registry = runtime
        .env
        .get::<crate::scene_registry::SceneRegistry<crate::windows::Scenes>>()
        .expect("runtime owns scene routing");
    if let Some(scenes) = registry.get(mount_id) {
        scenes.disconnect(&scene.session().persistentIdentifier().to_string());
    }
}

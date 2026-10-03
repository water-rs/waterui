//! Header-free embedding ABI. Handles and callbacks are main-thread owned.

use crate::contract::{KeepAlive, Mounted};
use cocoa_ui::{MainThreadMarker, PlatformView, Retained};
#[cfg(target_os = "ios")]
use objc2::Message;
use std::{
    ffi::{c_char, c_void},
    rc::Rc,
};
use waterui::{Environment, app::App};

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

/// An attached application instance. Dropping it detaches the entire subtree.
pub struct Mount {
    root: Retained<HostView>,
    _host: Retained<PlatformView>,
    _content: Rc<Mounted>,
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
    let root = create_root(&mut env, host, &mut keepalive, mtm);
    #[cfg(target_os = "ios")]
    let controller = cocoa_ui::uikit::ViewController::new(mtm);
    #[cfg(target_os = "ios")]
    let root = {
        crate::theme::install_controller(&mut env, &controller, &mut keepalive);
        controller.host_view().retain()
    };
    keepalive.keep(crate::locale::install(&mut env, mtm));
    let parts = app(env).into_parts();
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
    for window in windows {
        keepalive.keep(crate::windows::realize(window, &env, mtm));
    }
    #[cfg(target_os = "macos")]
    {
        bind_root(&root, declaration, &env, &mut keepalive, mtm);
        keepalive.keep(crate::menus::install_declared(
            mtm,
            &cocoa_ui::appkit::Application::shared(mtm),
            &parts.menu_bar,
            &env,
        ));
    }
    #[cfg(target_os = "ios")]
    {
        let background = declaration.resolved_background(&env);
        let background_host = root.clone();
        keepalive.bind(&background, move |color| {
            let [r, g, b, a] = color.components;
            let color = cocoa_ui::uikit::colors::extended_linear_display_p3(
                f64::from(r),
                f64::from(g),
                f64::from(b),
                f64::from(a),
            );
            cocoa_ui::view::set_background_color(&background_host, Some(&color));
        });
        let declarations = std::iter::once(declaration).chain(windows).collect();
        crate::windows::declare(&scenes.state, declarations, &env, 1, mtm);
        keepalive.keep(parts.menu_bar);
    }
    cocoa_ui::view::layout_immediately(&root);
    crate::first_paint::mark(&root, &env);
    keepalive.keep(env);
    Box::into_raw(Box::new(Mount {
        root,
        _host: cocoa_ui::view::retain_base(host),
        _content: content,
        _keepalive: keepalive,
        #[cfg(target_os = "ios")]
        controller,
        #[cfg(target_os = "ios")]
        scenes,
    }))
    .cast()
}

#[cfg(target_os = "macos")]
fn create_root(
    env: &mut Environment,
    host: &PlatformView,
    keepalive: &mut KeepAlive,
    mtm: MainThreadMarker,
) -> Retained<HostView> {
    let application = cocoa_ui::appkit::Application::shared(mtm);
    let theme = Rc::new(crate::theme::install(env, application.color_scheme()));
    let observed = theme.clone();
    keepalive.keep(application.observe_color_scheme(move |scheme| {
        crate::theme::refresh(&observed, scheme);
    }));
    keepalive.keep(theme);
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
    let placed = content.clone();
    crate::inspector::install(root, env, keepalive);
    root.set_layout_handler(move |root| {
        let frame = crate::native_layout::content_frame(placed.view(), root);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the layout contract uses f32 points"
        )]
        let proposal = waterui_core::layout::ProposalSize::new(
            Some(frame.size.width as f32),
            Some(frame.size.height as f32),
        );
        crate::proposal::deliver(placed.view(), proposal);
        cocoa_ui::view::set_frame(placed.view(), frame);
        // Placement changes the child's bounds after the host's own layout.
        // Complete that child's pending layout before this placement finishes.
        cocoa_ui::view::layout_immediately(placed.view());
    });
    content
}

#[cfg(target_os = "macos")]
fn bind_root(
    root: &HostView,
    declaration: waterui::window::Window,
    env: &Environment,
    keepalive: &mut KeepAlive,
    mtm: MainThreadMarker,
) {
    use std::cell::RefCell;
    use waterui::SignalExt;
    let declaration = Rc::new(RefCell::new(declaration));
    let binding = Rc::new(RefCell::new(None));
    let observed_env = env.clone();
    let observed_binding = binding.clone();
    let attach = Rc::new(move |root: &HostView| {
        let Some(window) = cocoa_ui::view::window(root) else {
            observed_binding.borrow_mut().take();
            return;
        };
        let mut declaration = declaration.borrow_mut();
        let toolbar = declaration.toolbar.take();
        *observed_binding.borrow_mut() = Some(crate::windows::bind_root_window(
            window,
            &observed_env,
            &declaration.title,
            &declaration.frame,
            &declaration.state,
            toolbar,
            &declaration.style.computed(),
            &declaration.level,
            &declaration.attention,
            declaration.resize_increments.as_ref(),
            &declaration.background.computed(),
            declaration.closable,
            declaration.resizable,
            mtm,
        ));
    });
    let on_window = attach.clone();
    root.set_window_handler(move |root| on_window(root));
    attach(root);
    keepalive.keep(binding);
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

//! Native application startup. GPU setup completes on the main executor
//! before services, application declarations, and windows are realized.

use waterui::app::App;
use waterui_backend_core::Environment;

/// Runs `app` on `env`, which the caller's `configure_environment!` already
/// built — the tail of `waterui_apple_main`. `accessory` selects the macOS
/// activation policy and is unused on iOS.
///
/// Never returns: both platforms end in their run loop.
///
/// # Safety
///
/// Call once, on the platform's main thread, as the process's entry. `env`
/// is lent to native services for the rest of the process, so the
/// caller's frame must live that long — this function does not return, which
/// is what guarantees it.
pub unsafe fn run(
    app: impl FnOnce(Environment) -> App + 'static,
    env: &mut Environment,
    accessory: bool,
) -> ! {
    let inspector = crate::startup::initialize();
    waterui::inspector::install(env, inspector);
    waterui::text::install_system_font_collection(env);
    let mut fonts = crate::fonts::FontRegistrations::default();
    crate::resources::install_application(env, &mut fonts);
    crate::dispatch::install(env);
    env.insert(crate::first_paint::FirstPaint::default());
    #[cfg(target_os = "ios")]
    env.insert(crate::scene_registry::SceneRegistry::<crate::windows::Scenes>::default());
    imp::launch(app, env, accessory)
}

#[cfg(target_os = "macos")]
mod imp {
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use core::any::Any;
    use core::cell::OnceCell;
    use core::ffi::c_void;

    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::appkit::{
        ActivationPolicy, Application, ApplicationHandlers, ColorSchemeObservation, TerminateReply,
    };
    use waterui::app::App;
    use waterui_backend_core::Environment;

    use crate::termination::Session;
    use crate::theme::ThemeSignals;

    /// What GPU preparation finishes the launch with — everything alive at
    /// launch-handler time that must live for the rest of the process, plus
    /// the user's `app` and the environment it runs under.
    struct Launch {
        app: Option<Box<dyn FnOnce(Environment) -> App>>,
        env: *mut Environment,
        _theme: Rc<ThemeSignals>,
        _appearance: ColorSchemeObservation,
        _locale: Box<dyn Any>,
        /// Filled once `app(env)` declared its termination hooks; the
        /// application delegate's quit questions go through it.
        termination: Rc<OnceCell<Session>>,
    }

    /// GPU preparation has installed native services; `app(env)` now declares
    /// its windows under the native window manager.
    unsafe extern "C" fn prepared(context: *mut c_void) {
        let mtm = MainThreadMarker::new().expect("GPU completion runs on the main thread");
        // SAFETY: `context` is the `Launch` box transferred by GPU completion,
        // which calls this function exactly once.
        let mut launch = unsafe { Box::from_raw(context.cast::<Launch>()) };
        let app = launch
            .app
            .take()
            .expect("GPU completion must run exactly once");
        // SAFETY: `run` lent `env` for the process and never returned.
        let env = unsafe { &mut *launch.env };
        crate::windows::install_manager(env);
        let parts = app(env.clone()).into_parts();
        // Content renders under the environment `app` returned: its own
        // installs (`install_chromium`, `.state(..)` chains) landed as
        // overlays on the clone it was handed, which the host env cannot
        // see — `insert` never propagates between clones.
        // `mut` only serves the `webview` install below; without the port
        // nothing borrows `app_env` mutably.
        #[allow(unused_mut)]
        let mut app_env = parts.env;
        // The web view controller fills its slot late and only when the
        // `webview` port is enabled — an application bundling its own
        // engine installed it during `app(env)`.
        #[cfg(feature = "webview")]
        crate::components::webview::install_service(&mut app_env);
        // The machine installs `Quit` into `app_env` before the menus and
        // windows realize under it, so a declared `MenuItem::Quit` and a
        // `|quit: Quit|` action find it.
        let session = Session::start(parts.termination, &mut app_env, parts.last_window, mtm);
        assert!(
            launch.termination.set(session).is_ok(),
            "the termination machine starts once"
        );
        let session = launch
            .termination
            .get()
            .expect("the termination machine was just started");
        // `installMenuBar`: the declared menus resolve and rebuild under the
        // environment `app` returned, exactly as windows do. The guard lives
        // for the process.
        core::mem::forget(crate::menus::install_declared(
            mtm,
            &Application::shared(mtm),
            &parts.menu_bar,
            &app_env,
        ));
        if parts.windows.is_empty() {
            // A zero-window application acts on its policy at launch.
            session.no_window_left();
        } else {
            for window in parts.windows {
                let host = crate::windows::realize(window, &app_env, mtm);
                crate::windows::track(host);
            }
        }
        // The theme signals, the appearance observation and the locale
        // observer must outlive the process; `app` is already consumed.
        core::mem::forget(launch);
    }

    pub fn launch(
        app: impl FnOnce(Environment) -> App + 'static,
        env: &mut Environment,
        accessory: bool,
    ) -> ! {
        let mtm = MainThreadMarker::new().expect("waterui_apple_main runs on the main thread");
        let application = Application::shared(mtm);
        // AppKit can refuse policy changes during early startup; the bundle's
        // Info.plist still supplies the application's activation policy.
        let _ = application.set_activation_policy(if accessory {
            ActivationPolicy::Accessory
        } else {
            ActivationPolicy::Regular
        });
        crate::menus::install_default(mtm, &application);

        let theme = Rc::new(crate::theme::install(env, application.color_scheme()));
        let appearance = application.observe_color_scheme({
            let theme = Rc::clone(&theme);
            move |scheme| crate::theme::refresh(&theme, scheme)
        });
        let locale = crate::locale::install(env, mtm);

        let termination = Rc::new(OnceCell::new());
        let launch = Box::new(Launch {
            app: Some(Box::new(app)),
            env: core::ptr::from_mut(env),
            _theme: theme,
            _appearance: appearance,
            _locale: Box::new(locale),
            termination: Rc::clone(&termination),
        });

        let handlers = ApplicationHandlers::new()
            .did_finish_launching(move |_| {
                let env_ptr = launch.env;
                // SAFETY: `launch` is consumed by `prepared` exactly once —
                // this handler runs once — and `env` is `run`'s borrow, lent
                // for the process.
                // Install native services only after the GPU context is ready.
                unsafe {
                    crate::gpu_runtime::prepare(env_ptr, move || {
                        crate::embedding::install_services(&mut *env_ptr);
                        prepared(Box::into_raw(launch).cast::<c_void>());
                    });
                }
            })
            .should_terminate({
                let termination = Rc::clone(&termination);
                // A quit before `app(env)` ran has no hook to ask.
                move |_| {
                    termination
                        .get()
                        .map_or(TerminateReply::Now, Session::should_terminate)
                }
            })
            .should_terminate_after_last_window_closed(move |_| {
                termination
                    .get()
                    .expect("windows are realized after the termination machine starts")
                    .no_window_left();
                // `AppKit`'s own `terminate:` would arrive as a cancellable
                // quit; the session filed a required request instead.
                false
            });
        application.run(handlers);
        std::process::exit(0);
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use core::any::Any;
    use core::ffi::c_void;

    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::uikit::{self, ApplicationHandlers};
    use waterui::app::App;
    use waterui_backend_core::Environment;

    use crate::theme::ThemeSignals;

    /// The same hand-off as macOS's `Launch`; the appearance side of the
    /// theme arrives per scene instead, through each controller's trait
    /// observation.
    struct Launch {
        app: Option<Box<dyn FnOnce(Environment) -> App>>,
        env: *mut Environment,
        declared: crate::menus::Declared,
        scenes: crate::scene_registry::Registration<crate::windows::Scenes>,
        _theme: Rc<ThemeSignals>,
        _locale: Box<dyn Any>,
    }

    /// GPU completion: `app(env)` declares its windows, and each fills
    /// the scene already waiting for it — or queues for the next connection.
    unsafe extern "C" fn prepared(context: *mut c_void) {
        let mtm = MainThreadMarker::new().expect("GPU completion runs on the main thread");
        // SAFETY: `context` is the `Launch` box transferred by GPU completion,
        // which calls this function exactly once.
        let mut launch = unsafe { Box::from_raw(context.cast::<Launch>()) };
        let app = launch
            .app
            .take()
            .expect("GPU completion must run exactly once");
        // SAFETY: `run` lent `env` for the process and never returned.
        let env = unsafe { &mut *launch.env };
        crate::windows::install_manager(env);
        let parts = app(env.clone()).into_parts();
        assert!(
            !parts.windows.is_empty(),
            "an iOS application must declare at least one window"
        );
        // iOS ends the process without notice, so `on_quit_request` and
        // `on_terminate` are never called: the machine is never started and
        // no `Quit` is installed, so extracting `Quit` fails here.
        let _ = parts.termination;
        // Same hand-off as macOS: content renders under the env `app`
        // returned — its installs are invisible to the host env.
        // `mut` only serves the `webview` install below; without the port
        // nothing borrows `app_env` mutably.
        #[allow(unused_mut)]
        let mut app_env = parts.env;
        // The web view controller fills its slot late and only when the
        // `webview` port is enabled — an application bundling its own
        // engine installed it during `app(env)`.
        #[cfg(feature = "webview")]
        crate::components::webview::install_service(&mut app_env);
        // `installMenuBar`: `declared` feeds the `build_menus` handler the
        // delegate registered at launch; the watch rebuilds on every change.
        // The guard lives for the process.
        core::mem::forget(crate::menus::install_declared(
            &parts.menu_bar,
            &app_env,
            &launch.declared,
        ));
        crate::windows::declare(&launch.scenes.state, parts.windows, &app_env, 0, mtm);
        core::mem::forget(launch);
    }

    pub fn launch(
        app: impl FnOnce(Environment) -> App + 'static,
        env: &mut Environment,
        _accessory: bool,
    ) -> ! {
        let mtm = MainThreadMarker::new().expect("waterui_apple_main runs on the main thread");

        let theme = Rc::new(crate::theme::install(
            env,
            cocoa_ui::uikit::current_scheme(),
        ));
        let locale = crate::locale::install(env, mtm);

        // Filled by `prepared`: `build_menus` can fire before `app(env)`
        // runs, so the declared menus sit behind this slot.
        let declared = crate::menus::declared();
        let build_menus = crate::menus::build_handler(Rc::clone(&declared));
        let scenes = env
            .get::<crate::scene_registry::SceneRegistry<crate::windows::Scenes>>()
            .expect("application owns scene routing")
            .register(crate::windows::Scenes::default());
        let scene_state = scenes.state.clone();
        let launch = Box::new(Launch {
            app: Some(Box::new(app)),
            env: core::ptr::from_mut(env),
            declared,
            scenes,
            _theme: Rc::clone(&theme),
            _locale: Box::new(locale),
        });

        // Scenes may connect before the asynchronous preparation finishes:
        // `connect` builds the platform window eagerly and the declaration
        // fills it when `prepared` lands.
        let handlers = ApplicationHandlers::new(move |scene| {
            crate::windows::connect(&scene_state, scene, Rc::clone(&theme), mtm)
        })
        .build_menus(build_menus)
        .did_finish_launching(move |_| {
            let env_ptr = launch.env;
            // SAFETY: `launch` is consumed by `prepared` exactly once, and
            // `env` is `run`'s borrow, lent for the process.
            // Install native services only after the GPU context is ready.
            unsafe {
                crate::gpu_runtime::prepare(env_ptr, move || {
                    crate::embedding::install_services(&mut *env_ptr);
                    prepared(Box::into_raw(launch).cast::<c_void>());
                });
            }
        });
        uikit::run(mtm, handlers)
    }
}

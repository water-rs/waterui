//! GTK Application setup and lifecycle management.

use std::future::Future;

use executor_core::{
    LocalExecutor,
    async_task::{self, AsyncTask, Runnable},
    spawn_local, try_init_global_executor, try_init_local_executor,
};
use gtk4::Application;
use gtk4::prelude::*;
use native_executor::NativeExecutor;
use waterui::app::App;
use waterui_core::{Environment, View};

use crate::renderer::GtkRenderer;
use crate::util::{store_watcher_guards, subscribe_then_get};
#[cfg(feature = "webview-system")]
use crate::webview::ensure_webview_controller;
use crate::window::{apply_window_background, create_window, install_inspect_gesture};

#[derive(Debug, Clone, Copy, Default)]
struct GtkMainThreadExecutor;

impl LocalExecutor for GtkMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let (runnable, task) = async_task::spawn_local(fut, |runnable: Runnable| {
            glib::idle_add_local_once(move || {
                runnable.run();
            });
        });
        runnable.schedule();
        task
    }
}

/// Initialize executors for GTK apps on the main thread.
///
/// Returns the inspector endpoint, which the caller must keep alive and install
/// into the environment: dropping it shuts the endpoint down and withdraws the
/// advertisement that lets `water inspect` find this application.
#[must_use]
pub fn init_main_thread_executors() -> Option<waterui::inspector::InspectorRuntime> {
    // GTK apps run UI rendering on the main thread. Initialize executors there so
    // spawn/spawn_local paths used by reactive bindings are always available.
    let _ = try_init_global_executor(NativeExecutor::new());
    let inspector = waterui::inspector::maybe_init_from_env("gtk");
    let inspector_probe = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::runtime_probe);
    let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
        GtkMainThreadExecutor,
        inspector_probe,
    ));

    // Locale changes reach views through a mailbox, whose pump needs the
    // executor installed just above.
    waterui_locale::start_system_locale_listener();
    inspector
}

/// Makes the staged application icon resolvable through GTK icon-name
/// lookup.
///
/// The water CLI installs a hicolor icon tree named after the application id
/// next to the staged asset bundle; adding that tree to the display's icon
/// theme search path and using the id as the default window icon name lets
/// GTK pick the right size everywhere the icon appears. Without a staged
/// bundle (bare `cargo run`, tests) the theme simply has no icon with that
/// name and GTK falls back to its generic window icon.
fn install_app_icon(app_id: &str) {
    gtk4::Window::set_default_icon_name(app_id);
    let Ok(bundle_root) = waterui_assets::bundle_root() else {
        return;
    };
    let Some(resources_root) = bundle_root.parent() else {
        return;
    };
    let icons_dir = resources_root.join("icons");
    if !icons_dir.is_dir() {
        return;
    }
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::IconTheme::for_display(&display).add_search_path(icons_dir);
    }
}

/// GTK4 application wrapper for `WaterUI`.
#[derive(Debug)]
pub struct GtkApp {
    app: Application,
}

impl GtkApp {
    /// Creates a new GTK application.
    ///
    /// # Arguments
    ///
    /// * `app_id` - The application identifier (e.g., "com.example.myapp")
    #[must_use]
    pub fn new(app_id: &str) -> Self {
        let app = Application::builder().application_id(app_id).build();

        Self { app }
    }

    /// Runs the application with the provided root view.
    ///
    /// This method blocks until the application exits.
    ///
    /// # Panics
    ///
    /// Panics if the GPU runtime cannot be created.
    #[must_use = "the returned value is the process exit status"]
    pub fn run<V: View + Clone + 'static>(self, view: V, env: Environment) -> i32 {
        // `env` is only mutated when the system WebView bridge installs its
        // controller into it.
        #[cfg(feature = "webview-system")]
        let mut env = env;
        #[cfg(feature = "webview-system")]
        ensure_webview_controller(&mut env);
        let env = env;

        self.app.connect_activate(move |app| {
            if let Some(app_id) = app.application_id() {
                install_app_icon(app_id.as_str());
            }
            let inspector = init_main_thread_executors();
            let app = app.clone();
            let view = view.clone();
            let mut env = env.clone();
            waterui::inspector::install(&mut env, inspector);
            spawn_local(async move {
                let runtime = waterui_graphics::GpuRuntime::new()
                    .await
                    .unwrap_or_else(|error| panic!("GTK GPU runtime creation failed: {error}"));
                env.insert(runtime);
                let window = create_window(&app, "WaterUI App", 800, 600);
                crate::theme::install(&mut env, window.upcast_ref());
                install_inspect_gesture(&window, &env);
                let mut renderer = GtkRenderer::new();
                let widget = renderer.render(view, &env);
                window.set_child(Some(&widget));
                window.present();
            })
            .detach();
        });

        self.app.run().into()
    }

    /// Runs a `WaterUI` `App` as a GTK application.
    ///
    /// This extracts the main window's content and environment from the App
    /// and renders it using GTK.
    ///
    /// # Panics
    ///
    /// Panics if the app has no windows, or if the GPU runtime cannot be created.
    #[must_use = "the returned value is the process exit status"]
    pub fn run_app(self, waterui_app: App) -> i32 {
        let (windows, _menu_bar, env) = waterui_app.into_parts();
        // `env` is only mutated when the system WebView bridge installs its
        // controller into it.
        #[cfg(feature = "webview-system")]
        let mut env = env;
        let main_window = windows
            .into_iter()
            .next()
            .expect("GtkApp::run_app requires at least one window");
        let title = main_window.display_title();
        let background = main_window.background.clone();
        let content = main_window.content;
        #[cfg(feature = "webview-system")]
        ensure_webview_controller(&mut env);

        self.app.connect_activate(move |app| {
            let inspector = init_main_thread_executors();
            let app = app.clone();
            let content = content.build();
            let title = title.clone();
            let background = background.clone();
            let mut env = env.clone();
            waterui::inspector::install(&mut env, inspector);
            spawn_local(async move {
                let runtime = waterui_graphics::GpuRuntime::new()
                    .await
                    .unwrap_or_else(|error| panic!("GTK GPU runtime creation failed: {error}"));
                env.insert(runtime);
                let window = create_window(&app, "", 800, 600);
                crate::theme::install(&mut env, window.upcast_ref());
                install_inspect_gesture(&window, &env);
                apply_window_background(&window, &background, &env);

                let (initial_title, title_guard) = subscribe_then_get(&title, {
                    let window = window.clone();
                    move |ctx| {
                        let title_text = ctx.into_value().as_str().to_owned();
                        let window = window.clone();
                        glib::idle_add_local_once(move || {
                            window.set_title(Some(&title_text));
                        });
                    }
                });
                window.set_title(Some(initial_title.as_str()));
                store_watcher_guards(&window, vec![title_guard]);

                let mut renderer = GtkRenderer::new();
                let widget = renderer.render_any(content, &env);
                window.set_child(Some(&widget));
                window.present();
            })
            .detach();
        });

        self.app.run().into()
    }

    /// Returns a reference to the underlying GTK Application.
    #[must_use]
    pub const fn application(&self) -> &Application {
        &self.app
    }
}

impl Default for GtkApp {
    fn default() -> Self {
        Self::new("com.waterui.app")
    }
}

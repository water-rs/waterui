//! GTK Application setup and lifecycle management.

use std::future::Future;

use executor_core::{
    LocalExecutor,
    async_task::{self, AsyncTask, Runnable},
    try_init_global_executor, try_init_local_executor,
};
use gtk4::Application;
use gtk4::prelude::*;
use nami::Signal;
use native_executor::NativeExecutor;
use waterui::app::App;
use waterui_core::{Environment, View};

use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guards;
use crate::webview::ensure_webview_controller;
use crate::window::{apply_window_background, create_window};

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
pub fn init_main_thread_executors() {
    // GTK apps run UI rendering on the main thread. Initialize executors there so
    // spawn/spawn_local paths used by reactive bindings are always available.
    let _ = try_init_global_executor(NativeExecutor::new());
    let _ = waterui::inspector::maybe_init_from_env();
    let _ = try_init_local_executor(waterui::task::monitored_local_executor(
        GtkMainThreadExecutor,
    ));
}

/// GTK4 application wrapper for WaterUI.
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
    pub fn run<V: View + Clone + 'static>(self, view: V, mut env: Environment) -> i32 {
        let view = view.clone();
        ensure_webview_controller(&mut env);
        let env = env.clone();

        self.app.connect_activate(move |app| {
            init_main_thread_executors();
            let window = create_window(app, "WaterUI App", 800, 600);

            let mut renderer = GtkRenderer::new();
            let widget = renderer.render(view.clone(), &env);

            window.set_child(Some(&widget));
            window.present();
        });

        self.app.run().into()
    }

    /// Runs a WaterUI `App` as a GTK application.
    ///
    /// This extracts the main window's content and environment from the App
    /// and renders it using GTK.
    pub fn run_app(self, waterui_app: App) -> i32 {
        let (windows, _menu_bar, mut env) = waterui_app.into_parts();
        let main_window = windows
            .into_iter()
            .next()
            .expect("GtkApp::run_app requires at least one window");
        let title = main_window.title.clone();
        let background = main_window.background.clone();
        let content = main_window.content;
        ensure_webview_controller(&mut env);

        self.app.connect_activate(move |app| {
            init_main_thread_executors();
            let content = content.build();

            let initial_title: String = title.get().as_str().to_owned();
            let window = create_window(app, &initial_title, 800, 600);
            apply_window_background(&window, &background, &env);

            let title_guard = title.watch({
                let window = window.clone();
                move |ctx| {
                    let title_text = ctx.into_value().as_str().to_owned();
                    let window = window.clone();
                    glib::idle_add_local_once(move || {
                        window.set_title(Some(&title_text));
                    });
                }
            });
            store_watcher_guards(&window, vec![title_guard]);

            let mut renderer = GtkRenderer::new();
            let widget = renderer.render_any(content, &env);

            window.set_child(Some(&widget));
            window.present();
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

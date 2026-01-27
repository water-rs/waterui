//! GTK Application setup and lifecycle management.

use std::cell::RefCell;

use gtk4::Application;
use gtk4::prelude::*;
use nami::Signal;
use waterui::app::App;
use waterui_core::{Environment, View};

use crate::renderer::GtkRenderer;
use crate::webview::ensure_webview_controller;
use crate::window::create_window;

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
    pub fn run_app(self, mut waterui_app: App) -> i32 {
        let main_window = waterui_app.main_window_mut();
        let title: String = main_window.title.get().as_str().to_owned();
        // Use RefCell to allow taking the content once (AnyView is not Clone)
        // Take ownership of the content using mem::take
        let content = RefCell::new(Some(std::mem::take(&mut main_window.content)));
        let mut env = waterui_app.env;
        ensure_webview_controller(&mut env);

        self.app.connect_activate(move |app| {
            // Take the content or use default if already taken (re-activation)
            let content = content.borrow_mut().take().unwrap_or_default();

            let window = create_window(app, &title, 800, 600);

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

//! GTK Application setup and lifecycle management.

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};
use waterui_core::{AnyView, Environment, View};

use crate::renderer::GtkRenderer;
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
    pub fn run<V: View + Clone + 'static>(self, view: V, env: Environment) -> i32 {
        let view = view.clone();
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

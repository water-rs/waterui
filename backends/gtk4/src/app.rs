//! GTK4 application wrapper for WaterUI.

use gtk4::{Application, ApplicationWindow, prelude::*};
use std::rc::Rc;
use waterui::{Environment, View};

/// A GTK4 application wrapper for WaterUI.
pub struct Gtk4App {
    app: Application,
    env: Environment,
}

impl Gtk4App {
    /// Create a new GTK4 application with the given application ID.
    pub fn new(application_id: &str) -> Self {
        let app = Application::builder()
            .application_id(application_id)
            .build();

        Self {
            app,
            env: Environment::new(),
        }
    }

    /// Set the environment for the application.
    pub fn environment(mut self, env: Environment) -> Self {
        self.env = env;
        self
    }

    /// Run the application with the given content.
    pub fn run<F, V>(self, content: F) -> i32
    where
        F: Fn() -> V + 'static,
        V: View,
    {
        let env = Rc::new(self.env);

        self.app.connect_activate(move |app| {
            let window = ApplicationWindow::builder()
                .application(app)
                .title("WaterUI GTK4 App")
                .default_width(800)
                .default_height(600)
                .build();

            let content_view = content();
            let content_widget = crate::renderer::render(content_view, &env);

            window.set_child(Some(&content_widget));
            window.show();
        });

        self.app.run().into()
    }
}

use waterui_core::{Environment, View};

use crate::{
    error::TuiError,
    renderer::{RenderFrame, Renderer},
    terminal::Terminal,
};

/// Entry point for running `WaterUI` applications in the terminal.
#[derive(Debug)]
pub struct TuiApp {
    terminal: Terminal,
    renderer: Renderer,
    environment: Environment,
}

impl TuiApp {
    /// Renders a view and flushes it to the terminal.
    ///
    /// # Errors
    ///
    /// Returns an error when rendering the view tree fails or when the
    /// terminal cannot be written to.
    pub fn render<V: View>(&mut self, view: V) -> Result<(), TuiError> {
        let frame = self.renderer.render(&self.environment, view)?;
        self.terminal.render(&frame)
    }

    /// Renders a view tree to a frame without drawing it to the terminal.
    ///
    /// # Errors
    ///
    /// Propagates failures from the renderer when the view tree cannot be
    /// transformed into a terminal frame.
    pub fn render_to_frame<V: View>(&mut self, view: V) -> Result<RenderFrame, TuiError> {
        self.renderer.render(&self.environment, view)
    }

    /// Provides mutable access to the renderer.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// Provides immutable access to the renderer.
    #[must_use]
    pub const fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Provides mutable access to the underlying environment.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn environment_mut(&mut self) -> &mut Environment {
        &mut self.environment
    }

    /// Provides immutable access to the environment.
    #[must_use]
    pub const fn environment(&self) -> &Environment {
        &self.environment
    }

    /// Provides mutable access to the terminal handle.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn terminal_mut(&mut self) -> &mut Terminal {
        &mut self.terminal
    }

    /// Provides immutable access to the terminal handle.
    #[must_use]
    pub const fn terminal(&self) -> &Terminal {
        &self.terminal
    }
}

/// Builder for [`TuiApp`] instances.
#[derive(Debug, Default)]
pub struct TuiAppBuilder {
    terminal: Option<Terminal>,
    environment: Environment,
}

impl TuiAppBuilder {
    /// Creates a new builder with default configuration.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn new() -> Self {
        Self {
            terminal: None,
            environment: Environment::new(),
        }
    }

    /// Overrides the terminal handle used by the application.
    #[must_use]
    pub fn with_terminal(mut self, terminal: Terminal) -> Self {
        self.terminal = Some(terminal);
        self
    }

    /// Replaces the environment used for rendering.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// Consumes the builder and produces a [`TuiApp`].
    ///
    /// # Errors
    ///
    /// Returns an error if the terminal cannot be initialised.
    pub fn build(self) -> Result<TuiApp, TuiError> {
        let terminal = match self.terminal {
            Some(terminal) => terminal,
            None => Terminal::stdout()?,
        };

        Ok(TuiApp {
            terminal,
            renderer: Renderer::new(),
            environment: self.environment,
        })
    }
}

impl From<TuiAppBuilder> for TuiApp {
    fn from(value: TuiAppBuilder) -> Self {
        value
            .build()
            .expect("TuiAppBuilder::build should succeed when using default configuration")
    }
}

impl Default for TuiApp {
    fn default() -> Self {
        TuiAppBuilder::new()
            .build()
            .expect("TuiAppBuilder::build should succeed with default terminal")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_text::text;

    #[test]
    fn render_into_buffer() {
        let terminal = Terminal::buffered();
        let mut app = TuiAppBuilder::new()
            .with_terminal(terminal)
            .build()
            .expect("building app should succeed");
        app.render(text("Hello TUI")).expect("rendering should succeed");
        let snapshot = app.terminal().snapshot().expect("buffered terminal");
        assert!(std::str::from_utf8(snapshot).expect("snapshot should be valid utf8").contains("Hello TUI"));
    }
}

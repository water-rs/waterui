//! Module defining the `Window` struct for UI windows.

use nami::{Binding, Computed, impl_constant, signal::IntoComputed};
use waterui_core::{AnyView, View};
use waterui_layout::{Point, Rect, Size};
use waterui_str::Str;

/// Represents a window in the UI.
#[derive(Debug)]
pub struct Window {
    /// The title of the window.
    ///
    /// Notice that it may not be displayed on all platforms.
    pub title: Computed<Str>,
    /// Whether the window is closable.
    ///
    /// Notice that it may not be supported on all platforms.
    pub closable: bool,
    /// The frame of the window.
    ///
    /// Notice that it may not be supported on all platforms.
    pub frame: Binding<Rect>,
    /// The content of the window.
    pub content: AnyView,
    /// The current state of the window.
    pub state: Binding<WindowState>,
}

/// The state of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowState {
    /// The window is in its normal state.
    #[default]
    Normal,
    /// The window is closed.
    Closed,
    /// The window is minimized.
    Minimized,
    /// The window is maximized to fullscreen.
    Fullscreen,
}

impl_constant!(WindowState);

impl Window {
    /// Create a new window instance with the specified title and content.
    ///
    /// Notice that would not show this window immediately
    #[must_use]
    pub fn new(title: impl IntoComputed<Str>, content: impl View) -> Self {
        let default_frame = Rect::new(Point::zero(), Size::new(800.0, 600.0));
        Self {
            title: title.into_computed(),
            closable: true,
            frame: Binding::container(default_frame),
            content: AnyView::new(content),
            state: Binding::default(),
        }
    }

    /// Get a handle to control the window after showing it.
    pub fn handle(&self) -> WindowHandle {
        WindowHandle {
            frame: self.frame.clone(),
            state: self.state.clone(),
        }
    }

    /// Show the window on screen.
    pub fn show(self) {
        todo!()
    }
}

/// A handle to control a window after it has been shown.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    frame: Binding<Rect>,
    state: Binding<WindowState>,
}

impl WindowHandle {
    /// Close the window.
    pub fn close(self) {
        self.state.set(WindowState::Closed);
    }

    /// Minimize the window.
    pub fn minimize(&self) {
        self.state.set(WindowState::Minimized);
    }

    /// Maximize the window to fullscreen.
    pub fn fullscreen(&self) {
        self.state.set(WindowState::Fullscreen);
    }

    /// Restore the window to its normal state.
    pub fn restore(&self) {
        self.state.set(WindowState::Normal);
    }

    /// Set the frame of the window.
    pub fn set_frame(&self, frame: Rect) {
        self.frame.set(frame);
    }
}

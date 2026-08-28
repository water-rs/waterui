use std::cell::Cell;
use std::rc::Rc;

use waterui_graphics::gpu_surface::GpuFrame;

use crate::CefPageHandle;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use crate::input::{CefInputGpuView, CefSurfaceInput};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod presenter;
#[cfg(target_os = "windows")]
mod windows;

/// Shared device scale used by the host layout and CEF OSR viewport.
#[derive(Debug, Clone)]
pub struct CefViewport(Rc<Cell<f64>>);

impl CefViewport {
    /// Creates a viewport at scale one.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(Cell::new(1.0)))
    }

    /// Updates the browser device scale.
    ///
    /// # Panics
    ///
    /// Panics when `scale` is not positive and finite.
    pub fn set_scale(&self, scale: f64) {
        assert!(
            scale.is_finite() && scale > 0.0,
            "CEF viewport scale must be finite and positive"
        );
        self.0.set(scale);
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    pub(crate) fn scale(&self) -> f64 {
        self.0.get()
    }
}

impl Default for CefViewport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
fn request_browser_frame(page: &CefPageHandle, _frame: &mut GpuFrame<'_>) {
    page.request_frame();
}

#[cfg(not(target_os = "macos"))]
fn request_browser_frame(page: &CefPageHandle, frame: &mut GpuFrame<'_>) {
    page.request_frame();
    frame.request_redraw();
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
#[cfg(target_os = "linux")]
#[must_use]
pub fn gpu_view(
    page: CefPageHandle,
    viewport: CefViewport,
) -> impl waterui_graphics::gpu_surface::GpuView {
    linux::gpu_view(page, viewport)
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
#[cfg(target_os = "macos")]
#[must_use]
pub fn gpu_view(
    page: CefPageHandle,
    viewport: CefViewport,
) -> impl waterui_graphics::gpu_surface::GpuView {
    macos::CefGpuView::new(page, viewport)
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
#[cfg(target_os = "windows")]
#[must_use]
pub fn gpu_view(
    page: CefPageHandle,
    viewport: CefViewport,
) -> impl waterui_graphics::gpu_surface::GpuView {
    windows::CefGpuView::new(page, viewport)
}

/// Creates the presenter for one visible CEF page, wired to take its own input.
///
/// The view reports
/// [`wants_input_events`](waterui_graphics::gpu_surface::GpuView::wants_input_events),
/// so a backend that routes surface input to GPU views needs nothing
/// CEF-specific: the pointer, keyboard, scroll and composition events landing
/// on this layer reach Chromium through [`CefSurfaceInput`]. A backend whose
/// input arrives somewhere else entirely — GTK delivers it to the `GtkGLArea`'s
/// event controllers — uses [`gpu_view`] and owns a [`CefSurfaceInput`] beside
/// it instead.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[must_use]
pub fn gpu_view_with_input(
    page: CefPageHandle,
    viewport: CefViewport,
) -> impl waterui_graphics::gpu_surface::GpuView {
    CefInputGpuView::new(gpu_view(page.clone(), viewport), CefSurfaceInput::new(page))
}

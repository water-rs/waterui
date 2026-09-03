#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use num_traits::ToPrimitive as _;

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

/// Sizes the browser's off-screen viewport to `frame` and returns its scale.
///
/// The device-pixel ratio is the frame's own — every backend already tells the
/// surface how many physical pixels a logical unit spans, so there is nothing
/// for a host to remember to publish separately.
///
/// # Panics
///
/// Panics when the logical viewport does not fit a `u32`, or the scale a `f32`.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn sync_browser_viewport(page: &CefPageHandle, frame: &GpuFrame<'_>) -> f64 {
    let scale = frame.scale();
    let logical_width = (f64::from(frame.width) / scale).round().max(1.0);
    let logical_height = (f64::from(frame.height) / scale).round().max(1.0);
    page.set_viewport(
        logical_width
            .to_u32()
            .expect("CEF logical width exceeds u32"),
        logical_height
            .to_u32()
            .expect("CEF logical height exceeds u32"),
        scale.to_f32().expect("CEF scale exceeds f32"),
    );
    scale
}

/// Asks Chromium for a frame, and asks the surface for the next one.
///
/// The browser is created with external begin frames, so it composites only
/// when told to: this call is its clock, and the surface's own redraw loop is
/// what ticks it. Every render therefore requests the next, and the loop runs at
/// the display's rate for as long as the surface is attached — on every
/// platform, because there is no other clock. macOS used to leave the redraw
/// out and rely on a display link the Apple host ran beside the surface for
/// exactly this purpose; that host now routes nothing CEF-specific, so the
/// pacing lives here with everything else the presenter needs.
fn request_browser_frame(page: &CefPageHandle, frame: &mut GpuFrame<'_>) {
    page.request_frame();
    frame.request_redraw();
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
#[cfg(target_os = "linux")]
#[must_use]
pub fn gpu_view(page: CefPageHandle) -> impl waterui_graphics::gpu_surface::GpuView {
    linux::gpu_view(page)
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
#[cfg(target_os = "macos")]
#[must_use]
pub fn gpu_view(page: CefPageHandle) -> impl waterui_graphics::gpu_surface::GpuView {
    macos::CefGpuView::new(page)
}

/// Creates the target-specific GPU-only presenter for one visible CEF page.
#[cfg(target_os = "windows")]
#[must_use]
pub fn gpu_view(page: CefPageHandle) -> impl waterui_graphics::gpu_surface::GpuView {
    windows::CefGpuView::new(page)
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
pub fn gpu_view_with_input(page: CefPageHandle) -> impl waterui_graphics::gpu_surface::GpuView {
    CefInputGpuView::new(gpu_view(page.clone()), CefSurfaceInput::new(page))
}

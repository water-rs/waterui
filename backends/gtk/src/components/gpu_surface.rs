//! GTK4 GpuSurface component implementation.
//!
//! This module provides GPU-accelerated rendering for GTK using wgpu.
//! The implementation uses a `gtk4::GLArea` widget and manages the wgpu
//! surface lifecycle (setup, render, resize, cleanup).
//!
//! # Platform Support
//!
//! - **Linux X11**: Enable with `x11` feature
//! - **Linux Wayland**: Enable with `wayland` feature
//! - **macOS**: Enable with `macos` feature
//!
//! At least one platform feature must be enabled for GPU rendering to work.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_graphics::gpu_surface::GpuSurface;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

// The code below is kept for future reference when GPU rendering is properly implemented.
// Currently, GPU rendering via wgpu is not working due to platform-specific issues.

#[allow(dead_code)]
use waterui_graphics::gpu_surface::{
    preferred_surface_format, GpuContext, GpuFrame, GpuRenderer,
};

/// Internal state for a GPU surface widget.
#[allow(dead_code)]
struct GpuSurfaceState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Box<dyn GpuRenderer>,
    initialized: bool,
    current_width: u32,
    current_height: u32,
}

#[allow(dead_code)]
impl GpuSurfaceState {
    /// Render a frame.
    fn render(&mut self) {
        let width = self.current_width;
        let height = self.current_height;

        if width == 0 || height == 0 {
            return;
        }

        // Call setup on first render
        if !self.initialized {
            let ctx = GpuContext {
                device: &self.device,
                queue: &self.queue,
                surface_format: self.config.format,
            };
            self.renderer.setup(&ctx);
            self.initialized = true;
        }

        // Get next frame texture
        let output = match self.surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                tracing::debug!("[GpuSurface] surface lost/outdated, reconfiguring");
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    Ok(o) => o,
                    Err(e) => {
                        tracing::error!(
                            "[GpuSurface] render failed: could not get texture after reconfigure: {e}"
                        );
                        return;
                    }
                }
            }
            Err(wgpu::SurfaceError::Timeout) => {
                // Surface isn't ready yet; skip this frame.
                return;
            }
            Err(e) => {
                tracing::error!("[GpuSurface] render failed: could not get current texture: {e}");
                return;
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("GpuSurface Frame View"),
            format: Some(self.config.format),
            ..Default::default()
        });

        // Create frame data
        let frame = GpuFrame {
            device: &self.device,
            queue: &self.queue,
            texture: &output.texture,
            view,
            format: self.config.format,
            width,
            height,
        };

        // Call user's render callback
        self.renderer.render(&frame);

        // Present
        output.present();
    }

    /// Handle resize.
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        if width != self.current_width || height != self.current_height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.current_width = width;
            self.current_height = height;
            self.renderer.resize(width, height);
        }
    }
}

/// Initialize wgpu for a GTK native surface.
#[allow(dead_code)]
fn init_wgpu_from_native(
    native: &gtk4::Native,
    width: u32,
    height: u32,
    renderer: Box<dyn GpuRenderer>,
) -> Option<GpuSurfaceState> {
    let gdk_surface = native.surface()?;
    let display = gdk_surface.display();

    // Determine backends based on platform
    #[cfg(all(unix, not(target_os = "macos")))]
    let backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    #[cfg(target_os = "macos")]
    let backends = wgpu::Backends::METAL;
    #[cfg(not(any(unix, target_os = "macos")))]
    let backends = wgpu::Backends::all();

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });

    // Create surface from GDK surface using platform-specific code
    let wgpu_surface = create_surface_from_gdk(&instance, &gdk_surface, &display)?;

    // Request adapter
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&wgpu_surface),
        force_fallback_adapter: false,
    }))
    .ok()?;

    tracing::info!("[GpuSurface] adapter: {:?}", adapter.get_info());

    // Request device and queue
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("WaterUI GTK GpuSurface Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        },
    ))
    .ok()?;

    // Configure surface
    let surface_caps = wgpu_surface.get_capabilities(&adapter);
    let format = preferred_surface_format(&surface_caps);

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: surface_caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Opaque),
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };

    wgpu_surface.configure(&device, &config);

    Some(GpuSurfaceState {
        device,
        queue,
        surface: wgpu_surface,
        config,
        renderer,
        initialized: false,
        current_width: width,
        current_height: height,
    })
}

// =============================================================================
// Linux X11 Implementation
// =============================================================================

#[cfg(all(unix, not(target_os = "macos"), feature = "x11"))]
#[allow(dead_code)]
fn try_create_x11_surface(
    instance: &wgpu::Instance,
    gdk_surface: &gdk4::Surface,
    display: &gdk4::Display,
) -> Option<wgpu::Surface<'static>> {
    use gdk4::prelude::*;
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle};
    use std::ptr::NonNull;

    // Check if this is an X11 surface
    let x11_surface = gdk_surface.downcast_ref::<gdk4_x11::X11Surface>()?;
    let x11_display = display.downcast_ref::<gdk4_x11::X11Display>()?;

    let xid = x11_surface.xid();
    let xdisplay = x11_display.xdisplay();

    let display_ptr = NonNull::new(xdisplay.cast())?;
    let display_handle = XlibDisplayHandle::new(Some(display_ptr), 0);
    let window_handle = XlibWindowHandle::new(xid as _);

    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Xlib(display_handle),
                raw_window_handle: RawWindowHandle::Xlib(window_handle),
            })
            .ok()
    }
}

#[cfg(not(all(unix, not(target_os = "macos"), feature = "x11")))]
#[allow(dead_code)]
fn try_create_x11_surface(
    _instance: &wgpu::Instance,
    _gdk_surface: &gdk4::Surface,
    _display: &gdk4::Display,
) -> Option<wgpu::Surface<'static>> {
    None
}

// =============================================================================
// Linux Wayland Implementation
// =============================================================================

#[cfg(all(unix, not(target_os = "macos"), feature = "wayland"))]
#[allow(dead_code)]
fn try_create_wayland_surface(
    instance: &wgpu::Instance,
    gdk_surface: &gdk4::Surface,
    display: &gdk4::Display,
) -> Option<wgpu::Surface<'static>> {
    use gdk4::prelude::*;
    use raw_window_handle::{
        RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
    };
    use std::ptr::NonNull;

    // Check if this is a Wayland surface
    let wayland_surface = gdk_surface.downcast_ref::<gdk4_wayland::WaylandSurface>()?;
    let wayland_display = display.downcast_ref::<gdk4_wayland::WaylandDisplay>()?;

    let wl_surface = wayland_surface.wl_surface();
    let wl_display = wayland_display.wl_display();

    // Get raw pointers from wayland-client objects
    let surface_ptr = NonNull::new(wl_surface.id().as_ptr().cast_mut().cast())?;
    let display_ptr = NonNull::new(wl_display.id().as_ptr().cast_mut().cast())?;

    let display_handle = WaylandDisplayHandle::new(display_ptr);
    let window_handle = WaylandWindowHandle::new(surface_ptr);

    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Wayland(display_handle),
                raw_window_handle: RawWindowHandle::Wayland(window_handle),
            })
            .ok()
    }
}

#[cfg(not(all(unix, not(target_os = "macos"), feature = "wayland")))]
#[allow(dead_code)]
fn try_create_wayland_surface(
    _instance: &wgpu::Instance,
    _gdk_surface: &gdk4::Surface,
    _display: &gdk4::Display,
) -> Option<wgpu::Surface<'static>> {
    None
}

// =============================================================================
// macOS Implementation
// =============================================================================

#[cfg(all(target_os = "macos", feature = "macos"))]
#[allow(dead_code)]
fn try_create_macos_surface(
    instance: &wgpu::Instance,
    gdk_surface: &gdk4::Surface,
    _display: &gdk4::Display,
) -> Option<wgpu::Surface<'static>> {
    use gdk4::prelude::*;
    use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle};
    use std::ptr::NonNull;

    // Check if this is a macOS surface
    let macos_surface = gdk_surface.downcast_ref::<gdk4_macos::MacosSurface>()?;

    // Get the NSView from the macOS surface
    let ns_view = macos_surface.native();
    let view_ptr = NonNull::new(ns_view as *mut _)?;

    let display_handle = AppKitDisplayHandle::new();
    let window_handle = AppKitWindowHandle::new(view_ptr);

    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::AppKit(display_handle),
                raw_window_handle: RawWindowHandle::AppKit(window_handle),
            })
            .ok()
    }
}

#[cfg(not(all(target_os = "macos", feature = "macos")))]
#[allow(dead_code)]
fn try_create_macos_surface(
    _instance: &wgpu::Instance,
    _gdk_surface: &gdk4::Surface,
    _display: &gdk4::Display,
) -> Option<wgpu::Surface<'static>> {
    None
}

// =============================================================================
// Unified Surface Creation
// =============================================================================

/// Create a wgpu surface from a GDK surface.
///
/// Tries platform-specific backends in order:
/// - Linux: X11, then Wayland
/// - macOS: AppKit/Metal
#[allow(dead_code)]
fn create_surface_from_gdk(
    instance: &wgpu::Instance,
    gdk_surface: &gdk4::Surface,
    display: &gdk4::Display,
) -> Option<wgpu::Surface<'static>> {
    // Try X11 first (Linux)
    if let Some(surface) = try_create_x11_surface(instance, gdk_surface, display) {
        tracing::info!("[GpuSurface] Created X11 surface");
        return Some(surface);
    }

    // Try Wayland (Linux)
    if let Some(surface) = try_create_wayland_surface(instance, gdk_surface, display) {
        tracing::info!("[GpuSurface] Created Wayland surface");
        return Some(surface);
    }

    // Try macOS
    if let Some(surface) = try_create_macos_surface(instance, gdk_surface, display) {
        tracing::info!("[GpuSurface] Created macOS surface");
        return Some(surface);
    }

    // No platform support available
    #[cfg(all(unix, not(target_os = "macos")))]
    tracing::error!(
        "[GpuSurface] No platform backend available. Enable 'x11' or 'wayland' feature."
    );

    #[cfg(target_os = "macos")]
    tracing::error!("[GpuSurface] No platform backend available. Enable 'macos' feature.");

    #[cfg(not(any(unix, target_os = "macos")))]
    tracing::error!("[GpuSurface] Platform not supported for GPU rendering.");

    None
}

// =============================================================================
// GtkComponent Implementation
// =============================================================================

impl GtkComponent for Native<GpuSurface> {
    /// Renders a `WaterUI` `GpuSurface` as a GTK4 widget.
    ///
    /// NOTE: GPU rendering via wgpu is not fully supported on GTK4 macOS yet
    /// due to incompatibilities between Metal and GTK's rendering model.
    /// This implementation shows a placeholder on macOS and attempts wgpu
    /// rendering on Linux where it's more likely to work.
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let _gpu_surface = self.into_inner();

        // GPU rendering via wgpu is complex on GTK4 and has platform-specific issues.
        // For now, show a placeholder with an explanation.
        //
        // The issues are:
        // - macOS: wgpu uses Metal which is incompatible with GTK's Cairo/OpenGL rendering
        // - Linux: Wayland compositor restrictions make raw surface access difficult
        //
        // A proper solution would require either:
        // 1. Using GTK's GLArea with OpenGL backend for wgpu (not trivial)
        // 2. Creating a separate window/overlay for Metal rendering
        // 3. Using a different approach like rendering to a texture and displaying it

        let placeholder = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        placeholder.set_hexpand(true);
        placeholder.set_vexpand(true);
        placeholder.set_halign(gtk4::Align::Center);
        placeholder.set_valign(gtk4::Align::Center);

        let label = gtk4::Label::new(Some("GPU Surface"));
        label.add_css_class("title-2");
        placeholder.append(&label);

        let note = gtk4::Label::new(Some("(GPU rendering not yet supported in GTK4 backend)"));
        note.add_css_class("dim-label");
        placeholder.append(&note);

        // Add a gray background to indicate the GPU surface area
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_data(
            r"
            box {
                background-color: #2d2d2d;
                border-radius: 8px;
                padding: 16px;
            }
            label.title-2 {
                font-weight: bold;
                font-size: 18px;
                color: #888;
            }
            label.dim-label {
                font-size: 12px;
                color: #666;
            }
            ",
        );

        gtk4::style_context_add_provider_for_display(
            &gdk4::Display::default().expect("display"),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        tracing::warn!(
            "[GpuSurface] GPU rendering is not supported in GTK4 backend yet. Showing placeholder."
        );

        placeholder.upcast()
    }
}

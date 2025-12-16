//! GTK4 GpuSurface component implementation.
//!
//! This module provides GPU-accelerated rendering for GTK using wgpu.
//!
//! # Platform Support
//!
//! - **Linux X11**: Enable with `x11` feature - Full GPU rendering support
//! - **Linux Wayland**: Enable with `wayland` feature - Full GPU rendering support
//! - **macOS**: Placeholder only (wgpu GL backend requires ANGLE which is not bundled)

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_graphics::gpu_surface::GpuSurface;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

// macOS: Show placeholder (wgpu GL requires ANGLE)
#[cfg(target_os = "macos")]
impl GtkComponent for Native<GpuSurface> {
    /// Renders a placeholder for `GpuSurface` on macOS.
    ///
    /// wgpu's GL backend requires ANGLE on macOS, which is not typically installed.
    /// A placeholder label is shown instead.
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        tracing::warn!(
            "[GpuSurface] GPU rendering not supported on macOS GTK4 (requires ANGLE). Showing placeholder."
        );

        let label = gtk4::Label::new(Some("GpuSurface: Not supported on macOS GTK4"));
        label.set_hexpand(true);
        label.set_vexpand(true);
        label.add_css_class("dim-label");
        label.upcast()
    }
}

// Linux: Full GPU rendering with GLArea + wgpu GLES
#[cfg(not(target_os = "macos"))]
mod linux_impl {
    use std::cell::RefCell;
    use std::num::NonZeroU32;
    use std::rc::Rc;

    use glow::HasContext;
    use gtk4::prelude::*;
    use gtk4::Widget;
    use waterui_core::{Environment, Native};
    use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

    use crate::component::GtkComponent;
    use crate::renderer::GtkRenderer;

    /// Internal state for a GPU surface widget using GLES backend.
    struct GlesGpuState {
        // wgpu resources (using GLES backend)
        device: wgpu::Device,
        queue: wgpu::Queue,

        // External framebuffer texture (bound to GLArea's FBO)
        framebuffer_texture: wgpu::Texture,
        framebuffer_view: wgpu::TextureView,
        format: wgpu::TextureFormat,

        // glow context for GL operations
        gl: glow::Context,

        // Current framebuffer ID (to detect changes)
        current_fbo_id: u32,

        // User renderer
        renderer: Box<dyn GpuRenderer>,
        initialized: bool,

        // Dimensions
        width: u32,
        height: u32,
    }

    impl GlesGpuState {
        /// Check if framebuffer changed and recreate texture wrapper if needed.
        fn update_framebuffer(&mut self) {
            let fbo_id =
                unsafe { self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) as u32 };

            // If framebuffer changed, recreate texture wrapper
            if fbo_id != self.current_fbo_id && fbo_id != 0 {
                tracing::debug!(
                    "[GpuSurface] Framebuffer changed: {} -> {}",
                    self.current_fbo_id,
                    fbo_id
                );
                self.current_fbo_id = fbo_id;

                // Recreate texture from new framebuffer
                if let Ok(new_texture) = create_texture_from_external_framebuffer(
                    &self.device,
                    glow::NativeFramebuffer(NonZeroU32::new(fbo_id).unwrap()),
                    self.width,
                    self.height,
                    self.format,
                ) {
                    self.framebuffer_texture = new_texture;
                    self.framebuffer_view = self
                        .framebuffer_texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                }
            }
        }

        /// Render a frame.
        fn render_frame(&mut self) {
            // Setup renderer on first frame
            if !self.initialized {
                tracing::debug!(
                    "[GpuSurface] Setting up renderer ({}x{})",
                    self.width,
                    self.height
                );
                let ctx = GpuContext {
                    device: &self.device,
                    queue: &self.queue,
                    surface_format: self.format,
                };
                self.renderer.setup(&ctx);
                self.initialized = true;
                tracing::debug!("[GpuSurface] Renderer setup complete");
            }

            // Create frame data pointing to GTK's framebuffer
            let frame = GpuFrame {
                device: &self.device,
                queue: &self.queue,
                texture: &self.framebuffer_texture,
                view: self.framebuffer_view.clone(),
                format: self.format,
                width: self.width,
                height: self.height,
            };

            // User's render callback - renders directly to GTK's FBO
            self.renderer.render(&frame);
        }

        /// Handle resize.
        fn resize(&mut self, width: u32, height: u32) {
            if width == 0 || height == 0 || (width == self.width && height == self.height) {
                return;
            }

            self.width = width;
            self.height = height;

            // Notify user renderer
            self.renderer.resize(width, height);

            // GTK manages framebuffer resize; we update dimensions
            // The texture wrapper will be recreated on next render if FBO changed
        }
    }

    /// Initialize epoxy library and return a reference to it.
    fn init_epoxy() -> Result<&'static libloading::Library, String> {
        use std::sync::OnceLock;

        static EPOXY_LIB: OnceLock<Option<libloading::Library>> = OnceLock::new();

        let lib = EPOXY_LIB.get_or_init(|| {
            // Try multiple paths to find libepoxy on Linux
            let paths = [
                "libepoxy.so.0",
                "/usr/lib/x86_64-linux-gnu/libepoxy.so.0",
                "/usr/lib/libepoxy.so.0",
                "/usr/local/lib/libepoxy.so.0",
            ];

            for path in paths {
                if let Ok(lib) = unsafe { libloading::Library::new(path) } {
                    tracing::debug!("[GpuSurface] Loaded libepoxy from: {}", path);
                    epoxy::load_with(|name| unsafe {
                        lib.get::<*const std::ffi::c_void>(name.as_bytes())
                            .map(|sym| *sym)
                            .unwrap_or(std::ptr::null())
                    });
                    return Some(lib);
                }
            }
            tracing::warn!("[GpuSurface] Failed to load libepoxy from any path");
            None
        });

        lib.as_ref()
            .ok_or_else(|| "Failed to load libepoxy library".to_string())
    }

    /// Initialize wgpu with GLES backend for GTK4 GLArea.
    fn init_wgpu_gles(
        gl_area: &gtk4::GLArea,
        width: u32,
        height: u32,
        renderer: Box<dyn GpuRenderer>,
    ) -> Result<GlesGpuState, String> {
        // Load libepoxy library first
        init_epoxy()?;

        // Create glow context from GTK's GL loader (epoxy)
        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                epoxy::get_proc_addr(s).cast::<std::ffi::c_void>()
            })
        };

        // Force GLES backend for GTK4 compatibility
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            ..Default::default()
        });

        // Request adapter
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("No GLES adapter available: {e}"))?;

        tracing::info!("[GpuSurface] Using adapter: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("WaterUI GTK GpuSurface Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e: wgpu::RequestDeviceError| e.to_string())?;

        // Get GTK's framebuffer ID
        gl_area.attach_buffers();
        let fbo_id = unsafe { gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) as u32 };

        tracing::debug!("[GpuSurface] GTK framebuffer ID: {}", fbo_id);

        if fbo_id == 0 {
            tracing::warn!("[GpuSurface] Framebuffer ID is 0, trying default framebuffer");
            // On some systems, the default framebuffer (0) may be valid
        }

        // Create wgpu texture from external framebuffer
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let framebuffer_texture = create_texture_from_external_framebuffer(
            &device,
            glow::NativeFramebuffer(NonZeroU32::new(fbo_id).unwrap()),
            width,
            height,
            format,
        )?;

        let framebuffer_view =
            framebuffer_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Ok(GlesGpuState {
            device,
            queue,
            framebuffer_texture,
            framebuffer_view,
            format,
            gl,
            current_fbo_id: fbo_id,
            renderer,
            initialized: false,
            width,
            height,
        })
    }

    /// Create a wgpu::Texture from GTK's framebuffer using HAL.
    fn create_texture_from_external_framebuffer(
        device: &wgpu::Device,
        fbo: glow::NativeFramebuffer,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<wgpu::Texture, String> {
        use wgpu_hal::CopyExtent;
        use wgpu_hal::gles::{Texture as HalTexture, TextureFormatDesc, TextureInner};

        // Construct HAL texture with external framebuffer
        let hal_texture = HalTexture {
            inner: TextureInner::ExternalNativeFramebuffer { inner: fbo },
            mip_level_count: 1,
            array_layer_count: 1,
            format: format.into(),
            format_desc: TextureFormatDesc {
                internal: 0, // Not needed for external framebuffer
                external: 0,
                data_type: 0,
            },
            copy_size: CopyExtent {
                width,
                height,
                depth: 1,
            },
            drop_guard: None, // GTK manages the framebuffer lifetime
        };

        // Wrap HAL texture in wgpu::Texture
        let desc = wgpu::TextureDescriptor {
            label: Some("GpuSurface External Framebuffer"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };

        // SAFETY: hal_texture is valid for the lifetime of GTK's GLArea
        let texture =
            unsafe { device.create_texture_from_hal::<wgpu_hal::gles::Api>(hal_texture, &desc) };

        Ok(texture)
    }

    impl GtkComponent for Native<GpuSurface> {
        /// Renders a `WaterUI` `GpuSurface` as a GTK4 GLArea widget with wgpu GLES rendering.
        fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
            let gpu_surface = self.into_inner();
            let user_renderer = gpu_surface.renderer;

            // Use GLArea for OpenGL rendering
            let gl_area = gtk4::GLArea::new();
            gl_area.set_hexpand(true);
            gl_area.set_vexpand(true);
            gl_area.set_auto_render(false); // We control render timing
            gl_area.set_required_version(3, 3); // OpenGL 3.3+ for wgpu

            let state: Rc<RefCell<Option<GlesGpuState>>> = Rc::new(RefCell::new(None));
            let init_failed: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
            let renderer_cell = Rc::new(RefCell::new(Some(user_renderer)));

            // Initialize wgpu when GL context is ready
            gl_area.connect_realize({
                let state = state.clone();
                let init_failed = init_failed.clone();
                let renderer_cell = renderer_cell.clone();
                move |area| {
                    area.make_current();
                    if let Some(err) = area.error() {
                        tracing::error!("[GpuSurface] GL error on realize: {}", err);
                        *init_failed.borrow_mut() = true;
                        return;
                    }

                    let width = area.width().max(1) as u32;
                    let height = area.height().max(1) as u32;

                    if let Some(renderer) = renderer_cell.borrow_mut().take() {
                        match init_wgpu_gles(area, width, height, renderer) {
                            Ok(gpu_state) => {
                                *state.borrow_mut() = Some(gpu_state);
                                tracing::info!("[GpuSurface] wgpu GLES initialized successfully");
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[GpuSurface] Initialization failed: {}. GPU rendering disabled.",
                                    e
                                );
                                *init_failed.borrow_mut() = true;
                            }
                        }
                    }
                }
            });

            // Render callback - wgpu renders to GTK's framebuffer
            gl_area.connect_render({
                let state = state.clone();
                let init_failed = init_failed.clone();
                move |area, _gl_context| {
                    // If initialization failed, draw a placeholder (dark gray)
                    if *init_failed.borrow() {
                        area.make_current();
                        // Try to clear the buffer using epoxy/glow
                        if init_epoxy().is_ok() {
                            unsafe {
                                let gl = glow::Context::from_loader_function(|s| {
                                    epoxy::get_proc_addr(s).cast::<std::ffi::c_void>()
                                });
                                gl.clear_color(0.18, 0.18, 0.18, 1.0);
                                gl.clear(glow::COLOR_BUFFER_BIT);
                            }
                        }
                        return glib::Propagation::Stop;
                    }

                    area.attach_buffers(); // Ensure GTK's FBO is bound

                    if let Some(ref mut gpu_state) = *state.borrow_mut() {
                        // Update framebuffer binding if needed
                        gpu_state.update_framebuffer();

                        // Render frame
                        gpu_state.render_frame();
                    }
                    glib::Propagation::Stop
                }
            });

            // Handle resize
            gl_area.connect_resize({
                let state = state.clone();
                move |_area, width, height| {
                    if let Some(ref mut gpu_state) = *state.borrow_mut() {
                        gpu_state.resize(width as u32, height as u32);
                    }
                }
            });

            // Animation loop - requests redraw on each frame tick (60fps)
            gl_area.add_tick_callback({
                let area_weak = gl_area.downgrade();
                move |_, _| {
                    if let Some(area) = area_weak.upgrade() {
                        area.queue_render();
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                }
            });

            // Cleanup on unrealize
            gl_area.connect_unrealize({
                let state = state.clone();
                move |_| {
                    if state.borrow_mut().take().is_some() {
                        tracing::debug!("[GpuSurface] Cleaned up GPU state");
                    }
                }
            });

            gl_area.upcast()
        }
    }
}

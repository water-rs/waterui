//! `GpuSurface` native renderer for GTK (Linux-only).
//!
//! Implementation strategy:
//! - Use `GtkGLArea` to obtain a per-widget OpenGL context + framebuffer.
//! - Create a wgpu device/queue from the *current* GL context via wgpu-hal "external" adapter.
//! - Each frame, wrap the `GtkGLArea` framebuffer as a wgpu texture and let `GpuSurface` render into it.
//!
//! This avoids any window-system-specific surface creation (Wayland/X11) and keeps GL details
//! fully internal to the GTK backend.

use std::cell::RefCell;
use std::ffi::{CString, c_char, c_void};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gdk4::prelude::*;
use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_graphics::gpu_surface::{
    GestureState, GpuContext, GpuFrame, GpuSurface, PointerState, preferred_msaa_samples,
};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

#[cfg(not(target_os = "linux"))]
compile_error!(
    "GTK GpuSurface implementation is Linux-only. The waterui-gtk crate should not be built on non-Linux targets."
);

mod gdk_gl_ffi {
    use super::*;

    // gtk4/gdk4 link us to libgdk-4 already; declare the symbol directly.
    //
    // Safety: the returned pointer is owned by the GL implementation; we must not free it.
    extern "C" {
        pub fn gdk_gl_context_get_proc_address(
            context: *mut gdk4::ffi::GdkGLContext,
            proc_name: *const c_char,
        ) -> *const c_void;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelSize {
    width: u32,
    height: u32,
}

impl PixelSize {
    fn from_widget(area: &gtk4::GLArea) -> Self {
        let scale = area.scale_factor().max(1) as u32;
        let w = area.allocated_width().max(1) as u32;
        let h = area.allocated_height().max(1) as u32;
        Self {
            width: w.saturating_mul(scale),
            height: h.saturating_mul(scale),
        }
    }
}

#[derive(Debug)]
struct GpuState {
    gpu_surface: Option<GpuSurface>,

    wgpu_instance: Option<wgpu::Instance>,
    wgpu_adapter: Option<wgpu::Adapter>,
    wgpu_device: Option<wgpu::Device>,
    wgpu_queue: Option<wgpu::Queue>,

    surface_format: Option<wgpu::TextureFormat>,
    msaa_samples: u32,

    last_size: Option<PixelSize>,
    setup_in_flight: bool,
    setup_done: bool,

    pointer: PointerState,
    gesture: GestureState,
    pan_active: bool,
    last_pinch_update: Option<Instant>,

    // Used only for querying framebuffer properties.
    glow: Option<Rc<glow::Context>>,
}

impl GpuState {
    fn new(gpu_surface: GpuSurface) -> Self {
        Self {
            gpu_surface: Some(gpu_surface),
            wgpu_instance: None,
            wgpu_adapter: None,
            wgpu_device: None,
            wgpu_queue: None,
            surface_format: None,
            msaa_samples: 1,
            last_size: None,
            setup_in_flight: false,
            setup_done: false,
            pointer: PointerState::default(),
            gesture: GestureState::default(),
            pan_active: false,
            last_pinch_update: None,
            glow: None,
        }
    }
}

fn map_gl_internal_format_to_wgpu(internal: i32) -> wgpu::TextureFormat {
    match internal as u32 {
        glow::RGBA8 => wgpu::TextureFormat::Rgba8Unorm,
        glow::SRGB8_ALPHA8 => wgpu::TextureFormat::Rgba8UnormSrgb,
        glow::RGB10_A2 => wgpu::TextureFormat::Rgb10a2Unorm,
        glow::RGBA16F => wgpu::TextureFormat::Rgba16Float,
        other => {
            panic!("GpuSurface(GL): unsupported default framebuffer internal format 0x{other:x}")
        }
    }
}

fn query_framebuffer_format(gl: &glow::Context) -> wgpu::TextureFormat {
    // GLArea binds its framebuffer before invoking the "render" signal.
    let obj_type = unsafe {
        gl.get_framebuffer_attachment_parameter_i32(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
        )
    };

    match obj_type as u32 {
        glow::RENDERBUFFER => {
            let name = unsafe {
                gl.get_framebuffer_attachment_parameter_i32(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
                )
            };
            let rb =
                glow::NativeRenderbuffer(NonZeroU32::new(name as u32).unwrap_or_else(|| {
                    panic!("GpuSurface(GL): expected non-zero renderbuffer name")
                }));
            unsafe {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            }
            let internal = unsafe {
                gl.get_renderbuffer_parameter_i32(
                    glow::RENDERBUFFER,
                    glow::RENDERBUFFER_INTERNAL_FORMAT,
                )
            };
            unsafe {
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            }
            map_gl_internal_format_to_wgpu(internal)
        }
        glow::TEXTURE => {
            let name = unsafe {
                gl.get_framebuffer_attachment_parameter_i32(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
                )
            };
            let target = unsafe {
                gl.get_framebuffer_attachment_parameter_i32(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::FRAMEBUFFER_ATTACHMENT_TEXTURE_TARGET,
                )
            };

            let tex = glow::NativeTexture(
                NonZeroU32::new(name as u32)
                    .unwrap_or_else(|| panic!("GpuSurface(GL): expected non-zero texture name")),
            );

            unsafe {
                gl.bind_texture(target as u32, Some(tex));
            }
            let internal = unsafe {
                gl.get_tex_level_parameter_i32(target as u32, 0, glow::TEXTURE_INTERNAL_FORMAT)
            };
            unsafe {
                gl.bind_texture(target as u32, None);
            }
            map_gl_internal_format_to_wgpu(internal)
        }
        other => panic!("GpuSurface(GL): unexpected framebuffer attachment type {other}"),
    }
}

fn current_framebuffer(gl: &glow::Context) -> glow::NativeFramebuffer {
    let id = unsafe { gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) };
    glow::NativeFramebuffer(NonZeroU32::new(id as u32).unwrap_or_else(|| {
        panic!(
            "GpuSurface(GL): expected non-zero FRAMEBUFFER_BINDING (GtkGLArea should use an FBO)"
        )
    }))
}

fn make_gl_loader(gl_ctx: &gdk4::GLContext) -> impl FnMut(&str) -> *const c_void {
    use glib::translate::ToGlibPtr;

    let ctx_ptr = gl_ctx.as_ref().to_glib_none().0;
    move |name: &str| {
        let c = CString::new(name)
            .unwrap_or_else(|_| panic!("GpuSurface(GL): GL proc name contains NUL byte: {name:?}"));
        // SAFETY: `ctx_ptr` is a valid `GdkGLContext*` as long as `gl_ctx` is alive.
        unsafe { gdk_gl_ffi::gdk_gl_context_get_proc_address(ctx_ptr, c.as_ptr()) }
    }
}

fn init_wgpu_if_needed(area: &gtk4::GLArea, state: &mut GpuState, gl_ctx: &gdk4::GLContext) {
    if state.wgpu_device.is_some() {
        return;
    }

    let mut loader = make_gl_loader(gl_ctx);
    let glow = Rc::new(unsafe { glow::Context::from_loader_function(|s| loader(s)) });
    let format = query_framebuffer_format(&glow);

    let exposed = unsafe {
        wgpu::hal::gles::Adapter::new_external(|s| loader(s), wgpu::GlBackendOptions::default())
    }
    .unwrap_or_else(|| panic!("GpuSurface(GL): wgpu-hal failed to create external adapter"));

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::GL,
        ..Default::default()
    });

    // SAFETY: We rely on wgpu-core accepting the external hal adapter for the GL backend.
    // The GL context must be current whenever any derived objects are used/dropped.
    let adapter = unsafe { instance.create_adapter_from_hal::<wgpu::hal::api::Gles>(exposed) };

    let adapter_limits = adapter.limits();
    let required_limits = wgpu::Limits::default().using_resolution(adapter_limits);

    // Create the hal device/queue while the GtkGLArea context is current.
    let open = unsafe {
        adapter
            .as_hal::<wgpu::hal::api::Gles>()
            .expect("GpuSurface(GL): expected GL adapter")
            .open(
                wgpu::Features::empty(),
                &required_limits,
                &wgpu::MemoryHints::Performance,
            )
    }
    .unwrap_or_else(|e| panic!("GpuSurface(GL): failed to open hal device: {e:?}"));

    let (device, queue) = unsafe {
        adapter.create_device_from_hal::<wgpu::hal::api::Gles>(
            open,
            &wgpu::DeviceDescriptor {
                label: Some("WaterUI GTK(GLES) Device"),
                required_features: wgpu::Features::empty(),
                required_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::default(),
            },
        )
    }
    .unwrap_or_else(|e| panic!("GpuSurface(GL): failed to create wgpu device from hal: {e}"));

    device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
        tracing::error!("[wgpu] Uncaptured error: {error}");
    }));

    let msaa_samples = preferred_msaa_samples(&adapter, format, 4);

    state.wgpu_instance = Some(instance);
    state.wgpu_adapter = Some(adapter);
    state.wgpu_device = Some(device);
    state.wgpu_queue = Some(queue);
    state.surface_format = Some(format);
    state.msaa_samples = msaa_samples;
    state.glow = Some(glow);

    // Ensure we drive frames once wgpu is ready.
    area.queue_render();
}

fn schedule_setup(state: Rc<RefCell<GpuState>>) {
    gtk4::glib::MainContext::default().spawn_local(async move {
        let (mut gpu_surface, device, queue, adapter, format, msaa_samples) = {
            let mut st = state.borrow_mut();
            if st.setup_in_flight || st.setup_done {
                return;
            }
            let Some(device) = st.wgpu_device.as_ref() else {
                return;
            };
            let Some(queue) = st.wgpu_queue.as_ref() else {
                return;
            };
            let Some(adapter) = st.wgpu_adapter.as_ref() else {
                return;
            };
            let Some(format) = st.surface_format else {
                return;
            };
            let Some(gpu_surface) = st.gpu_surface.take() else {
                panic!("GpuSurface: missing GpuSurface during setup (internal state corrupted)");
            };

            st.setup_in_flight = true;

            (
                gpu_surface,
                device.clone(),
                queue.clone(),
                adapter.clone(),
                format,
                st.msaa_samples,
            )
        };

        let ctx = GpuContext {
            adapter: Some(&adapter),
            device: &device,
            queue: &queue,
            surface_format: format,
            msaa_samples,
            pipeline_cache: None,
        };

        gpu_surface.setup(&ctx).await;

        let mut st = state.borrow_mut();
        st.gpu_surface = Some(gpu_surface);
        st.setup_in_flight = false;
        st.setup_done = true;
    });
}

fn render_frame(area: &gtk4::GLArea, state: &Rc<RefCell<GpuState>>) {
    let (device, queue, adapter, format, msaa_samples, mut gpu_surface, pointer, gesture, glow) = {
        let mut st = state.borrow_mut();
        let Some(device) = st.wgpu_device.as_ref() else {
            return;
        };
        let Some(queue) = st.wgpu_queue.as_ref() else {
            return;
        };
        let Some(adapter) = st.wgpu_adapter.as_ref() else {
            return;
        };
        let Some(format) = st.surface_format else {
            return;
        };
        let Some(glow) = st.glow.as_ref() else { return };
        let Some(gpu_surface) = st.gpu_surface.take() else {
            // Setup may still be running.
            return;
        };

        (
            device.clone(),
            queue.clone(),
            adapter.clone(),
            format,
            msaa_samples,
            gpu_surface,
            st.pointer,
            st.gesture,
            Rc::clone(glow),
        )
    };

    let size = PixelSize::from_widget(area);
    gpu_surface.resize(size.width, size.height);

    // Confirm format hasn't changed (it shouldn't). If it does, crash early.
    let observed_format = query_framebuffer_format(&glow);
    if observed_format != format {
        panic!(
            "GpuSurface(GL): framebuffer format changed at runtime: {format:?} -> {observed_format:?}"
        );
    }

    let fb = current_framebuffer(&glow);

    let hal_texture = wgpu::hal::gles::Texture {
        inner: wgpu::hal::gles::TextureInner::ExternalNativeFramebuffer { inner: fb },
        drop_guard: None,
        mip_level_count: 1,
        array_layer_count: 1,
        format,
        format_desc: wgpu::hal::gles::TextureFormatDesc {
            internal: 0,
            external: 0,
            data_type: 0,
        },
        copy_size: wgpu::hal::CopyExtent {
            width: size.width,
            height: size.height,
            depth: 1,
        },
    };

    let texture = unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Gles>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("WaterUI GTK External FBO Texture"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        )
    };

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let frame = GpuFrame {
        device: &device,
        queue: &queue,
        texture: &texture,
        view,
        format,
        width: size.width,
        height: size.height,
        pointer,
        gesture,
    };

    // Let the WaterUI renderer submit work.
    gpu_surface.render(&frame);

    // Keep the surface alive for the next frame.
    let mut st = state.borrow_mut();
    // `double_tap` is a one-frame pulse.
    st.gesture.double_tap = false;
    // Decay pinch state when updates stop coming.
    if let Some(last) = st.last_pinch_update {
        if last.elapsed() > Duration::from_millis(140) {
            st.last_pinch_update = None;
            st.gesture.pinch_scale = 1.0;
            st.gesture.pinch_center = None;
            if !st.pan_active {
                st.gesture.active = false;
            }
        }
    }
    st.last_size = Some(size);
    st.gpu_surface = Some(gpu_surface);

    // Prevent GTK from drawing anything else for this GLArea.
    let _ = (adapter, msaa_samples);
}

fn install_input_controllers(area: &gtk4::GLArea, state: &Rc<RefCell<GpuState>>) {
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_enter({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_ctrl, x, y| {
            let mut st = state.borrow_mut();
            let scale = area.scale_factor().max(1) as f32;
            st.pointer.position = Some(waterui_core::layout::Point::new(
                x as f32 * scale,
                y as f32 * scale,
            ));
            area.queue_render();
        }
    });
    motion.connect_motion({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_ctrl, x, y| {
            let mut st = state.borrow_mut();
            let scale = area.scale_factor().max(1) as f32;
            st.pointer.position = Some(waterui_core::layout::Point::new(
                x as f32 * scale,
                y as f32 * scale,
            ));
            area.queue_render();
        }
    });
    motion.connect_leave({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_ctrl| {
            let mut st = state.borrow_mut();
            st.pointer.position = None;
            area.queue_render();
        }
    });
    area.add_controller(motion);

    let click = gtk4::GestureClick::new();
    click.set_button(0);
    click.connect_pressed({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_gesture, _n_press, x, y| {
            let mut st = state.borrow_mut();
            let scale = area.scale_factor().max(1) as f32;
            let p = waterui_core::layout::Point::new(x as f32 * scale, y as f32 * scale);
            st.pointer.hit = Some(p);
            area.queue_render();
        }
    });
    click.connect_released({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_gesture, n_press, _x, _y| {
            let mut st = state.borrow_mut();
            st.pointer.hit = None;
            if n_press >= 2 {
                st.gesture.double_tap = true;
                st.gesture.active = false;
                st.gesture.pinch_scale = 1.0;
                st.gesture.pinch_center = None;
                st.gesture.pan_offset = waterui_core::layout::Point::new(0.0, 0.0);
                st.pan_active = false;
                st.last_pinch_update = None;
            }
            area.queue_render();
        }
    });
    area.add_controller(click);

    let pan = gtk4::GestureDrag::new();
    pan.set_button(0);
    pan.connect_drag_begin({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_gesture, _x, _y| {
            let mut st = state.borrow_mut();
            st.pan_active = true;
            st.gesture.active = true;
            st.gesture.pan_offset = waterui_core::layout::Point::new(0.0, 0.0);
            area.queue_render();
        }
    });
    pan.connect_drag_update({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_gesture, offset_x, offset_y| {
            let mut st = state.borrow_mut();
            let scale = area.scale_factor().max(1) as f32;
            st.gesture.active = true;
            st.gesture.pan_offset =
                waterui_core::layout::Point::new(offset_x as f32 * scale, offset_y as f32 * scale);
            area.queue_render();
        }
    });
    pan.connect_drag_end({
        let area = area.clone();
        let state = Rc::clone(state);
        move |_gesture, _offset_x, _offset_y| {
            let mut st = state.borrow_mut();
            st.pan_active = false;
            st.gesture.pan_offset = waterui_core::layout::Point::new(0.0, 0.0);
            if st.last_pinch_update.is_none() {
                st.gesture.active = false;
            }
            area.queue_render();
        }
    });
    area.add_controller(pan);

    let zoom = gtk4::GestureZoom::new();
    zoom.connect_scale_changed({
        let area = area.clone();
        let state = Rc::clone(state);
        move |gesture, scale| {
            let mut st = state.borrow_mut();
            let scale_factor = area.scale_factor().max(1) as f32;
            st.gesture.active = true;
            st.gesture.pinch_scale = scale as f32;
            st.gesture.pinch_center = gesture.bounding_box().map(|bbox| {
                let center_x = bbox.x() + bbox.width() * 0.5;
                let center_y = bbox.y() + bbox.height() * 0.5;
                waterui_core::layout::Point::new(center_x as f32 * scale_factor, center_y as f32 * scale_factor)
            });
            st.last_pinch_update = Some(Instant::now());
            area.queue_render();
        }
    });
    area.add_controller(zoom);
}

fn render_gpu_surface(mut gpu_surface: GpuSurface) -> gtk4::Widget {
    let area = gtk4::GLArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_can_target(true);
    area.set_auto_render(false);
    area.set_has_depth_buffer(false);
    area.set_has_stencil_buffer(false);

    // Request both GL and GLES; GDK picks the best available.
    area.set_allowed_apis(gdk4::GLAPI::GL | gdk4::GLAPI::GLES);

    let state = Rc::new(RefCell::new(GpuState::new(gpu_surface)));
    install_input_controllers(&area, &state);

    area.connect_realize({
        let state = Rc::clone(&state);
        move |area| {
            area.make_current();
            if let Some(err) = area.error() {
                panic!("GpuSurface(GL): GtkGLArea realize failed: {err}");
            }

            // Drive frames via the frame clock.
            let widget: gtk4::Widget = area.clone().upcast();
            widget.add_tick_callback({
                let area = area.clone();
                let state = Rc::clone(&state);
                move |_widget, _clock| {
                    // Kick setup once the device exists.
                    schedule_setup(Rc::clone(&state));
                    area.queue_render();
                    gtk4::glib::ControlFlow::Continue
                }
            });
        }
    });

    area.connect_unrealize({
        let state = Rc::clone(&state);
        move |area| {
            // Drop wgpu objects while the GtkGLArea context is still current.
            area.make_current();
            if let Some(err) = area.error() {
                panic!("GpuSurface(GL): GtkGLArea unrealize error: {err}");
            }

            let mut st = state.borrow_mut();
            st.gpu_surface = None;
            st.wgpu_queue = None;
            st.wgpu_device = None;
            st.wgpu_adapter = None;
            st.wgpu_instance = None;
            st.glow = None;
            st.setup_in_flight = false;
            st.setup_done = false;
            st.last_size = None;
        }
    });

    area.connect_render({
        let state = Rc::clone(&state);
        move |area, gl_ctx| {
            area.make_current();
            if let Some(err) = area.error() {
                panic!("GpuSurface(GL): GtkGLArea render error: {err}");
            }

            {
                let mut st = state.borrow_mut();
                init_wgpu_if_needed(area, &mut st, gl_ctx);
            }

            render_frame(area, &state);

            gtk4::glib::Propagation::Stop
        }
    });

    area.upcast()
}

impl GtkComponent for Native<GpuSurface> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        render_gpu_surface(self.into_inner())
    }
}

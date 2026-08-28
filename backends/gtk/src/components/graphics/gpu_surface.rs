//! `GpuSurface` native renderer for GTK (Linux-only).
//!
//! Implementation strategy:
//! - Use `GtkGLArea` to obtain a per-widget OpenGL context + framebuffer.
//! - Create a wgpu device/queue from the *current* GL context via wgpu-hal "external" adapter.
//! - Each frame, wrap the `GtkGLArea` framebuffer as a wgpu texture and let `GpuSurface` render into it.
//!
//! This avoids any window-system-specific surface creation (Wayland/X11) and keeps GL details
//! fully internal to the GTK backend.
//!
//! Each widget owns its own wgpu instance/adapter/device on purpose: the
//! device is created *from* the widget's GL context via wgpu-hal's external
//! adapter, so its validity is tied to that specific `GdkGLContext` being
//! current. Sharing one external-GL device across `GLArea`s would require
//! guaranteeing share-group compatibility and context currency across widgets,
//! which GDK does not promise; `SharedGpuContext` (waterui-graphics) is for
//! hosts that own a real windowing surface, not for externally-adopted GL.

use std::cell::RefCell;
use std::ffi::{CString, c_char, c_void};
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use gdk4::prelude::*;
use glow::HasContext;
use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::layout::ProposalSize;
use waterui_core::{Environment, Native};
use waterui_graphics::gpu_surface::WgslModuleCache;
use waterui_graphics::gpu_surface::{
    GestureState, GpuContext, GpuFrame, GpuSurface, PointerState, RedrawHandle,
    preferred_msaa_samples,
};
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::{SceneEngine, SharedSceneRenderer};

use crate::browser_input::{SurfaceInputSink, install as install_surface_input};
use crate::component::GtkComponent;
use crate::renderer::{CSS_CLASS_DYNAMIC_RANGE_HDR, CSS_CLASS_DYNAMIC_RANGE_SDR, GtkRenderer};

#[cfg(not(target_os = "linux"))]
compile_error!(
    "GTK GpuSurface implementation is Linux-only. The waterui-gtk crate should not be built on non-Linux targets."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelSize {
    width: u32,
    height: u32,
}

impl PixelSize {
    #[allow(
        clippy::cast_sign_loss,
        reason = "OpenGL enums and object names are non-negative"
    )]
    fn from_widget(area: &gtk4::GLArea) -> Self {
        let scale = area.scale_factor().max(1) as u32;
        let w = area.width().max(1) as u32;
        let h = area.height().max(1) as u32;
        Self {
            width: w.saturating_mul(scale),
            height: h.saturating_mul(scale),
        }
    }
}

#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent GL surface capability reported by the driver"
)]
struct GpuState {
    gpu_surface: Option<GpuSurface>,
    msaa_max_samples: NonZeroU32,
    wgpu_instance: Option<wgpu::Instance>,
    wgpu_adapter: Option<wgpu::Adapter>,
    wgpu_device: Option<wgpu::Device>,
    wgpu_queue: Option<wgpu::Queue>,
    device_init_in_progress: bool,
    /// Module cache and scene renderer for the device above.
    ///
    /// Both hold objects compiled against that device — `wgpu::ShaderModule`s
    /// and vello pipelines — so they are created with it and dropped with it on
    /// unrealize. Keeping them here rather than inside the setup task is what
    /// makes them shared at all: a cache created per setup call is thrown away
    /// the moment setup returns and caches nothing.
    device_shared: Option<DeviceSharedResources>,

    surface_format: Option<wgpu::TextureFormat>,
    msaa_samples: u32,

    last_size: Option<PixelSize>,
    setup_done: bool,
    setup_in_progress: bool,
    /// Bumped every unrealize. In-flight async work (device request, renderer
    /// setup) captures the generation it started under and discards its result
    /// when the GL context it was created against is gone.
    context_generation: u64,
    /// The external framebuffer texture wrapped for wgpu, keyed by the pixel
    /// size and GL attachment it was created for. Re-wrapping (and the
    /// format-introspection assert) only happens when either changes, not per
    /// frame.
    cached_target: Option<CachedExternalTarget>,

    pointer: PointerState,
    gesture: GestureState,
    pan_active: bool,
    last_pinch_update: Option<Instant>,
    start_time: Instant,
    last_frame_time: Instant,
    redraw_handle: RedrawHandle,
    env: Environment,

    // Used only for querying framebuffer properties.
    glow: Option<Rc<glow::Context>>,
}

/// The resources every renderer on one GL-adopted device shares.
///
/// The GTK backend gives each `GLArea` its own external-GL device (see the
/// module header), so "per device" is "per widget" here — but a single surface
/// still assembles the same WGSL and draws vector scenes across many setups and
/// frames, and both of these exist so that work happens once per device instead
/// of once per use.
#[derive(Debug)]
struct DeviceSharedResources {
    shader_cache: Arc<WgslModuleCache>,
    scene_renderer: Arc<SharedSceneRenderer>,
}

impl DeviceSharedResources {
    /// Builds the pair for `adapter`, choosing the scene engine from what the
    /// adapter reports. External-GL adapters frequently lack indirect
    /// execution, so this is not a formality: asking such a device to run the
    /// classic pipeline aborts inside wgpu rather than degrading.
    fn new(adapter: &wgpu::Adapter) -> Self {
        Self {
            shader_cache: Arc::new(WgslModuleCache::new()),
            scene_renderer: Arc::new(SharedSceneRenderer::new(SceneEngine::for_adapter(adapter))),
        }
    }
}

impl GpuState {
    fn new(gpu_surface: GpuSurface, env: Environment) -> Self {
        let msaa_max_samples = gpu_surface.msaa_sample_limit();
        Self {
            start_time: Instant::now(),
            last_frame_time: Instant::now()
                .checked_sub(Duration::from_secs_f32(1.0 / 60.0))
                .unwrap(),
            gpu_surface: Some(gpu_surface),
            msaa_max_samples,
            wgpu_instance: None,
            wgpu_adapter: None,
            wgpu_device: None,
            wgpu_queue: None,
            device_init_in_progress: false,
            device_shared: None,
            surface_format: None,
            msaa_samples: 1,
            last_size: None,
            setup_done: false,
            setup_in_progress: false,
            context_generation: 0,
            cached_target: None,
            pointer: PointerState::default(),
            gesture: GestureState::default(),
            pan_active: false,
            last_pinch_update: None,
            redraw_handle: RedrawHandle::new(),
            env,
            glow: None,
        }
    }
}

#[derive(Debug)]
struct CachedExternalTarget {
    size: PixelSize,
    attachment: glow::NativeFramebuffer,
    texture: wgpu::Texture,
}

pin_project_lite::pin_project! {
    /// Makes the `GtkGLArea`'s GL context current before every poll of the
    /// wrapped future. wgpu's external-GL adapter requires the owning context
    /// to be current whenever a wgpu entry point runs, and an async device
    /// request or renderer setup resumes on the main loop with no context
    /// current at all - other GL widgets may even have made theirs current in
    /// between polls.
    struct WithAreaContextCurrent<F> {
        area: gtk4::GLArea,
        #[pin]
        future: F,
    }
}

impl<F: Future> Future for WithAreaContextCurrent<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.project();
        // An unrealized area has no context to make current; the wrapped task
        // observes the bumped context generation and bails on its own.
        if this.area.context().is_some() {
            this.area.make_current();
        }
        this.future.poll(cx)
    }
}

/// Installs a thread-safe waker on the surface's `RedrawHandle` so
/// `request_redraw()` from outside `render()` (worker threads, CEF/WPE paint
/// callbacks, timers) schedules a GTK frame instead of being discovered only
/// if some other event happens to redraw the widget.
fn install_redraw_waker(area: &gtk4::GLArea, state: &Rc<RefCell<GpuState>>) {
    // `WeakRef<GLArea>` is not `Send`, so it crosses threads inside a
    // `ThreadGuard` and is only dereferenced back on the main context's
    // thread, where `invoke` runs the closure.
    let area_guard = Arc::new(gtk4::glib::thread_guard::ThreadGuard::new(
        gtk4::prelude::ObjectExt::downgrade(area),
    ));
    let main_context = gtk4::glib::MainContext::default();
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let area_guard = Arc::clone(&area_guard);
        main_context.invoke(move || {
            if let Some(area) = area_guard.get_ref().upgrade() {
                area.queue_render();
            }
        });
    });
    state.borrow().redraw_handle.set_waker(Some(waker));
}

#[allow(
    clippy::cast_sign_loss,
    reason = "OpenGL enums and object names are non-negative"
)]
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

#[allow(
    clippy::cast_sign_loss,
    reason = "OpenGL enums and object names are non-negative"
)]
fn query_framebuffer_format(gl: &glow::Context) -> wgpu::TextureFormat {
    // GLArea binds its framebuffer before invoking the "render" signal.
    // SAFETY: the caller runs with the GtkGLArea's GL context current on this
    // thread (GTK makes it current before its GL signals), and this query only
    // reads driver state for the framebuffer GTK just bound.
    let obj_type = unsafe {
        gl.get_framebuffer_attachment_parameter_i32(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
        )
    };

    match obj_type as u32 {
        glow::RENDERBUFFER => {
            // SAFETY: same current-context invariant as above; reads the
            // attachment's object name from driver state.
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
            // SAFETY: same current-context invariant; `rb` is the live
            // renderbuffer the driver just reported as the color attachment.
            unsafe {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            }
            // SAFETY: same current-context invariant; queries the renderbuffer
            // bound directly above.
            let internal = unsafe {
                gl.get_renderbuffer_parameter_i32(
                    glow::RENDERBUFFER,
                    glow::RENDERBUFFER_INTERNAL_FORMAT,
                )
            };
            // SAFETY: same current-context invariant; restores the binding
            // touched above.
            unsafe {
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            }
            map_gl_internal_format_to_wgpu(internal)
        }
        glow::TEXTURE => {
            // SAFETY: same current-context invariant as the query above.
            let _name = unsafe {
                gl.get_framebuffer_attachment_parameter_i32(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
                )
            };
            // SAFETY: same current-context invariant as the query above.
            let encoding = unsafe {
                gl.get_framebuffer_attachment_parameter_i32(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING,
                )
            } as u32;
            // SAFETY: same current-context invariant as the query above.
            let component_type = unsafe {
                gl.get_framebuffer_attachment_parameter_i32(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE,
                )
            } as u32;

            match component_type {
                glow::FLOAT | glow::HALF_FLOAT | glow::HALF_FLOAT_OES => {
                    wgpu::TextureFormat::Rgba16Float
                }
                glow::UNSIGNED_NORMALIZED => {
                    if encoding == glow::SRGB {
                        wgpu::TextureFormat::Rgba8UnormSrgb
                    } else {
                        wgpu::TextureFormat::Rgba8Unorm
                    }
                }
                other => panic!(
                    "GpuSurface(GL): unsupported texture-backed framebuffer component type 0x{other:x}"
                ),
            }
        }
        other => panic!("GpuSurface(GL): unexpected framebuffer attachment type {other}"),
    }
}

#[allow(
    clippy::cast_sign_loss,
    reason = "OpenGL enums and object names are non-negative"
)]
fn current_framebuffer(gl: &glow::Context) -> glow::NativeFramebuffer {
    // SAFETY: the caller runs with the GtkGLArea's GL context current on this
    // thread; reading FRAMEBUFFER_BINDING only inspects driver state.
    let id = unsafe { gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) };
    glow::NativeFramebuffer(NonZeroU32::new(id as u32).unwrap_or_else(|| {
        panic!(
            "GpuSurface(GL): expected non-zero FRAMEBUFFER_BINDING (GtkGLArea should use an FBO)"
        )
    }))
}

const FRAMEBUFFER_ATTACHMENT_TEXTURE_TARGET_PNAME: u32 = 0x8CD2;

#[allow(
    clippy::cast_sign_loss,
    reason = "OpenGL enums and object names are non-negative"
)]
fn current_color_attachment(gl: &glow::Context) -> glow::NativeFramebuffer {
    // SAFETY: the caller runs with the GtkGLArea's GL context current on this
    // thread; this query only reads driver state for the bound framebuffer.
    let obj_type = unsafe {
        gl.get_framebuffer_attachment_parameter_i32(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
        )
    } as u32;

    match obj_type {
        glow::RENDERBUFFER => current_framebuffer(gl),
        glow::TEXTURE => {
            // SAFETY: same current-context invariant as the query above.
            let target = unsafe {
                gl.get_framebuffer_attachment_parameter_i32(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    FRAMEBUFFER_ATTACHMENT_TEXTURE_TARGET_PNAME,
                )
            } as u32;
            tracing::debug!(
                "[gtk-gpu] texture-backed default FBO target=0x{target:x}; using external framebuffer path"
            );
            current_framebuffer(gl)
        }
        other => panic!("GpuSurface(GL): unexpected color attachment type {other}"),
    }
}

fn texture_format_desc(format: wgpu::TextureFormat) -> wgpu::hal::gles::TextureFormatDesc {
    let (internal, external, data_type) = match format {
        wgpu::TextureFormat::Rgba8Unorm => (glow::RGBA8, glow::RGBA, glow::UNSIGNED_BYTE),
        wgpu::TextureFormat::Rgba8UnormSrgb => {
            (glow::SRGB8_ALPHA8, glow::RGBA, glow::UNSIGNED_BYTE)
        }
        wgpu::TextureFormat::Rgba16Float => (glow::RGBA16F, glow::RGBA, glow::HALF_FLOAT),
        wgpu::TextureFormat::Rgb10a2Unorm => (
            glow::RGB10_A2,
            glow::RGBA,
            glow::UNSIGNED_INT_2_10_10_10_REV,
        ),
        other => panic!("GpuSurface(GL): unsupported external framebuffer format {other:?}"),
    };
    wgpu::hal::gles::TextureFormatDesc {
        internal,
        external,
        data_type,
    }
}

type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type GlxGetProcAddress = unsafe extern "C" fn(*const u8) -> *const c_void;

struct GlProcResolver {
    libs: Vec<libloading::Library>,
    egl_get_proc: Option<EglGetProcAddress>,
    glx_get_proc: Option<GlxGetProcAddress>,
}

impl GlProcResolver {
    fn new(uses_es: bool) -> Self {
        let candidates: &[&str] = if uses_es {
            &["libGLESv2.so.2", "libEGL.so.1"]
        } else {
            &["libGL.so.1", "libOpenGL.so.0", "libEGL.so.1"]
        };
        let mut libs = Vec::new();
        let mut egl_get_proc = None;
        let mut glx_get_proc = None;

        for path in candidates {
            // SAFETY: these are the platform's own GL runtime libraries, which
            // GDK has already loaded to create the GL context this resolver
            // serves, so dlopening them again only bumps a refcount and runs
            // no untrusted initializer.
            let Ok(lib) = (unsafe { libloading::Library::new(*path) }) else {
                continue;
            };
            if egl_get_proc.is_none() {
                // SAFETY: symbol lookup only; the signature matches the EGL
                // specification for eglGetProcAddress, and the pointer is used
                // while the library stays loaded (GDK pins it for the process
                // lifetime).
                let symbol = unsafe { lib.get::<EglGetProcAddress>(b"eglGetProcAddress\0") };
                if let Ok(symbol) = symbol {
                    egl_get_proc = Some(*symbol);
                }
            }
            if glx_get_proc.is_none() {
                // SAFETY: symbol lookup only; the signature matches the GLX
                // specification for glXGetProcAddressARB, and the pointer is
                // used while the library stays loaded (GDK pins it for the
                // process lifetime).
                let symbol = unsafe { lib.get::<GlxGetProcAddress>(b"glXGetProcAddressARB\0") };
                if let Ok(symbol) = symbol {
                    glx_get_proc = Some(*symbol);
                }
            }
            libs.push(lib);
        }

        Self {
            libs,
            egl_get_proc: if uses_es { egl_get_proc } else { None },
            glx_get_proc: if uses_es { None } else { glx_get_proc },
        }
    }

    fn load(&self, name: &str) -> *const c_void {
        let Ok(cname) = CString::new(name) else {
            return std::ptr::null();
        };
        let bytes = cname.as_bytes_with_nul();

        for lib in &self.libs {
            // SAFETY: this is a symbol lookup by NUL-terminated name.
            if let Ok(symbol) = unsafe { lib.get::<*const c_void>(bytes) } {
                let ptr = *symbol;
                if !ptr.is_null() {
                    return ptr;
                }
            }
        }

        if let Some(get_proc) = self.egl_get_proc {
            // SAFETY: function pointer comes from the loaded EGL library.
            let ptr = unsafe { get_proc(cname.as_ptr()) };
            if !ptr.is_null() {
                return ptr;
            }
        }

        if let Some(get_proc) = self.glx_get_proc {
            // SAFETY: function pointer comes from the loaded GLX library.
            let ptr = unsafe { get_proc(cname.as_ptr().cast()) };
            if !ptr.is_null() {
                return ptr;
            }
        }

        std::ptr::null()
    }
}

fn make_gl_loader(gl_ctx: &gdk4::GLContext) -> impl FnMut(&str) -> *const c_void {
    let uses_es = gl_ctx.uses_es();
    let resolver = GlProcResolver::new(uses_es);
    move |name: &str| {
        let ptr = resolver.load(name);
        if ptr.is_null() {
            tracing::debug!("[gtk-gpu] unresolved GL symbol: {name}");
        }
        ptr
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one cohesive widget-construction pass; splitting it would scatter GTK setup order"
)]
fn init_wgpu_if_needed(
    area: &gtk4::GLArea,
    state: &Rc<RefCell<GpuState>>,
    gl_ctx: &gdk4::GLContext,
) {
    {
        let st = state.borrow();
        if st.wgpu_device.is_some() {
            tracing::debug!("[gtk-gpu] init_wgpu_if_needed: device already initialized");
            return;
        }
        if st.device_init_in_progress {
            tracing::debug!("[gtk-gpu] init_wgpu_if_needed: device request already pending");
            return;
        }
    }

    let (major, minor) = gl_ctx.version();
    tracing::debug!(
        uses_es = gl_ctx.uses_es(),
        major,
        minor,
        "creating GTK GpuSurface wgpu device"
    );

    let mut loader = make_gl_loader(gl_ctx);
    // SAFETY: the GLArea's GL context is current (this runs inside the
    // "render" signal after `make_current`), and the loader resolves symbols
    // from the platform GL runtime libraries, returning null for unknown
    // names; glow copies the function pointers here, and the libraries stay
    // loaded for the process lifetime because GDK's own GL context holds them.
    let glow = Rc::new(unsafe { glow::Context::from_loader_function(|s| loader(s)) });
    let format = query_framebuffer_format(&glow);
    let (prefers_hdr_explicit, msaa_max_samples) = {
        let st = state.borrow();
        (
            st.gpu_surface
                .as_ref()
                .and_then(GpuSurface::resolved_hdr_preference),
            st.msaa_max_samples,
        )
    };
    let prefers_hdr = prefers_hdr_explicit.or_else(|| inherited_hdr_preference(area));
    let format_is_hdr = matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    );
    if let Some(requested_hdr) = prefers_hdr.filter(|requested| *requested != format_is_hdr) {
        tracing::debug!(
            requested_hdr,
            ?format,
            "GTK compositor selected a framebuffer that differs from the requested dynamic range"
        );
    }
    tracing::debug!(
        ?format,
        format_is_hdr,
        "selected GTK GpuSurface framebuffer"
    );

    // SAFETY: the GLArea's GL context is current on this thread, which is
    // what `new_external` requires while it probes the context; the loader
    // has the same validity guarantees as for the glow context above.
    let exposed = unsafe {
        wgpu::hal::gles::Adapter::new_external(|s| loader(s), wgpu::GlBackendOptions::default())
    }
    .unwrap_or_else(|| panic!("GpuSurface(GL): wgpu-hal failed to create external adapter"));

    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = wgpu::Backends::GL;
    let instance = wgpu::Instance::new(instance_descriptor);
    // SAFETY: `exposed` was created by wgpu-hal's GL backend just above and is
    // handed to an instance configured for exactly that backend, with the same
    // GL context still current.
    let adapter = unsafe { instance.create_adapter_from_hal::<wgpu::hal::api::Gles>(exposed) };

    // Request what the adapter actually supports: capping a native GL driver
    // to WebGL2 downlevel limits starved desktop renderers of texture size,
    // bind groups, and uniform space they really have.
    let required_limits = adapter.limits();
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("WaterUI GTK(GLES) Device"),
        required_features: wgpu::Features::empty(),
        required_limits,
        memory_hints: wgpu::MemoryHints::Performance,
        experimental_features: wgpu::ExperimentalFeatures::default(),
        trace: wgpu::Trace::default(),
    };

    let msaa_samples = preferred_msaa_samples(&adapter, format, msaa_max_samples);
    {
        let mut st = state.borrow_mut();
        st.wgpu_instance = Some(instance);
        st.device_shared = Some(DeviceSharedResources::new(&adapter));
        st.wgpu_adapter = Some(adapter.clone());
        st.surface_format = Some(format);
        st.msaa_samples = msaa_samples;
        st.glow = Some(glow);
        st.device_init_in_progress = true;
    }

    let state_clone = Rc::clone(state);
    let area_clone = area.clone();
    let descriptor = descriptor.clone();
    let adapter_for_task = adapter;
    let generation = state.borrow().context_generation;
    gtk4::glib::MainContext::default().spawn_local(WithAreaContextCurrent {
        area: area.clone(),
        future: async move {
            let result = adapter_for_task.request_device(&descriptor).await;
            {
                let mut st = state_clone.borrow_mut();
                if st.context_generation != generation {
                    // The GL context this device was created against was torn
                    // down while the request was in flight; the device is dead.
                    tracing::debug!(
                        "[gtk-gpu] init_wgpu_if_needed: discarding device for unrealized context"
                    );
                    return;
                }
                st.device_init_in_progress = false;
                match result {
                    Ok((device, queue)) => {
                        device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
                            tracing::error!("[wgpu] Uncaptured error: {error}");
                        }));
                        st.wgpu_device = Some(device);
                        st.wgpu_queue = Some(queue);
                        let format = st.surface_format.unwrap_or(wgpu::TextureFormat::Rgba8Unorm);
                        let msaa = st.msaa_samples;
                        tracing::debug!(
                            "[gtk-gpu] init_wgpu_if_needed: device ready format={format:?} msaa={msaa}"
                        );
                    }
                    Err(e) => panic!("GpuSurface(GL): failed to request device: {e}"),
                }
            }
            area_clone.queue_render();
        },
    });
}

fn inherited_hdr_preference(area: &gtk4::GLArea) -> Option<bool> {
    let mut current = area.clone().upcast::<Widget>().parent();
    while let Some(widget) = current {
        if widget.has_css_class(CSS_CLASS_DYNAMIC_RANGE_HDR) {
            return Some(true);
        }
        if widget.has_css_class(CSS_CLASS_DYNAMIC_RANGE_SDR) {
            return Some(false);
        }
        current = widget.parent();
    }
    None
}

/// Everything one renderer setup pass needs, taken out of the state together so
/// the async task owns a consistent snapshot of a single GL context generation.
struct SetupInputs {
    gpu_surface: GpuSurface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::Adapter,
    format: wgpu::TextureFormat,
    msaa_max_samples: NonZeroU32,
    redraw_handle: RedrawHandle,
    env: Environment,
    shader_cache: Arc<WgslModuleCache>,
    scene_renderer: Arc<SharedSceneRenderer>,
}

impl GpuState {
    /// Claims the surface for a setup pass, or reports why one cannot start.
    ///
    /// Returns `None` while the device is still being requested, while another
    /// pass is running, or while a pass from a torn-down context still owns the
    /// surface. On `Some` the surface has been taken out of the state and
    /// `setup_in_progress` is set, so the caller must run the pass.
    fn begin_setup(&mut self) -> Option<SetupInputs> {
        if self.setup_in_progress {
            return None;
        }
        let Some(device) = self.wgpu_device.clone() else {
            tracing::debug!("[gtk-gpu] setup_if_needed: missing device");
            return None;
        };
        let Some(queue) = self.wgpu_queue.clone() else {
            tracing::debug!("[gtk-gpu] setup_if_needed: missing queue");
            return None;
        };
        let Some(adapter) = self.wgpu_adapter.clone() else {
            tracing::debug!("[gtk-gpu] setup_if_needed: missing adapter");
            return None;
        };
        let Some(format) = self.surface_format else {
            tracing::debug!("[gtk-gpu] setup_if_needed: missing surface format");
            return None;
        };
        let Some(shared) = self.device_shared.as_ref() else {
            tracing::debug!("[gtk-gpu] setup_if_needed: missing device-shared resources");
            return None;
        };
        let shader_cache = Arc::clone(&shared.shader_cache);
        let scene_renderer = Arc::clone(&shared.scene_renderer);
        let Some(gpu_surface) = self.gpu_surface.take() else {
            // A setup task from a torn-down context still owns the surface; it
            // hands it back (with setup_done unset) when it finishes, and the
            // next frame restarts setup on this context.
            tracing::debug!("[gtk-gpu] setup_if_needed: surface still owned by in-flight setup");
            return None;
        };
        self.setup_in_progress = true;
        Some(SetupInputs {
            gpu_surface,
            device,
            queue,
            adapter,
            format,
            msaa_max_samples: self.msaa_max_samples,
            redraw_handle: self.redraw_handle.clone(),
            env: self.env.clone(),
            shader_cache,
            scene_renderer,
        })
    }
}

fn setup_if_needed(area: &gtk4::GLArea, state: &Rc<RefCell<GpuState>>) -> bool {
    if state.borrow().setup_done {
        return true;
    }
    // Nothing can run between this borrow and the next on the GTK main loop, so
    // the state cannot change underneath the two checks.
    let Some(inputs) = state.borrow_mut().begin_setup() else {
        return false;
    };

    tracing::debug!("[gtk-gpu] setup_if_needed: begin setup");
    spawn_renderer_setup(area, state, inputs);
    false
}

/// Runs `GpuSurface::setup` on the main context and hands the surface back.
fn spawn_renderer_setup(area: &gtk4::GLArea, state: &Rc<RefCell<GpuState>>, inputs: SetupInputs) {
    let SetupInputs {
        mut gpu_surface,
        device,
        queue,
        adapter,
        format,
        msaa_max_samples,
        redraw_handle,
        mut env,
        shader_cache,
        scene_renderer,
    } = inputs;
    let state_clone = Rc::clone(state);
    let area_clone = area.clone();
    let generation = state.borrow().context_generation;
    gtk4::glib::MainContext::default().spawn_local(WithAreaContextCurrent {
        area: area.clone(),
        future: async move {
            let ctx = GpuContext::new(
                &adapter,
                &device,
                &queue,
                format,
                &shader_cache,
                &scene_renderer,
                msaa_max_samples,
                redraw_handle,
            );
            gpu_surface.setup(&ctx, &mut env).await;
            {
                let mut st = state_clone.borrow_mut();
                assert!(
                    st.gpu_surface.is_none(),
                    "GpuSurface(GL): setup completed but state still had a live surface"
                );
                st.gpu_surface = Some(gpu_surface);
                st.env = env;
                st.setup_in_progress = false;
                if st.context_generation == generation {
                    st.setup_done = true;
                    tracing::debug!("[gtk-gpu] setup complete");
                } else {
                    // The context died mid-setup: the renderer's GPU resources
                    // are gone with it. Leave setup_done unset so the next
                    // realized context runs setup again on its own device.
                    tracing::debug!(
                        "[gtk-gpu] setup finished for a torn-down context; will re-run"
                    );
                }
            }
            area_clone.queue_render();
        },
    });
}

#[allow(
    clippy::too_many_lines,
    reason = "one cohesive widget-construction pass; splitting it would scatter GTK setup order"
)]
fn render_frame(area: &gtk4::GLArea, state: &Rc<RefCell<GpuState>>) -> bool {
    let (
        device,
        queue,
        format,
        msaa_samples,
        mut gpu_surface,
        pointer,
        gesture,
        elapsed,
        delta,
        glow,
        redraw_handle,
    ) = {
        let mut st = state.borrow_mut();
        let Some(device) = st.wgpu_device.clone() else {
            tracing::debug!("[gtk-gpu] render_frame: missing device");
            return false;
        };
        let Some(queue) = st.wgpu_queue.clone() else {
            tracing::debug!("[gtk-gpu] render_frame: missing queue");
            return false;
        };
        let Some(format) = st.surface_format else {
            tracing::debug!("[gtk-gpu] render_frame: missing surface format");
            return false;
        };
        let Some(glow) = st.glow.clone() else {
            tracing::debug!("[gtk-gpu] render_frame: missing glow context");
            return false;
        };
        let Some(gpu_surface) = st.gpu_surface.take() else {
            // Setup may still be running.
            tracing::debug!("[gtk-gpu] render_frame: surface not available");
            return false;
        };

        let now = Instant::now();
        let elapsed = now.duration_since(st.start_time);
        let delta = now
            .duration_since(st.last_frame_time)
            .min(Duration::from_millis(100));
        st.last_frame_time = now;

        (
            device,
            queue,
            format,
            st.msaa_samples,
            gpu_surface,
            st.pointer,
            st.gesture,
            elapsed,
            delta,
            glow,
            st.redraw_handle.clone(),
        )
    };

    let size = PixelSize::from_widget(area);
    tracing::debug!("[gtk-gpu] render frame size={}x{}", size.width, size.height);

    let attachment = current_color_attachment(&glow);
    let cached = {
        let st = state.borrow();
        st.cached_target.as_ref().and_then(|cached| {
            (cached.size == size && cached.attachment == attachment).then(|| cached.texture.clone())
        })
    };
    let texture = cached.unwrap_or_else(|| {
        // The framebuffer changed (resize or FBO swap): re-run the format
        // introspection - the one point a driver could legally hand us a
        // different attachment - and re-wrap it for wgpu. Doing this per frame
        // was a chain of glGet round trips for an answer that cannot change
        // between resizes.
        let observed_format = query_framebuffer_format(&glow);
        assert!(
            observed_format == format,
            "GpuSurface(GL): framebuffer format changed at runtime: {format:?} -> {observed_format:?}"
        );

        let hal_texture = wgpu::hal::gles::Texture {
            inner: wgpu::hal::gles::TextureInner::ExternalNativeFramebuffer { inner: attachment },
            drop_guard: None,
            mip_level_count: 1,
            array_layer_count: 1,
            format,
            format_desc: texture_format_desc(format),
            copy_size: wgpu::hal::CopyExtent {
                width: size.width,
                height: size.height,
                depth: 1,
            },
        };

        // SAFETY: `hal_texture` wraps the framebuffer the driver reported for
        // this frame, whose introspected format was just asserted to match
        // `format` and whose extent is the widget's current pixel size, so the
        // descriptor describes the real attachment. The wrapped texture is
        // only used while that attachment exists: the cache is invalidated
        // whenever the size or attachment changes, and unrealize drops it with
        // the GL context still current.
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
        state.borrow_mut().cached_target = Some(CachedExternalTarget {
            size,
            attachment,
            texture: texture.clone(),
        });
        texture
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut frame = GpuFrame::new(
        &device,
        &queue,
        &texture,
        view,
        format,
        size.width,
        size.height,
        f64::from(area.scale_factor().max(1)),
        pointer,
        gesture,
        elapsed,
        delta,
    );

    // Let the WaterUI renderer submit work.
    gpu_surface.render(&mut frame);
    let needs_redraw = frame.was_redraw_requested() || redraw_handle.take_dirty();

    // Keep the surface alive for the next frame.
    let mut st = state.borrow_mut();
    // `double_tap` is a one-frame pulse.
    st.gesture.double_tap = false;
    // Decay pinch state when updates stop coming.
    if let Some(last) = st.last_pinch_update
        && last.elapsed() > Duration::from_millis(140)
    {
        st.last_pinch_update = None;
        st.gesture.pinch_scale = 1.0;
        st.gesture.pinch_center = None;
        if !st.pan_active {
            st.gesture.active = false;
        }
    }
    st.last_size = Some(size);
    st.gpu_surface = Some(gpu_surface);

    // Prevent GTK from drawing anything else for this GLArea.
    let _ = msaa_samples;
    needs_redraw
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
)]
#[allow(
    clippy::too_many_lines,
    reason = "one cohesive widget-construction pass; splitting it would scatter GTK setup order"
)]
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
                let center_x = (bbox.width() as f32).mul_add(0.5, bbox.x() as f32);
                let center_y = (bbox.height() as f32).mul_add(0.5, bbox.y() as f32);
                waterui_core::layout::Point::new(center_x * scale_factor, center_y * scale_factor)
            });
            st.last_pinch_update = Some(Instant::now());
            area.queue_render();
        }
    });
    area.add_controller(zoom);
}

/// Sizes the `GLArea` to honor the surface's layout contract instead of
/// assuming it is greedy on both axes: a non-stretch axis takes its extent
/// from the view's own measurement (an aspect-ratio renderer, a fixed-size
/// gauge).
fn apply_stretch_sizing(area: &gtk4::GLArea, gpu_surface: &GpuSurface) {
    let stretch = gpu_surface.stretch_axis();
    area.set_hexpand(stretch.stretches_horizontal());
    area.set_vexpand(stretch.stretches_vertical());
    if !(stretch.stretches_horizontal() && stretch.stretches_vertical()) {
        let measured = gpu_surface.measure(ProposalSize::UNSPECIFIED).size;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "GTK size requests are integer logical pixels"
        )]
        area.set_size_request(
            if stretch.stretches_horizontal() {
                -1
            } else {
                measured.width.ceil() as i32
            },
            if stretch.stretches_vertical() {
                -1
            } else {
                measured.height.ceil() as i32
            },
        );
    }
}

/// Delivers `GtkGLArea` input to a GPU view that asked to handle its own.
///
/// The surface is briefly absent from the state while an in-flight `setup`
/// owns it; an event that lands then is dropped, which is what a view that has
/// not finished initializing can do with it anyway.
struct GpuSurfaceInput {
    area: gtk4::GLArea,
    state: Rc<RefCell<GpuState>>,
}

impl GpuSurfaceInput {
    fn new(area: &gtk4::GLArea, state: &Rc<RefCell<GpuState>>) -> Self {
        Self {
            area: area.clone(),
            state: Rc::clone(state),
        }
    }
}

impl SurfaceInputSink for GpuSurfaceInput {
    fn handle(&self, event: &SurfaceInputEvent) {
        {
            let mut state = self.state.borrow_mut();
            let Some(surface) = state.gpu_surface.as_mut() else {
                return;
            };
            surface.input(event);
        }
        // The view draws its own response to the event, and this widget renders
        // on demand.
        self.area.queue_render();
    }
}

pub(crate) fn render_gpu_surface(gpu_surface: GpuSurface, env: Environment) -> gtk4::Widget {
    tracing::debug!("[gtk-gpu] create GLArea widget");
    let area = gtk4::GLArea::new();
    apply_stretch_sizing(&area, &gpu_surface);
    area.set_visible(true);
    area.set_can_target(true);
    area.set_auto_render(false);
    area.set_has_depth_buffer(false);
    area.set_has_stencil_buffer(false);

    let wants_input_events = gpu_surface.wants_input_events();
    let state = Rc::new(RefCell::new(GpuState::new(gpu_surface, env)));
    install_input_controllers(&area, &state);
    // A view that draws its own interactive content — a browser page, a
    // terminal, an editor — takes the raw events instead of the per-frame
    // pointer snapshot. GTK delivers them to the `GtkGLArea`'s own event
    // controllers rather than through a renderer that hit-tests layers, so this
    // is where the routing lives; nothing here knows what the surface is.
    if wants_input_events {
        area.set_focusable(true);
        install_surface_input(&area, Rc::new(GpuSurfaceInput::new(&area, &state)));
    }

    area.connect_realize({
        let state = Rc::clone(&state);
        move |area| {
            tracing::debug!("[gtk-gpu] GLArea realize");
            area.make_current();
            let err = area.error();
            assert!(
                err.is_none(),
                "GpuSurface(GL): GtkGLArea realize failed: {err:?}"
            );
            // The redraw handle is replaced on unrealize, so the waker is
            // (re-)installed per realized context.
            install_redraw_waker(area, &state);
        }
    });

    area.connect_map(|area| {
        tracing::debug!(
            "[gtk-gpu] GLArea map size={}x{}",
            area.width(),
            area.height()
        );
        area.queue_render();
    });

    area.connect_unrealize({
        let state = Rc::clone(&state);
        move |area| {
            // Drop wgpu objects while the GtkGLArea context is still current.
            area.make_current();
            let err = area.error();
            assert!(
                err.is_none(),
                "GpuSurface(GL): GtkGLArea unrealize error: {err:?}"
            );

            let mut st = state.borrow_mut();
            // Anything still in flight against this context (a pending device
            // request, an unfinished renderer setup) must not resurrect state
            // for a context that no longer exists.
            st.context_generation += 1;
            st.cached_target = None;
            st.wgpu_queue = None;
            st.wgpu_device = None;
            st.device_init_in_progress = false;
            // Compiled shader modules and vello pipelines belong to the device
            // that just died; the next realized context builds its own.
            st.device_shared = None;
            st.wgpu_adapter = None;
            st.wgpu_instance = None;
            st.glow = None;
            st.setup_done = false;
            st.setup_in_progress = false;
            st.redraw_handle.set_waker(None);
            st.redraw_handle = RedrawHandle::new();
            st.last_size = None;
        }
    });

    area.connect_render({
        let state = Rc::clone(&state);
        move |area, gl_ctx| {
            tracing::debug!("[gtk-gpu] GLArea render callback");
            area.make_current();
            let err = area.error();
            assert!(
                err.is_none(),
                "GpuSurface(GL): GtkGLArea render error: {err:?}"
            );

            init_wgpu_if_needed(area, &state, gl_ctx);

            tracing::debug!("[gtk-gpu] GLArea render callback: setup_if_needed");
            if !setup_if_needed(area, &state) {
                area.queue_render();
                return gtk4::glib::Propagation::Stop;
            }

            tracing::debug!("[gtk-gpu] GLArea render callback: render_frame");
            if render_frame(area, &state) {
                area.queue_render();
            }

            gtk4::glib::Propagation::Stop
        }
    });

    area.upcast()
}

impl GtkComponent for Native<GpuSurface> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        render_gpu_surface(self.into_inner(), env.clone())
    }
}

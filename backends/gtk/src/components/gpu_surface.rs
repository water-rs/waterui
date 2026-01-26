//! `GpuSurface` native renderer for GTK.
//!
//! On macOS, GTK uses the Cocoa backend. We render WaterUI's `GpuSurface` by
//! creating a per-view `CAMetalLayer` and presenting via `wgpu` using
//! `SurfaceTargetUnsafe::CoreAnimationLayer`.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_graphics::gpu_surface::{GestureState, GpuContext, GpuFrame, GpuSurface, PointerState};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gtk4::prelude::*;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSView, NSWindow};
    use objc2_core_graphics::{CGPoint, CGRect, CGSize};
    use objc2_quartz_core::{CAMetalLayer, CALayer};

    use waterui_graphics::gpu_surface::{preferred_msaa_samples, preferred_surface_format};

    use super::*;

    #[derive(Debug)]
    struct GpuSurfaceState {
        // WaterUI object that owns the user renderer.
        gpu_surface: Option<GpuSurface>,
        // Native Metal host objects.
        content_view: Option<Retained<NSView>>,
        overlay_view: Option<Retained<NSView>>,
        metal_layer: Option<Retained<CAMetalLayer>>,
        // wgpu objects (created asynchronously).
        instance: Option<wgpu::Instance>,
        surface: Option<wgpu::Surface<'static>>,
        adapter: Option<wgpu::Adapter>,
        device: Option<wgpu::Device>,
        queue: Option<wgpu::Queue>,
        config: Option<wgpu::SurfaceConfiguration>,
        msaa_samples: u32,
        // Render lifecycle.
        setup_in_flight: bool,
        configured: bool,
        // Input state (pixel coordinates).
        pointer: PointerState,
        gesture: GestureState,
        // Cached scale factor for pixel conversion.
        scale_factor: i32,
    }

    impl GpuSurfaceState {
        fn new(gpu_surface: GpuSurface) -> Self {
            Self {
                gpu_surface: Some(gpu_surface),
                content_view: None,
                overlay_view: None,
                metal_layer: None,
                instance: None,
                surface: None,
                adapter: None,
                device: None,
                queue: None,
                config: None,
                msaa_samples: 1,
                setup_in_flight: false,
                configured: false,
                pointer: PointerState::default(),
                gesture: GestureState::default(),
                scale_factor: 1,
            }
        }

        fn pixel_size_for_widget(widget: &gtk4::Widget, scale_factor: i32) -> (u32, u32) {
            let w = widget.allocated_width().max(1) as u32;
            let h = widget.allocated_height().max(1) as u32;
            let scale = scale_factor.max(1) as u32;
            (w.saturating_mul(scale), h.saturating_mul(scale))
        }
    }

    fn get_nswindow_from_widget(widget: &gtk4::Widget) -> Retained<NSWindow> {
        let root = widget
            .root()
            .unwrap_or_else(|| panic!("GpuSurface: widget must be rooted before realize"));
        let surface = root
            .surface()
            .unwrap_or_else(|| panic!("GpuSurface: GtkNative must have a GdkSurface"));

        let macos_surface = surface
            .downcast::<gdk4_macos::MacosSurface>()
            .unwrap_or_else(|_| panic!("GpuSurface: expected GdkMacosSurface on macOS"));

        let native_window = macos_surface.native();
        let ptr = native_window as *mut AnyObject;
        let ptr = ptr.cast::<NSWindow>();

        // SAFETY: GDK returns a valid `NSWindow*` that remains alive while the
        // window/surface is alive. We retain it for our own lifetime tracking.
        unsafe { Retained::retain(ptr) }
            .unwrap_or_else(|| panic!("GpuSurface: failed to retain NSWindow from GDK"))
    }

    fn make_overlay_view(
        content_view: &Retained<NSView>,
        mtm: MainThreadMarker,
    ) -> (Retained<NSView>, Retained<CAMetalLayer>) {
        let overlay_view = NSView::new(mtm);
        overlay_view.setWantsLayer(true);

        let metal_layer = CAMetalLayer::new();
        metal_layer.setGeometryFlipped(true);
        metal_layer.setOpaque(false);

        // SAFETY: CAMetalLayer is a CALayer subclass.
        overlay_view.setLayer(Some(metal_layer.as_ref().as_ref()));

        content_view.addSubview(&overlay_view);

        (overlay_view, metal_layer)
    }

    fn update_overlay_geometry(
        widget: &gtk4::Widget,
        content_view: &Retained<NSView>,
        overlay_view: &Retained<NSView>,
        metal_layer: &Retained<CAMetalLayer>,
        scale_factor: i32,
    ) {
        let root = widget
            .root()
            .unwrap_or_else(|| panic!("GpuSurface: widget must be rooted"));
        let root_widget: gtk4::Widget = root.upcast();
        let bounds = widget
            .compute_bounds(&root_widget)
            .unwrap_or_else(|| panic!("GpuSurface: failed to compute widget bounds in root"));

        let x = bounds.x() as f64;
        let y = bounds.y() as f64;
        let w = bounds.width().max(1.0) as f64;
        let h = bounds.height().max(1.0) as f64;

        // GTK's coordinate system is top-left. Cocoa can be flipped or not.
        let flipped = content_view.isFlipped();
        let content_h = content_view.bounds().size.height;
        let y = if flipped { y } else { content_h - y - h };

        let frame = CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        overlay_view.setFrame(frame);

        let scale = scale_factor.max(1) as f64;
        metal_layer.setDrawableSize(CGSize::new(w * scale, h * scale));
    }

    fn install_pointer_controllers(widget: &gtk4::Widget, state: &Rc<RefCell<GpuSurfaceState>>) {
        // Motion (hover).
        let motion = gtk4::EventControllerMotion::new();
        motion.connect_enter({
            let state = Rc::clone(state);
            move |_ctrl, x, y| {
                let mut state = state.borrow_mut();
                let scale = state.scale_factor.max(1) as f32;
                state.pointer.position = Some(waterui_core::layout::Point::new(
                    x as f32 * scale,
                    y as f32 * scale,
                ));
            }
        });
        motion.connect_motion({
            let state = Rc::clone(state);
            move |_ctrl, x, y| {
                let mut state = state.borrow_mut();
                let scale = state.scale_factor.max(1) as f32;
                state.pointer.position = Some(waterui_core::layout::Point::new(
                    x as f32 * scale,
                    y as f32 * scale,
                ));
            }
        });
        motion.connect_leave({
            let state = Rc::clone(state);
            move |_ctrl| {
                let mut state = state.borrow_mut();
                state.pointer.position = None;
            }
        });
        widget.add_controller(motion);

        // Press state.
        let click = gtk4::GestureClick::new();
        click.set_button(0);
        click.connect_pressed({
            let state = Rc::clone(state);
            move |_gesture, _n_press, x, y| {
                let mut state = state.borrow_mut();
                let scale = state.scale_factor.max(1) as f32;
                let p = waterui_core::layout::Point::new(x as f32 * scale, y as f32 * scale);
                state.pointer.hit = Some(p);
            }
        });
        click.connect_released({
            let state = Rc::clone(state);
            move |_gesture, _n_press, _x, _y| {
                let mut state = state.borrow_mut();
                state.pointer.hit = None;
            }
        });
        widget.add_controller(click);
    }

    fn schedule_wgpu_init(widget: &gtk4::Widget, state: Rc<RefCell<GpuSurfaceState>>) {
        let widget = widget.clone();
        gtk4::glib::MainContext::default().spawn_local(async move {
            let (layer_ptr, scale_factor, width, height) = {
                let mut state_ref = state.borrow_mut();

                let layer = state_ref
                    .metal_layer
                    .as_ref()
                    .unwrap_or_else(|| panic!("GpuSurface: CAMetalLayer must exist before init"));

                let scale = state_ref.scale_factor.max(1);
                let (w, h) = GpuSurfaceState::pixel_size_for_widget(&widget, scale);
                (Retained::as_ptr(layer) as *mut std::ffi::c_void, scale, w, h)
            };

            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::METAL,
                ..Default::default()
            });

            let surface = unsafe {
                instance
                    .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer_ptr))
                    .unwrap_or_else(|e| panic!("GpuSurface: failed to create wgpu Surface: {e}"))
            };

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .unwrap_or_else(|e| panic!("GpuSurface: request_adapter failed: {e}"));

            let adapter_limits = adapter.limits();
            let required_limits = wgpu::Limits::default().using_resolution(adapter_limits);

            let (device, queue) = adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        label: Some("WaterUI GTK GpuSurface Device"),
                        required_features: wgpu::Features::empty(),
                        required_limits,
                        memory_hints: wgpu::MemoryHints::Performance,
                        experimental_features: wgpu::ExperimentalFeatures::default(),
                        trace: wgpu::Trace::default(),
                    },
                    None,
                )
                .await
                .unwrap_or_else(|e| panic!("GpuSurface: request_device failed: {e}"));

            device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
                tracing::error!("[wgpu] Uncaptured error: {error}");
            }));

            let caps = surface.get_capabilities(&adapter);
            let format = preferred_surface_format(&caps);
            let present_mode = caps
                .present_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::PresentMode::Fifo)
                .unwrap_or_else(|| {
                    *caps
                        .present_modes
                        .first()
                        .unwrap_or(&wgpu::PresentMode::AutoVsync)
                });
            let alpha_mode = caps
                .alpha_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied)
                .or_else(|| caps.alpha_modes.first().copied())
                .unwrap_or(wgpu::CompositeAlphaMode::Auto);

            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width,
                height,
                present_mode,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);

            let msaa_samples = preferred_msaa_samples(&adapter, format, 4);

            let mut state_ref = state.borrow_mut();
            state_ref.instance = Some(instance);
            state_ref.surface = Some(surface);
            state_ref.adapter = Some(adapter);
            state_ref.device = Some(device);
            state_ref.queue = Some(queue);
            state_ref.config = Some(config);
            state_ref.msaa_samples = msaa_samples;
            state_ref.configured = true;
            state_ref.scale_factor = scale_factor;
        });
    }

    fn schedule_setup(state: Rc<RefCell<GpuSurfaceState>>) {
        gtk4::glib::MainContext::default().spawn_local(async move {
            let (mut gpu_surface, device, queue, adapter, format, msaa_samples) = {
                let mut state_ref = state.borrow_mut();
                if state_ref.setup_in_flight {
                    return;
                }
                if !state_ref.configured {
                    return;
                }

                state_ref.setup_in_flight = true;

                let gpu_surface = state_ref
                    .gpu_surface
                    .take()
                    .unwrap_or_else(|| panic!("GpuSurface: missing GpuSurface during setup"));
                let device = state_ref
                    .device
                    .as_ref()
                    .unwrap_or_else(|| panic!("GpuSurface: device must exist before setup"))
                    .clone();
                let queue = state_ref
                    .queue
                    .as_ref()
                    .unwrap_or_else(|| panic!("GpuSurface: queue must exist before setup"))
                    .clone();
                let adapter = state_ref
                    .adapter
                    .as_ref()
                    .unwrap_or_else(|| panic!("GpuSurface: adapter must exist before setup"))
                    .clone();
                let format = state_ref
                    .config
                    .as_ref()
                    .unwrap_or_else(|| panic!("GpuSurface: config must exist before setup"))
                    .format;
                let msaa_samples = state_ref.msaa_samples;
                (gpu_surface, device, queue, adapter, format, msaa_samples)
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

            let mut state_ref = state.borrow_mut();
            state_ref.gpu_surface = Some(gpu_surface);
            state_ref.setup_in_flight = false;
        });
    }

    fn render_frame(widget: &gtk4::Widget, state: &Rc<RefCell<GpuSurfaceState>>) {
        let (device, queue, adapter, surface, mut gpu_surface, mut config, pointer, gesture) = {
            let mut state_ref = state.borrow_mut();
            if state_ref.setup_in_flight || !state_ref.configured {
                return;
            }

            let Some(surface) = state_ref.surface.as_ref() else { return };
            let Some(device) = state_ref.device.as_ref() else { return };
            let Some(queue) = state_ref.queue.as_ref() else { return };
            let Some(adapter) = state_ref.adapter.as_ref() else { return };
            let Some(config) = state_ref.config.as_ref() else { return };
            let Some(gpu_surface) = state_ref.gpu_surface.take() else {
                // Setup might still be running; avoid re-entrancy.
                return;
            };

            (
                device.clone(),
                queue.clone(),
                adapter.clone(),
                // `wgpu::Surface` is not Clone; keep it borrowed by re-borrowing later.
                (),
                gpu_surface,
                config.clone(),
                state_ref.pointer,
                state_ref.gesture,
            )
        };

        let _ = (queue, adapter, surface, device, config, pointer, gesture, widget);
        let _ = gpu_surface;
        panic!("GpuSurface: internal error - render path not initialized correctly");
    }

    pub(super) fn render_gpu_surface(mut gpu_surface: GpuSurface) -> gtk4::Widget {
        let area = gtk4::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_can_target(true);

        let state = Rc::new(RefCell::new(GpuSurfaceState::new(gpu_surface)));

        // Track tick callback id so we can stop it when unrealized.
        let tick_enabled = Rc::new(Cell::new(false));

        install_pointer_controllers(&area.clone().upcast(), &state);

        area.connect_realize({
            let state = Rc::clone(&state);
            let tick_enabled = Rc::clone(&tick_enabled);
            move |area| {
                let widget: gtk4::Widget = area.clone().upcast();

                let root = widget
                    .root()
                    .unwrap_or_else(|| panic!("GpuSurface: widget must be rooted before realize"));
                let surface = root
                    .surface()
                    .unwrap_or_else(|| panic!("GpuSurface: GtkNative must have a GdkSurface"));

                let scale_factor = surface.scale_factor();
                {
                    let mut st = state.borrow_mut();
                    st.scale_factor = scale_factor.max(1);
                }

                let ns_window = get_nswindow_from_widget(&widget);
                let content_view = ns_window
                    .contentView()
                    .unwrap_or_else(|| panic!("GpuSurface: NSWindow has no contentView"));

                let mtm = MainThreadMarker::new().unwrap_or_else(|| {
                    panic!("GpuSurface: must be created on the main thread (Cocoa requirement)")
                });

                let (overlay_view, metal_layer) = make_overlay_view(&content_view, mtm);

                update_overlay_geometry(
                    &widget,
                    &content_view,
                    &overlay_view,
                    &metal_layer,
                    scale_factor,
                );

                {
                    let mut st = state.borrow_mut();
                    st.content_view = Some(content_view);
                    st.overlay_view = Some(overlay_view);
                    st.metal_layer = Some(metal_layer);
                }

                schedule_wgpu_init(&widget, Rc::clone(&state));

                // Start driving frames from GTK's frame clock.
                if !tick_enabled.get() {
                    tick_enabled.set(true);
                    widget.add_tick_callback({
                        let state = Rc::clone(&state);
                        move |widget, _clock| {
                            let (content_view, overlay_view, metal_layer, scale_factor) = {
                                let st = state.borrow();
                                match (
                                    st.content_view.as_ref(),
                                    st.overlay_view.as_ref(),
                                    st.metal_layer.as_ref(),
                                ) {
                                    (Some(c), Some(v), Some(l)) => {
                                        (c.clone(), v.clone(), l.clone(), st.scale_factor)
                                    }
                                    _ => return gtk4::glib::ControlFlow::Continue,
                                }
                            };

                            update_overlay_geometry(
                                widget,
                                &content_view,
                                &overlay_view,
                                &metal_layer,
                                scale_factor,
                            );

                            // Kick async setup once wgpu is configured.
                            schedule_setup(Rc::clone(&state));

                            // Real rendering is wired up once `render_frame` is implemented.
                            gtk4::glib::ControlFlow::Continue
                        }
                    });
                }
            }
        });

        area.connect_unrealize({
            let state = Rc::clone(&state);
            move |_area| {
                let mut st = state.borrow_mut();
                if let Some(overlay) = st.overlay_view.take() {
                    overlay.removeFromSuperview();
                }
                st.metal_layer = None;
                st.content_view = None;
                st.configured = false;
            }
        });

        area.upcast()
    }
}

impl GtkComponent for Native<GpuSurface> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        #[cfg(target_os = "macos")]
        {
            return macos::render_gpu_surface(self.into_inner());
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = self;
            panic!("GpuSurface: GTK backend currently supports GPU surfaces only on macOS");
        }
    }
}


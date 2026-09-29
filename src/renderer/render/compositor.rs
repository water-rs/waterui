use super::*;
use core::num::NonZeroU32;
use kurbo::Shape;
#[cfg(hydrolysis_macos_system_webview)]
use objc2::rc::Retained;
#[cfg(hydrolysis_macos_system_webview)]
use objc2_web_kit::WKWebView;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use shaderloom::WgslModuleCache;
use waterui_graphics::input::SurfaceInputEvent;

#[derive(Default)]
pub(crate) struct Compositor {
    pub(crate) render_layers: Vec<RenderLayer>,
    pub(crate) active_scene_layers: Vec<ActiveSceneLayer>,
}

pub(crate) struct EmbeddedGpuSurfaceRuntime {
    surface: Option<GpuSurface>,
    env: Option<Environment>,
    setup_complete: bool,
    /// Whether the view handles its own keyboard, IME, pointer and scroll
    /// input. Sampled once at construction: the surface is moved out for the
    /// duration of async setup, and a target that disappeared for those frames
    /// would drop the focus it holds.
    wants_input_events: bool,
    /// Whether the view fills every pixel it is handed opaquely, and may
    /// therefore be rendered straight into the window's uncleared texture.
    /// Sampled once at construction for the same reason as
    /// `wants_input_events`: the surface is moved out for the duration of async
    /// setup, and the layer-pushing path must still be able to ask.
    is_opaque: bool,
    msaa_samples: NonZeroU32,
    prefers_hdr: Option<bool>,
    output_format: wgpu::TextureFormat,
    output_texture: Option<wgpu::Texture>,
    output_view: Option<wgpu::TextureView>,
    gesture: GestureState,
    trackpad_pan_ending: bool,
    redraw_handle: RedrawHandle,
    /// An unserved redraw request: the view asked for another frame, input
    /// reached it, or an off-thread [`RedrawHandle`] fired. The handle's dirty
    /// flag is consumed by
    /// [`poll_gpu_surface_redraw_handles`](crate::HydrolysisRenderer::poll_gpu_surface_redraw_handles)
    /// at the top of the frame, well before the render path runs, so the
    /// request is recorded here instead of being left on the handle for
    /// [`Self::prepare_layer`] to find.
    pending_render: bool,
    /// What the retained [`Self::output_texture`] currently shows: the inputs
    /// the view was last rendered with. `None` means the texture holds nothing
    /// this runtime drew — before the first frame, after a resize or format
    /// change recreated it, after setup replaced the view's GPU resources, and
    /// while the surface renders straight into the window instead.
    ///
    /// This is the whole of the render-on-demand test: a frame whose inputs
    /// equal these, with nothing pending, would redraw the same pixels, so it
    /// composites the texture it already has.
    rendered_inputs: Option<RenderedFrameInputs>,
    /// First frame instant, fixed when the surface first renders; the frame
    /// clock's origin for `GpuFrame::elapsed`.
    start_time: Option<Instant>,
    /// Frame instant of the previous frame, for `GpuFrame::delta`. Advanced on
    /// skipped frames too: see [`Self::frame_timing`].
    last_frame_time: Option<Instant>,
}

/// Everything a rendered frame of an embedded surface depended on, besides the
/// clock. Two frames agreeing on all of it draw the same pixels, so the second
/// one does not have to run.
///
/// Pointer and gesture belong here because views sample them per frame rather
/// than requesting a redraw when they change: a particle field that repels
/// under the cursor, or a shader that highlights on hover, never calls
/// `GpuFrame::request_redraw`, and would freeze if only explicit requests
/// re-rendered it.
///
/// Comparing the whole struct is deliberate: a field added here without a
/// matching thought about staleness fails closed (an extra render) rather than
/// open (a stale texture).
#[derive(Clone, Copy, PartialEq)]
struct RenderedFrameInputs {
    size: (u32, u32),
    scale: f64,
    pointer: PointerState,
    gesture: GestureState,
}

#[derive(Clone)]
struct EmbeddedGpuSurfaceSetup {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader_cache: Arc<WgslModuleCache>,
    scene_renderer: Arc<waterui_graphics::SharedSceneRenderer>,
    host_redraw_handle: Option<RedrawHandle>,
    /// The outer context's handle, so a nested view's worker observes the same
    /// device loss its host does.
    device_loss: waterui_graphics::DeviceLoss,
}

#[derive(Clone)]
pub(crate) enum LayerShape {
    Rect(kurbo::Rect),
    RoundedRect {
        path: kurbo::BezPath,
        #[cfg_attr(
            not(hydrolysis_macos_system_webview),
            expect(
                dead_code,
                reason = "rounded geometry is consumed by macOS native-view clipping"
            )
        )]
        rect: kurbo::Rect,
        #[cfg_attr(
            not(hydrolysis_macos_system_webview),
            expect(
                dead_code,
                reason = "rounded geometry is consumed by macOS native-view clipping"
            )
        )]
        corner_width: f64,
        #[cfg_attr(
            not(hydrolysis_macos_system_webview),
            expect(
                dead_code,
                reason = "rounded geometry is consumed by macOS native-view clipping"
            )
        )]
        corner_height: f64,
    },
    Path(kurbo::BezPath),
}

#[derive(Clone)]
pub(crate) struct ActiveSceneLayer {
    pub(crate) alpha: f32,
    pub(crate) transform: kurbo::Affine,
    pub(crate) shape: LayerShape,
}

/// Where a [`GpuSurfaceLayer`]'s runtime lives. The retained render tree owns
/// its runtime directly inside its `GpuSurfaceNode` (`Owned`), so a reactive
/// swap renders the new surface and a per-frame re-flush re-binds the same
/// runtime structurally — no cursor desync.
#[derive(Clone)]
pub(crate) enum GpuSurfaceSource {
    Owned(Rc<RefCell<EmbeddedGpuSurfaceRuntime>>),
}

#[derive(Clone)]
pub(crate) struct GpuSurfaceLayer {
    pub(crate) source: GpuSurfaceSource,
    /// The mount identity: which visual node's surface this frame presents.
    pub(crate) key: crate::renderer::retained::RenderKey,
    pub(crate) transform: kurbo::Affine,
    pub(crate) bounds: kurbo::Rect,
    /// The surface's rect in window hit-test space, used to project the
    /// window pointer into surface-local coordinates at composite time.
    pub(crate) hit_rect: kurbo::Rect,
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
    pub(crate) direct_to_target: bool,
}

#[cfg(hydrolysis_macos_system_webview)]
#[derive(Clone)]
pub(crate) struct NativeViewLayer {
    pub(crate) view: Retained<WKWebView>,
    pub(crate) transform: kurbo::Affine,
    pub(crate) bounds: kurbo::Rect,
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
    /// Where `WaterUI`-drawn interactive content covers this view, in window
    /// hit-test space, refreshed every frame by
    /// [`NativeViewOcclusion`](crate::renderer::NativeViewOcclusion). The view
    /// host refuses AppKit hits inside these rects so the content on top gets
    /// the click it visibly deserves.
    pub(crate) occlusion: Rc<RefCell<Vec<kurbo::Rect>>>,
}

/// A GPU texture produced during the frame's traversal — an effect's output
/// plane — mounted as engine external-frame content at its own layer so it
/// composites under the same clip/opacity ancestry as the scene around it.
pub(crate) struct ExternalTextureLayer {
    pub(crate) key: crate::renderer::retained::RenderKey,
    /// The produced plane, mounted as engine external-frame content.
    pub(crate) texture: wgpu::Texture,
    /// The plane's format; selects the external frame's decode.
    pub(crate) format: wgpu::TextureFormat,
    /// Placement transform mapping `bounds` into scene space.
    pub(crate) transform: kurbo::Affine,
    pub(crate) bounds: kurbo::Rect,
    /// The clip/opacity ancestry the plane is shown under.
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
}

pub(crate) enum RenderLayer {
    Scene(Recording),
    GpuSurface(GpuSurfaceLayer),
    ExternalTexture(ExternalTextureLayer),
    #[cfg(hydrolysis_macos_system_webview)]
    NativeView(NativeViewLayer),
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) struct HybridRenderSegment {
    layers: Vec<RenderLayer>,
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) struct HybridComposition {
    pub(crate) segments: Vec<HybridRenderSegment>,
    pub(crate) native_views: Vec<NativeViewLayer>,
    pub(crate) transient_scene: Option<Recording>,
}

pub(crate) struct PreparedGpuSurfaceLayer {
    /// The frame's produced plane, mounted as engine external-frame content.
    pub(crate) texture: wgpu::Texture,
    /// The plane's format; selects the external frame's decode.
    pub(crate) output_format: wgpu::TextureFormat,
    pub(crate) needs_redraw: bool,
}

pub(crate) fn take_gpu_surface_redraw_request(
    frame_requested_redraw: bool,
    redraw_handle: &RedrawHandle,
) -> bool {
    let external_redraw_requested = redraw_handle.take_dirty();
    frame_requested_redraw || external_redraw_requested
}

pub struct HydrolysisRenderTarget<'a> {
    pub adapter: &'a wgpu::Adapter,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// Reports this device lost; taken when the device was opened.
    pub device_loss: waterui_graphics::DeviceLoss,
    /// The device-creation chain the frame's handles belong to — the engine
    /// pool key.
    pub gpu_context_id: u64,
    /// All four handles of that chain; the shared engine requires them to
    /// come from the same creation chain.
    pub shared_device: cherenkov_gpu::interop::SharedDevice,
    /// Device pixels per logical unit on the display the target shows on.
    pub display_scale: f64,
    /// HDR headroom the display reaches; 1.0 for SDR.
    pub headroom: f32,
    /// `true` for the renderer's mounted output — its engine surface and
    /// mounts persist across frames, keyed by [`Self::gpu_context_id`].
    /// `false` for a transient target (a subtree capture): it renders through
    /// a short-lived engine surface whose mounts die with the call.
    pub persistent: bool,
    pub texture: Option<&'a wgpu::Texture>,
    pub view: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub base_color: peniko::Color,
}

pub(crate) struct DirectGpuSurfaceTarget<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) texture: &'a wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Device pixels per logical unit, from the layer's transform.
    pub(crate) scale: f64,
    pub(crate) pointer: PointerState,
    pub(crate) now: Instant,
}

pub(crate) struct EmbeddedLayerTarget {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) transform: kurbo::Affine,
    pub(crate) bounds: kurbo::Rect,
    /// The surface's rect in window hit-test space; the pointer is projected
    /// into surface-local pixels inside `prepare_layer`, which is where the
    /// layer's output pixel size is decided.
    pub(crate) hit_rect: kurbo::Rect,
    pub(crate) pointer_position: Option<kurbo::Point>,
    pub(crate) pointer_press_origin: Option<kurbo::Point>,
    pub(crate) now: Instant,
}

/// Projects the window pointer into an embedded surface's local pixel space.
///
/// `hit_rect` is the surface's rect in window hit-test coordinates and
/// `(width, height)` its output texture size. The hover position maps only
/// while inside the rect; the press origin maps only when the press started on
/// this surface, so a drag that leaves the bounds keeps reporting its origin.
pub(crate) fn project_pointer_into_surface(
    pointer_position: Option<kurbo::Point>,
    pointer_press_origin: Option<kurbo::Point>,
    hit_rect: kurbo::Rect,
    width: u32,
    height: u32,
) -> PointerState {
    if hit_rect.width() <= 0.0 || hit_rect.height() <= 0.0 {
        return PointerState::default();
    }
    #[allow(clippy::cast_possible_truncation)]
    let map = |point: kurbo::Point| {
        waterui_core::layout::Point::new(
            ((point.x - hit_rect.x0) / hit_rect.width() * f64::from(width)) as f32,
            ((point.y - hit_rect.y0) / hit_rect.height() * f64::from(height)) as f32,
        )
    };
    let position = pointer_position
        .filter(|point| hit_rect.contains(*point))
        .map(map);
    let hit = pointer_press_origin
        .filter(|origin| hit_rect.contains(*origin))
        .map(map);
    PointerState { position, hit }
}

/// One layer fully prepared for the final composite pass: its content and mask
/// views (pooled textures ride along so they return to the pool afterwards)
/// plus the 80-byte compositor uniform.
impl ActiveSceneLayer {
    pub(crate) fn push_to_scene(&self, scene: &mut Recording) {
        match &self.shape {
            LayerShape::Rect(rect) => {
                scene.push_group(
                    peniko::Fill::NonZero,
                    peniko::BlendMode::default(),
                    self.alpha,
                    self.transform,
                    rect,
                );
            }
            LayerShape::RoundedRect { path, .. } | LayerShape::Path(path) => {
                scene.push_group(
                    peniko::Fill::NonZero,
                    peniko::BlendMode::default(),
                    self.alpha,
                    self.transform,
                    path,
                );
            }
        }
    }
}

impl EmbeddedGpuSurfaceRuntime {
    pub(crate) fn new(surface: GpuSurface, env: &Environment) -> Self {
        let msaa_samples = surface.msaa_sample_limit();
        let wants_input_events = surface.wants_input_events();
        let is_opaque = surface.is_opaque();
        let prefers_hdr = surface.resolved_hdr_preference().or_else(|| {
            env.get::<DynamicRangePreference>()
                .map(|preference| preference.0)
        });
        Self {
            surface: Some(surface),
            env: Some(env.clone()),
            setup_complete: false,
            wants_input_events,
            is_opaque,
            msaa_samples,
            prefers_hdr,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            output_texture: None,
            output_view: None,
            gesture: GestureState::new(),
            trackpad_pan_ending: false,
            redraw_handle: RedrawHandle::new(),
            pending_render: false,
            rendered_inputs: None,
            start_time: None,
            last_frame_time: None,
        }
    }

    /// Advances the surface's animation clock to the renderer's frame instant
    /// and returns `(elapsed, delta)` for this frame. Driving the clock from
    /// the frame instant (not wall time) keeps offscreen hosts that pump the
    /// clock deterministic.
    ///
    /// The clock runs on every window frame, including the ones
    /// [`Self::prepare_layer`] skips: `elapsed` is wall-clock from the
    /// surface's first frame, and `delta` is the step since the previous
    /// *window* frame rather than since the previous *rendered* one. A skip
    /// means the view declared its state frozen, so nothing moved during it;
    /// resuming with one frame's step continues the animation from where it
    /// paused, where measuring from the last render would hand it the whole
    /// idle gap (capped at 100ms) and jump it forward by time in which it
    /// deliberately did not move. A view that animates off `elapsed` requests
    /// redraws to do so and therefore never skips a frame in the first place.
    fn frame_timing(&mut self, now: Instant) -> (Duration, Duration) {
        let start = *self.start_time.get_or_insert(now);
        let elapsed = now.saturating_duration_since(start);
        let delta = self.last_frame_time.map_or_else(
            || crate::TARGET_FRAME_INTERVAL,
            |last| {
                now.saturating_duration_since(last)
                    .min(Duration::from_millis(100))
            },
        );
        self.last_frame_time = Some(now);
        (elapsed, delta)
    }

    /// Consumes an off-thread redraw request, recording it as pending so the
    /// render path still sees it: the poll that calls this runs at the top of
    /// the frame and would otherwise be the only thing that ever learns of it.
    pub(crate) fn take_external_redraw_request(&mut self) -> bool {
        let requested = self.redraw_handle.take_dirty();
        self.pending_render |= requested;
        requested
    }

    /// Marks the view as owing a frame and wakes the host that would sleep
    /// through it.
    fn request_render(&mut self) {
        self.pending_render = true;
        self.redraw_handle.request_redraw();
    }

    /// Folds a finished render's outcome into the pending-render state and
    /// reports whether the window must schedule another frame for it.
    ///
    /// Requests raised *during* the render — by the view itself, or by a signal
    /// watcher its scene content installed — land on the handle after the
    /// decision to render was taken, which is why they are collected here
    /// rather than left for the next frame's poll to race over.
    fn settle_after_render(&mut self, frame_requested_redraw: bool) -> bool {
        self.pending_render =
            take_gpu_surface_redraw_request(frame_requested_redraw, &self.redraw_handle);
        self.pending_render
    }

    /// Whether this surface's view handles its own input.
    pub(crate) const fn wants_input_events(&self) -> bool {
        self.wants_input_events
    }

    /// Whether this surface's view declares that it fills every pixel opaquely.
    ///
    /// Only such a view may be rendered straight into the window's target,
    /// which arrives uncleared and still holding the previous frame; a view
    /// that leaves any pixel alone is composited over the window's base colour
    /// instead.
    pub(crate) const fn is_opaque(&self) -> bool {
        self.is_opaque
    }

    /// Delivers one backend-neutral input event to the view.
    ///
    /// Input reaching a surface whose view is still being set up has no
    /// receiver: the view is moved out for the duration of that async setup.
    pub(crate) fn input(&mut self, event: &SurfaceInputEvent) {
        let Some(surface) = self.surface.as_mut() else {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                event = ?event,
                "dropped an input event for a GpuSurface that is still setting up"
            );
            return;
        };
        surface.input(event);
        self.request_render();
    }

    /// The view's text caret, in logical surface-local coordinates.
    pub(crate) fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.surface.as_ref().and_then(GpuSurface::ime_caret)
    }

    /// The name the surface's view gives itself for assistive technologies.
    ///
    /// `None` while the surface is moved out for async setup, exactly as a
    /// view that never names itself answers `None`.
    pub(crate) fn accessibility_label(&self) -> Option<String> {
        self.surface
            .as_ref()
            .and_then(GpuSurface::accessibility_label)
    }

    /// The semantic content the surface's view reports — what it *says* —
    /// announced beside its label. `None` during async setup, as above.
    pub(crate) fn accessibility_value(&self) -> Option<String> {
        self.surface
            .as_ref()
            .and_then(GpuSurface::accessibility_value)
    }

    pub(crate) fn handle_trackpad_pan(&mut self, dx: f32, dy: f32, phase: TouchPhase) -> bool {
        match phase {
            TouchPhase::Started => {
                self.gesture.pan_offset = waterui_core::layout::Point::new(dx, dy);
                self.gesture.active = true;
                self.trackpad_pan_ending = false;
            }
            TouchPhase::Moved => {
                if !self.gesture.active {
                    self.gesture.pan_offset = waterui_core::layout::Point::zero();
                    self.gesture.active = true;
                }
                self.trackpad_pan_ending = false;
                self.gesture.pan_offset.x += dx;
                self.gesture.pan_offset.y += dy;
            }
            TouchPhase::Ended => {
                if !self.gesture.active {
                    self.gesture.pan_offset = waterui_core::layout::Point::zero();
                    self.gesture.active = true;
                }
                self.gesture.pan_offset.x += dx;
                self.gesture.pan_offset.y += dy;
                self.trackpad_pan_ending = true;
            }
            TouchPhase::Cancelled => {
                self.trackpad_pan_ending = self.gesture.active;
            }
        }
        self.request_render();
        true
    }

    fn finish_trackpad_pan_frame(&mut self) {
        if self.trackpad_pan_ending {
            self.trackpad_pan_ending = false;
            self.gesture.active = false;
            self.request_render();
        }
    }

    /// Composites this surface into the window, rendering the view first only
    /// when this frame would produce something the retained output texture does
    /// not already hold.
    ///
    /// The window still redraws its whole scene every frame it runs — that part
    /// of Hydrolysis is not negotiable — but an embedded surface's texture is
    /// an *input* to that composite, retained across frames exactly like the
    /// render tree it hangs in. An idle QR code alongside an animating spinner
    /// pays one composite, not one render.
    pub(crate) fn prepare_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: EmbeddedLayerTarget,
    ) -> PreparedGpuSurfaceLayer {
        let top_left = target.transform * kurbo::Point::new(target.bounds.x0, target.bounds.y0);
        let top_right = target.transform * kurbo::Point::new(target.bounds.x1, target.bounds.y0);
        let bottom_right = target.transform * kurbo::Point::new(target.bounds.x1, target.bounds.y1);
        let bottom_left = target.transform * kurbo::Point::new(target.bounds.x0, target.bounds.y1);

        let layer_width =
            edge_length_in_pixels(top_left, top_right, target.width, target.height).max(1);
        let layer_height =
            edge_length_in_pixels(top_left, bottom_left, target.width, target.height).max(1);
        let output_format = self.output_format;
        // Recreating the texture discards whatever it held, so a resize or a
        // format change is itself a reason to render.
        self.ensure_output_target(device, layer_width, layer_height, output_format);

        let (elapsed, delta) = self.frame_timing(target.now);
        let view = self
            .output_view
            .as_ref()
            .expect("hydrolysis embedded GpuSurface missing output view")
            .clone();
        let inputs = RenderedFrameInputs {
            size: (layer_width, layer_height),
            scale: layer_device_scale(target.transform),
            pointer: project_pointer_into_surface(
                target.pointer_position,
                target.pointer_press_origin,
                target.hit_rect,
                layer_width,
                layer_height,
            ),
            gesture: self.gesture,
        };
        // An off-thread handle can fire between this frame's poll and here, so
        // the handle is consulted again rather than trusting `pending_render`
        // alone.
        let externally_requested = self.redraw_handle.take_dirty();
        let needs_redraw = if self.rendered_inputs == Some(inputs)
            && !self.pending_render
            && !externally_requested
        {
            tracing::trace!(
                width = layer_width,
                height = layer_height,
                "reusing an embedded Hydrolysis GPU surface's retained texture"
            );
            false
        } else {
            let texture = self
                .output_texture
                .as_ref()
                .expect("hydrolysis embedded GpuSurface missing output texture");
            let mut frame = GpuFrame::new(
                device,
                queue,
                texture,
                view.clone(),
                output_format,
                layer_width,
                layer_height,
                inputs.scale,
                inputs.pointer,
                inputs.gesture,
                elapsed,
                delta,
            );
            assert!(
                self.setup_complete,
                "hydrolysis embedded GpuSurface used before setup"
            );
            self.surface
                .as_mut()
                .expect("hydrolysis embedded GpuSurface missing after setup")
                .render(&mut frame);
            let frame_requested_redraw = frame.was_redraw_requested();
            drop(frame);
            self.rendered_inputs = Some(inputs);
            // Recorded before the pan settles: `inputs` is what the view was
            // handed, and ending the pan is a gesture change of its own, which
            // must reach the view on a frame of its own.
            self.finish_trackpad_pan_frame();
            self.settle_after_render(frame_requested_redraw)
        };
        PreparedGpuSurfaceLayer {
            texture: self
                .output_texture
                .as_ref()
                .expect("hydrolysis embedded GpuSurface missing output texture")
                .clone(),
            output_format,
            needs_redraw,
        }
    }

    /// Renders the view straight into the window's own texture, for a surface
    /// that covers the whole window with nothing above or below it.
    ///
    /// This path draws somewhere the retained output texture is not, so it
    /// leaves that texture holding pixels from before: a later frame that
    /// composites this surface again (an overlay appeared, so it is no longer
    /// alone) must render, not reuse.
    pub(crate) fn render_direct_to_target(&mut self, target: DirectGpuSurfaceTarget<'_>) -> bool {
        self.rendered_inputs = None;
        let (elapsed, delta) = self.frame_timing(target.now);
        let mut frame = GpuFrame::new(
            target.device,
            target.queue,
            target.texture,
            target.view,
            target.format,
            target.width,
            target.height,
            target.scale,
            target.pointer,
            self.gesture,
            elapsed,
            delta,
        );
        assert!(
            self.setup_complete,
            "hydrolysis embedded GpuSurface used before setup"
        );
        self.surface
            .as_mut()
            .expect("hydrolysis embedded GpuSurface missing after setup")
            .render(&mut frame);
        let frame_requested_redraw = frame.was_redraw_requested();
        drop(frame);
        self.finish_trackpad_pan_frame();
        self.settle_after_render(frame_requested_redraw)
    }

    async fn setup(
        runtime: Rc<RefCell<Self>>,
        resources: EmbeddedGpuSurfaceSetup,
        surface_format: wgpu::TextureFormat,
    ) {
        let (surface, env, msaa_samples, redraw_handle) = {
            let mut runtime = runtime.borrow_mut();
            if runtime.setup_complete && runtime.output_format == surface_format {
                return;
            }
            let surface = runtime
                .surface
                .take()
                .expect("hydrolysis embedded GpuSurface setup started concurrently");
            let env = runtime
                .env
                .take()
                .expect("hydrolysis embedded GpuSurface environment missing before setup");
            runtime.setup_complete = false;
            (
                surface,
                env,
                runtime.msaa_samples,
                runtime.redraw_handle.clone(),
            )
        };

        let wake_parent: Option<Arc<dyn Fn() + Send + Sync>> =
            resources.host_redraw_handle.as_ref().map(|handle| {
                let handle = handle.clone();
                Arc::new(move || handle.request_redraw()) as Arc<dyn Fn() + Send + Sync>
            });
        redraw_handle.set_waker(wake_parent);

        let mut surface = surface;
        let mut env = env;
        {
            // `GpuContext::new` resolves the surface's declared MSAA limit
            // against what the adapter actually supports for this format, so
            // the renderer sees a sample count it can really use rather than
            // the authoring-side cap.
            let context = GpuContext::new(
                &resources.adapter,
                &resources.device,
                &resources.queue,
                surface_format,
                resources.shader_cache.as_ref(),
                &resources.scene_renderer,
                msaa_samples,
                redraw_handle,
                resources.device_loss.clone(),
            );
            surface.setup(&context, &mut env).await;
        }
        let mut runtime = runtime.borrow_mut();
        runtime.surface = Some(surface);
        runtime.env = Some(env);
        runtime.output_format = surface_format;
        runtime.setup_complete = true;
        // Setup rebuilds the view's GPU resources, so nothing the old view left
        // in the output texture is still that view's output.
        runtime.rendered_inputs = None;
    }

    fn ensure_setup(
        runtime: &Rc<RefCell<Self>>,
        resources: EmbeddedGpuSurfaceSetup,
        signals: FrameSignals,
        surface_format: wgpu::TextureFormat,
    ) -> bool {
        {
            let runtime = runtime.borrow();
            if runtime.setup_complete && runtime.output_format == surface_format {
                return true;
            }
            if runtime.surface.is_none() {
                return false;
            }
        }

        let wake_host = resources.host_redraw_handle.clone();
        let runtime = Rc::clone(runtime);
        spawn_local(async move {
            Self::setup(runtime, resources, surface_format).await;
            signals.request_redraw();
            if let Some(handle) = wake_host {
                handle.request_redraw();
            }
        })
        .detach();
        false
    }

    pub(crate) fn output_format_for(
        &self,
        target_format: wgpu::TextureFormat,
    ) -> wgpu::TextureFormat {
        select_embedded_surface_format(target_format, self.prefers_hdr)
    }

    fn ensure_output_target(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        // The texture itself is the source of truth for what it can hold:
        // `setup` assigns `output_format` on its own, so a field comparison
        // would report a match while the texture is still the previous format.
        let matches_request = self.output_texture.as_ref().is_some_and(|texture| {
            texture.width() == width && texture.height() == height && texture.format() == format
        });
        if matches_request {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_embedded_gpu_surface_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.output_format = format;
        self.output_texture = Some(texture);
        self.output_view = Some(view);
        // A fresh texture holds nothing; whatever the view drew at the old size
        // or format is gone with the texture that held it.
        self.rendered_inputs = None;
    }
}

/// The one texture format an embedded surface's view is set up for and renders
/// into, whichever path carries it to the window.
///
/// An SDR surface takes the *target's own linear format* — `Bgra8Unorm` for a
/// `Bgra8Unorm` or `Bgra8UnormSrgb` window, `Rgba8Unorm` for an `Rgba8Unorm`
/// one — rather than a fixed `Rgba8Unorm`. That is what lets the direct path
/// hand the view the window texture itself without the format under it
/// changing: [`HydrolysisRenderer::render_scene_to_surface`] takes that path
/// only when this format already equals the target's, and Hydrolysis
/// normalizes every swapchain to its linear variant when the surface offers
/// one (`normalize_surface_format`), so it does. A window that can only be
/// configured sRGB keeps its surfaces composed; viewing such a swapchain
/// texture through its linear variant instead would need that variant declared
/// in `SurfaceConfiguration::view_formats`, which rests on
/// `DownlevelFlags::VIEW_FORMATS` and is not universal.
///
/// HDR is unchanged: an HDR target under a view that wants HDR renders
/// `Rgba16Float`, and one that asked for SDR renders the target's linear
/// format and is therefore composited.
fn select_embedded_surface_format(
    target_format: wgpu::TextureFormat,
    prefers_hdr_override: Option<bool>,
) -> wgpu::TextureFormat {
    let target_hdr = matches!(
        target_format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    );
    let prefers_hdr = prefers_hdr_override.unwrap_or(true);
    if target_hdr && prefers_hdr {
        return wgpu::TextureFormat::Rgba16Float;
    }
    if target_hdr {
        return wgpu::TextureFormat::Rgba8Unorm;
    }
    target_format.remove_srgb_suffix()
}

fn layer_device_scale(transform: kurbo::Affine) -> f64 {
    transform.determinant().abs().sqrt()
}

/// Where a layer's bounds land on the window's physical pixel grid, when its
/// transform is one the direct-to-target path can honour at all.
///
/// Direct rendering replaces the window's whole pass with the view's own, so
/// the view is handed the target's axis-aligned pixel rectangle and nothing
/// else: there is no quad to map its output through the way
/// [`encode_compositor_uniform`] maps a composited layer's. A rotation, a skew,
/// a mirrored axis or a collapsed one all describe output this path cannot
/// place, so they answer `None` and the surface composites instead.
///
/// A *pure scale* does survive, which is the whole point — the window root's
/// transform is `Affine::scale(scale_factor)`, so on every HiDPI display a
/// full-window surface arrives here scaled by 2 or 3 rather than as the
/// identity. Translation is not rejected outright either; it simply has to come
/// out matching the viewport, which the caller checks against the rect returned
/// here.
fn direct_target_rect(transform: kurbo::Affine, bounds: kurbo::Rect) -> Option<kurbo::Rect> {
    let [x_scale, shear_y, shear_x, y_scale, ..] = transform.as_coeffs();
    // Comparing the transform's linear part against a pure non-uniform scale is
    // the "no rotation, no skew" test, taken at the same tolerance every other
    // affine comparison in the renderer uses. The translation is dropped from
    // both sides: where the surface lands is the caller's question, not this
    // one's.
    if !affine_near(
        kurbo::Affine::new([x_scale, shear_y, shear_x, y_scale, 0.0, 0.0]),
        kurbo::Affine::scale_non_uniform(x_scale, y_scale),
    ) {
        return None;
    }
    // A mirrored axis would flip the view's output, and a collapsed one leaves
    // it no pixels at all; neither is something handing over the window texture
    // can express.
    if x_scale <= 0.0 || y_scale <= 0.0 {
        return None;
    }
    Some(transform.transform_rect_bbox(bounds))
}

/// Whether a layer's transformed bounds cover `viewport` exactly, in physical
/// pixels — the geometric half of the direct-to-target decision.
pub(crate) fn covers_viewport_directly(
    transform: kurbo::Affine,
    bounds: kurbo::Rect,
    viewport: kurbo::Rect,
) -> bool {
    direct_target_rect(transform, bounds).is_some_and(|rect| rect_near(rect, viewport))
}

/// The whole-pixel size of a direct-to-target layer, taken from the same
/// transformed bounds the decision was made on rather than assumed from the
/// window.
fn direct_target_size(transform: kurbo::Affine, bounds: kurbo::Rect) -> (u32, u32) {
    let rect = direct_target_rect(transform, bounds)
        .expect("hydrolysis direct GpuSurface layer must have a direct-renderable transform");
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the rect was accepted only after matching the window's own pixel viewport"
    )]
    (
        rect.width().round().max(1.0) as u32,
        rect.height().round().max(1.0) as u32,
    )
}

fn edge_length_in_pixels(
    start: kurbo::Point,
    end: kurbo::Point,
    target_width: u32,
    target_height: u32,
) -> u32 {
    assert!(
        target_width != 0 && target_height != 0,
        "hydrolysis compositor target size must be non-zero"
    );
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    ((dx * dx + dy * dy).sqrt().round().max(1.0)) as u32
}

impl HydrolysisRenderer {
    fn embedded_gpu_surface_setup(
        &self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        device_loss: waterui_graphics::DeviceLoss,
    ) -> EmbeddedGpuSurfaceSetup {
        EmbeddedGpuSurfaceSetup {
            adapter: adapter.clone(),
            device: device.clone(),
            queue: queue.clone(),
            shader_cache: Arc::clone(&self.shader_cache),
            scene_renderer: Arc::clone(&self.scene_renderer),
            host_redraw_handle: self.host_redraw_handle.clone(),
            device_loss,
        }
    }

    /// Await setup for every GPU surface reachable from the statically built
    /// retained tree. A `HydrolysisGpuView` calls this from its own async setup,
    /// making its first sized frame fully ready without polling or fixed retries.
    pub(crate) async fn setup_embedded_gpu_surfaces(&self, context: &GpuContext<'_>) {
        let runtimes = self.node_gpu_surfaces.clone();
        for runtime in runtimes {
            let surface_format = runtime.borrow().output_format_for(context.surface_format);
            EmbeddedGpuSurfaceRuntime::setup(
                runtime,
                self.embedded_gpu_surface_setup(
                    context.adapter,
                    context.device,
                    context.queue,
                    context.device_loss.clone(),
                ),
                surface_format,
            )
            .await;
        }
    }

    pub fn render_scene_to_texture(&mut self, target: HydrolysisRenderTarget<'_>) {
        self.render_scene_to_surface_with_alpha_mode(target, false, true);
    }

    pub fn render_scene_to_surface(&mut self, target: HydrolysisRenderTarget<'_>) {
        self.render_scene_to_surface_with_alpha_mode(target, false, true);
    }

    /// [`Self::render_scene_to_surface`] with the target's composite alpha
    /// convention made explicit: `premultiply_alpha` selects the alpha mode the
    /// presenter writes into the acquired frame — premultiplied for an OS
    /// surface configured `CompositeAlphaMode::PreMultiplied`, straight for
    /// offscreen/readback targets.
    ///
    /// `rasterize_scene_layers` skips only re-installing segment content: a
    /// frame whose pixels no consumer can read still mounts every layer and
    /// ticks embedded GpuSurface views, but leaves each segment showing the
    /// content it already carries. Capture and presented frames always pass
    /// `true`.
    pub(crate) fn render_scene_to_surface_with_alpha_mode(
        &mut self,
        target: HydrolysisRenderTarget<'_>,
        premultiply_alpha: bool,
        rasterize_scene_layers: bool,
    ) {
        assert!(
            matches!(
                target.format.remove_srgb_suffix(),
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            ) || matches!(
                target.format,
                wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
            ),
            "hydrolysis renderer: unsupported surface format {:?}",
            target.format
        );

        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        self.flush_scene_layer();
        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(target.device, target.queue, 0);
        self.frame_direct_gpu_surfaces = 0;

        let render_layers = core::mem::take(&mut self.compositor.render_layers);
        let transient = self.transient_scene.take().filter(scene_has_content);

        // A sole GPU surface the tree marked `direct_to_target` — it covers
        // the viewport, is opaque, and nothing composites around it —
        // replaces the whole engine pass: it renders straight into the
        // acquired texture and the surface never presents at all.
        if transient.is_none()
            && render_layers.len() == 1
            && let Some(RenderLayer::GpuSurface(layer)) = render_layers.first()
            && layer.direct_to_target
            && let Some(texture) = target.texture
        {
            let mut needs_redraw = false;
            let GpuSurfaceSource::Owned(runtime) = &layer.source;
            let output_format = runtime.borrow().output_format_for(target.format);
            if !EmbeddedGpuSurfaceRuntime::ensure_setup(
                runtime,
                self.embedded_gpu_surface_setup(
                    target.adapter,
                    target.device,
                    target.queue,
                    target.device_loss.clone(),
                ),
                self.frame_signals(),
                output_format,
            ) {
                needs_redraw = true;
            } else {
                let pointer = project_pointer_into_surface(
                    self.hit_test.pointer_position,
                    self.hit_test.pointer_press_origin,
                    layer.hit_rect,
                    target.width,
                    target.height,
                );
                #[cfg(feature = "frame-profile")]
                self.gpu_profile_mark(target.device, target.queue, 1);
                if runtime
                    .borrow_mut()
                    .render_direct_to_target(DirectGpuSurfaceTarget {
                        device: target.device,
                        queue: target.queue,
                        texture,
                        view: target.view.clone(),
                        format: target.format,
                        width: target.width,
                        height: target.height,
                        scale: layer_device_scale(layer.transform),
                        pointer,
                        now: self.frame_instant,
                    })
                {
                    needs_redraw = true;
                }
                #[cfg(feature = "frame-profile")]
                self.gpu_profile_mark(target.device, target.queue, 2);
                self.frame_direct_gpu_surfaces = 1;
            }
            self.compositor.render_layers = render_layers;
            if needs_redraw {
                self.request_redraw();
            }
            return;
        }

        // The shared engine for this frame's device context; `wake` is the
        // host's display-link wake the engine's redraw callback drives.
        let host_wake = self.host_redraw_handle.clone();
        let engine = crate::engine::shared_engine(
            target.gpu_context_id,
            target.adapter,
            target.shared_device.clone(),
            move || {
                if let Some(handle) = &host_wake {
                    handle.request_redraw();
                }
            },
        );

        // One window surface per GPU context this renderer presents through.
        // Entries whose device was reported lost are dropped — their engine
        // surface, mounts and resources all die with the dead device — and a
        // context change mid-session replaces the previous window's mount
        // state with a fresh one. The map leaves `self` for the frame so the
        // layer walk may borrow the renderer's other state.
        let mut windows = core::mem::take(&mut self.cherenkov_windows);
        windows.retain(|_, window| !window.device_loss.is_lost());
        let backend = target.adapter.get_info().backend;
        let context_id = target.gpu_context_id;
        // A persistent output keeps its window (and mounts) across frames; a
        // transient target's window lives for this call only — its mounts and
        // engine surface are created fresh and dropped with the capture.
        let mut transient_window = None;
        let window = if target.persistent {
            windows.entry(context_id).or_insert_with(|| {
                CherenkovWindow::new(
                    engine.clone(),
                    target.device,
                    backend,
                    (target.width, target.height),
                    target.device_loss.clone(),
                )
            })
        } else {
            transient_window.insert(CherenkovWindow::new(
                engine.clone(),
                target.device,
                backend,
                (target.width, target.height),
                target.device_loss.clone(),
            ))
        };

        // The frame's mount plan: the ordered slots under the surface root,
        // the content each slot installs, and the placement/ancestry edits
        // keyed mounts carry.
        let mut order: Vec<crate::renderer::retained::MountSlot> =
            Vec::with_capacity(render_layers.len() + 1);
        let mut live_keys: rustc_hash::FxHashSet<crate::renderer::retained::RenderKey> =
            rustc_hash::FxHashSet::default();
        let mut installs: Vec<(
            crate::renderer::retained::MountSlot,
            cherenkov::LayerContent<cherenkov_gpu::Gpu>,
        )> = Vec::new();
        let mut ancestries: Vec<(
            crate::renderer::retained::RenderKey,
            Vec<crate::renderer::retained::mount::AncestryScope>,
        )> = Vec::new();
        let mut placements: Vec<(crate::renderer::retained::RenderKey, kurbo::Affine)> = Vec::new();
        let mut needs_redraw = false;
        let mut segment_index = 0usize;

        for layer in &render_layers {
            match layer {
                RenderLayer::Scene(recording) => {
                    let slot = crate::renderer::retained::MountSlot::Segment(segment_index);
                    segment_index += 1;
                    order.push(slot);
                    if rasterize_scene_layers {
                        let content = recording.to_content(&mut window.resources);
                        installs.push((slot, content.into()));
                    }
                }
                // Embedded GPU surfaces render serially by design: `GpuView`
                // is a user-facing, main-thread contract (`!Send` setup/render
                // futures, `&mut Environment`), so parallelizing this loop
                // would force `Send` onto every user renderer.
                RenderLayer::GpuSurface(layer) => {
                    if layer_device_scale(layer.transform) <= 0.0 {
                        continue;
                    }
                    let slot = crate::renderer::retained::MountSlot::Keyed(layer.key);
                    order.push(slot);
                    live_keys.insert(layer.key);
                    let GpuSurfaceSource::Owned(runtime) = &layer.source;
                    let output_format = runtime.borrow().output_format_for(target.format);
                    if !EmbeddedGpuSurfaceRuntime::ensure_setup(
                        runtime,
                        self.embedded_gpu_surface_setup(
                            target.adapter,
                            target.device,
                            target.queue,
                            target.device_loss.clone(),
                        ),
                        self.frame_signals(),
                        output_format,
                    ) {
                        needs_redraw = true;
                        continue;
                    }
                    let prepared = runtime.borrow_mut().prepare_layer(
                        target.device,
                        target.queue,
                        EmbeddedLayerTarget {
                            width: target.width,
                            height: target.height,
                            transform: layer.transform,
                            bounds: layer.bounds,
                            hit_rect: layer.hit_rect,
                            pointer_position: self.hit_test.pointer_position,
                            pointer_press_origin: self.hit_test.pointer_press_origin,
                            now: self.frame_instant,
                        },
                    );
                    if prepared.needs_redraw {
                        needs_redraw = true;
                    }
                    let color =
                        if matches!(prepared.output_format, wgpu::TextureFormat::Rgba16Float) {
                            cherenkov_gpu::interop::FrameColor::LINEAR_P3
                        } else {
                            cherenkov_gpu::interop::FrameColor::SRGB
                        };
                    let frame = cherenkov_gpu::interop::ExternalFrame::rgb(
                        prepared.texture.clone(),
                        cherenkov_gpu::interop::RgbAlpha::Premultiplied,
                        color,
                    )
                    .expect(
                        "hydrolysis renderer: embedded surface texture rejected as a frame plane",
                    );
                    installs.push((slot, engine.external_frame(frame).into()));
                    placements.push((
                        layer.key,
                        gpu_frame_transform(layer.transform, layer.bounds, &prepared.texture),
                    ));
                    ancestries.push((layer.key, ancestry_scopes(&layer.active_layers)));
                }
                RenderLayer::ExternalTexture(layer) => {
                    let slot = crate::renderer::retained::MountSlot::Keyed(layer.key);
                    order.push(slot);
                    live_keys.insert(layer.key);
                    let color = if matches!(layer.format, wgpu::TextureFormat::Rgba16Float) {
                        cherenkov_gpu::interop::FrameColor::LINEAR_P3
                    } else {
                        cherenkov_gpu::interop::FrameColor::SRGB
                    };
                    let frame = cherenkov_gpu::interop::ExternalFrame::rgb(
                        layer.texture.clone(),
                        cherenkov_gpu::interop::RgbAlpha::Premultiplied,
                        color,
                    )
                    .expect("hydrolysis renderer: effect output texture rejected as a frame plane");
                    installs.push((slot, engine.external_frame(frame).into()));
                    placements.push((
                        layer.key,
                        gpu_frame_transform(layer.transform, layer.bounds, &layer.texture),
                    ));
                    ancestries.push((layer.key, ancestry_scopes(&layer.active_layers)));
                }
                #[cfg(hydrolysis_macos_system_webview)]
                RenderLayer::NativeView(_) => {
                    panic!(
                        "hydrolysis renderer: native views are not supported on the \
                         Cherenkov output path; the hydrolysis_macos_system_webview \
                         gate is blocked until the platform-host integration lands \
                         (water-rs/hydrolysis#205)"
                    )
                }
            }
        }
        if let Some(recording) = &transient {
            order.push(crate::renderer::retained::MountSlot::Overlay);
            installs.push((
                crate::renderer::retained::MountSlot::Overlay,
                recording.to_content(&mut window.resources).into(),
            ));
        }

        window.surface.resize((target.width, target.height));
        window
            .surface
            .display(target.display_scale, target.headroom);
        window
            .surface
            .clear_color(crate::renderer::recording::working_color(target.base_color));

        let installs_len = installs.len();
        let mounts = &mut window.mounts;
        let surface = window.surface.engine_surface();
        surface.update(|tx| {
            for (slot, content) in installs {
                let layer = mounts.layer(surface, slot);
                tx[layer].content(content);
            }
            for (key, scopes) in ancestries {
                mounts.set_ancestry(surface, tx, key, &scopes);
            }
            for (key, transform) in placements {
                let layer = mounts.layer(surface, crate::renderer::retained::MountSlot::Keyed(key));
                tx[layer].transform(transform);
            }
            mounts.sync_order(surface, tx, &order, &live_keys);
        });
        let window = transient_window.as_mut().unwrap_or_else(|| {
            windows
                .get_mut(&context_id)
                .expect("hydrolysis renderer: window surface lost within a frame")
        });
        self.state.counters.recorded_view_contents +=
            u64::try_from(installs_len).unwrap_or(u64::MAX);
        let (created, removed) = window.mounts.take_frame_stats();
        self.state.counters.layer_creations += created;
        self.state.counters.layer_removals += removed;

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(target.device, target.queue, 1);

        let next = window.surface.render();
        self.engine_next = Some(next);
        let texture = target
            .texture
            .expect("hydrolysis renderer: surface presentation requires the acquired texture");
        window.surface.present_into(
            target.device,
            target.queue,
            texture,
            premultiply_alpha,
            target.headroom,
        );

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(target.device, target.queue, 2);

        self.compositor.render_layers = render_layers;
        self.cherenkov_windows = windows;
        if needs_redraw {
            self.request_redraw();
        }
    }
}

/// One window surface's engine-side state: the `TextureTarget` surface, the
/// stable mounts the frame's `RenderLayer`s show through, and the resource
/// registrations recorded content names.
pub(crate) struct CherenkovWindow {
    pub(crate) surface: crate::engine::CherenkovSurface,
    pub(crate) mounts: crate::renderer::retained::Mounts,
    pub(crate) resources: crate::renderer::recording::SceneResources,
    /// The device-loss token taken when this window's context was opened; a
    /// dead token prunes the entry so a recovered context gets a fresh mount
    /// set instead of reusing a surface on a dead device.
    device_loss: waterui_graphics::DeviceLoss,
}

impl CherenkovWindow {
    pub(crate) fn new(
        engine: Rc<crate::engine::GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
        device_loss: waterui_graphics::DeviceLoss,
    ) -> Self {
        Self {
            surface: crate::engine::CherenkovSurface::new(engine.clone(), device, backend, size),
            mounts: crate::renderer::retained::Mounts::new(),
            resources: crate::renderer::recording::SceneResources::new(engine),
            device_loss,
        }
    }
}

/// The placement transform of an embedded surface's produced texture: the
/// layer's own transform positions its bounds, then the texture's pixel
/// extent is normalised onto those bounds so engine sampling maps one
/// produced pixel onto one bound area regardless of rounding.
fn gpu_frame_transform(
    transform: kurbo::Affine,
    bounds: kurbo::Rect,
    texture: &wgpu::Texture,
) -> kurbo::Affine {
    let size = texture.size();
    assert!(
        size.width > 0 && size.height > 0,
        "hydrolysis renderer: an external frame plane is an empty texture"
    );
    transform
        * kurbo::Affine::translate((bounds.x0, bounds.y0))
        * kurbo::Affine::scale_non_uniform(
            bounds.width() / f64::from(size.width),
            bounds.height() / f64::from(size.height),
        )
}

/// The clip/opacity ancestry a surface layer is drawn under, as mount
/// scopes: each active scene layer becomes one scope carrying its
/// silhouette transformed into root space and its alpha.
fn ancestry_scopes(
    active_layers: &[ActiveSceneLayer],
) -> Vec<crate::renderer::retained::mount::AncestryScope> {
    active_layers
        .iter()
        .map(|layer| {
            let mut path = match &layer.shape {
                LayerShape::Rect(rect) => rect.to_path(cherenkov::PATH_TOLERANCE),
                LayerShape::RoundedRect { path, .. } | LayerShape::Path(path) => path.clone(),
            };
            path.apply_affine(layer.transform);
            crate::renderer::retained::mount::AncestryScope {
                clip: Some(cherenkov::ShapeData::of(&path)),
                opacity: layer.alpha,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GestureProbe;

    impl waterui_graphics::GpuView for GestureProbe {
        async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut Environment) {}

        fn render(&mut self, _frame: &mut GpuFrame) {}
    }

    #[test]
    fn trackpad_pan_reaches_one_active_frame_before_settling() {
        let mut runtime =
            EmbeddedGpuSurfaceRuntime::new(GpuSurface::new(GestureProbe), &Environment::new());

        assert!(runtime.handle_trackpad_pan(3.0, -2.0, TouchPhase::Started));
        assert!(runtime.handle_trackpad_pan(24.0, -12.0, TouchPhase::Moved));
        assert_eq!(
            runtime.gesture.pan_offset,
            waterui_core::layout::Point::new(27.0, -14.0)
        );
        assert!(runtime.gesture.active);

        assert!(runtime.handle_trackpad_pan(2.0, -1.0, TouchPhase::Ended));
        assert_eq!(
            runtime.gesture.pan_offset,
            waterui_core::layout::Point::new(29.0, -15.0)
        );
        assert!(runtime.gesture.active);
        assert!(runtime.trackpad_pan_ending);

        runtime.finish_trackpad_pan_frame();
        assert!(!runtime.gesture.active);
        assert!(!runtime.trackpad_pan_ending);
    }

    #[test]
    fn embedded_surface_inherits_dynamic_range_metadata() {
        let mut env = Environment::new();
        env.insert(DynamicRangePreference(false));
        let runtime = EmbeddedGpuSurfaceRuntime::new(GpuSurface::new(GestureProbe), &env);
        assert_eq!(runtime.prefers_hdr, Some(false));

        let explicit = EmbeddedGpuSurfaceRuntime::new(
            GpuSurface::new(GestureProbe).prefer_hdr_surface(),
            &env,
        );
        assert_eq!(explicit.prefers_hdr, Some(true));
    }
}

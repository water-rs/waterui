// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use kurbo::Shape;
#[cfg(hydrolysis_macos_system_webview)]
use objc2::rc::Retained;
#[cfg(hydrolysis_macos_system_webview)]
use objc2_web_kit::WKWebView;
use rustc_hash::FxHashSet;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

#[derive(Default)]
pub struct Compositor {
    pub(crate) render_layers: Vec<RenderLayer>,
    pub active_scene_layers: Vec<ActiveSceneLayer>,
}

#[derive(Clone)]
pub enum LayerShape {
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
pub struct ActiveSceneLayer {
    pub(crate) alpha: f32,
    pub(crate) transform: kurbo::Affine,
    pub(crate) shape: LayerShape,
}

/// A `SceneView` leaf presenting this frame: the retained content is
/// re-recorded onto its keyed layer every frame inside
/// [`waterui_graphics::SceneContent::build_scene`].
pub struct SceneContentLayer {
    /// The mount identity: which visual node presents this content.
    pub(crate) key: crate::renderer::retained::RenderKey,
    /// The node-owned content — shared so the compositor can borrow it while
    /// the render tree still owns it.
    pub(crate) content: Rc<RefCell<Box<dyn waterui_graphics::SceneContent>>>,
    /// The semantic invalidator the node installed at build — re-installed
    /// after a `rebuild_for_engine` clears engine-bound hooks.
    pub(crate) invalidator: waterui_graphics::SceneInvalidator,
    /// The engine resource-table identity the content last recorded against:
    /// `None` before its first mount, a `Weak` that expires with the pooled
    /// engine's table. Shared with the owning node's other layer clones.
    pub(crate) association: Rc<RefCell<Option<Weak<crate::renderer::recording::SceneResources>>>>,
    /// Placement transform mapping `bounds` into scene space.
    pub(crate) transform: kurbo::Affine,
    /// The content's rect in scene space; `build_scene` draws inside it.
    pub(crate) bounds: kurbo::Rect,
    /// The clip/opacity ancestry the layer is shown under, captured at flush
    /// (only scopes inside the nearest enclosing filtered group).
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
}

/// A `GpuContentView` leaf presenting this frame: install-once engine content
/// sized per frame on a keyed layer.
pub struct GpuContentLayer {
    /// The mount identity: which visual node presents this content.
    pub(crate) key: crate::renderer::retained::RenderKey,
    /// The node-owned view state — the `GpuContentView` and its one-shot
    /// engine-content install flag.
    pub(crate) runtime: Rc<RefCell<crate::gpu_view::GpuContentRuntime>>,
    /// Placement transform mapping `bounds` into scene space.
    pub(crate) transform: kurbo::Affine,
    /// The content's rect in scene space.
    pub(crate) bounds: kurbo::Rect,
    /// The clip/opacity ancestry the layer is shown under.
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
}

/// An `ExternalFrameView` leaf presenting this frame: a keyed layer that
/// drains the stream's mailbox each pass and hands the newest published
/// frame to the engine as its layer content.
pub struct ExternalFrameLayer {
    /// The mount identity: which visual node presents this content.
    pub(crate) key: crate::renderer::retained::RenderKey,
    /// The node-owned view state — the `ExternalFrameView` and its stream's
    /// frame receiver once the source has been started.
    pub(crate) runtime: Rc<RefCell<crate::gpu_view::ExternalFrameRuntime>>,
    /// Placement transform mapping `bounds` into scene space.
    pub(crate) transform: kurbo::Affine,
    /// The content's rect in scene space.
    pub(crate) bounds: kurbo::Rect,
    /// The clip/opacity ancestry the layer is shown under.
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
}

/// A `FilteredView` wrapper presenting this frame: a keyed layer carrying the
/// registered `Filter`, whose children mount under it as group layers.
pub struct FilteredLayer {
    /// The mount identity: which visual node owns this filter.
    pub(crate) key: crate::renderer::retained::RenderKey,
    /// The node-owned filter runtime — unbuilt source until registration,
    /// then the engine `Filter` handle.
    pub(crate) runtime: Rc<RefCell<crate::renderer::effects::FilteredRuntime>>,
    /// The layers the filtered subtree produced at flush, in bottom-to-top
    /// order; they mount under this layer so the filter covers them all.
    pub(crate) children: Vec<RenderLayer>,
    /// The clip/opacity ancestry the filtered layer itself is shown under.
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
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

pub enum RenderLayer {
    /// Positional recorded content: a contiguous run of scene ops drained by
    /// `flush_scene_layer` shows on the segment layer at that stack position.
    Scene(Recording),
    /// A `SceneView` leaf re-recorded onto its keyed layer every frame.
    SceneContent(SceneContentLayer),
    /// A `GpuContentView` leaf: install-once GPU content on a keyed layer.
    GpuContent(GpuContentLayer),
    /// An `ExternalFrameView` leaf: a keyed layer whose content is the newest
    /// frame the stream's source published, sampled in place.
    ExternalFrame(ExternalFrameLayer),
    /// A `FilteredView` wrapper: a keyed layer carrying a `Filter`, with its
    /// child layers mounted underneath.
    Filtered(FilteredLayer),
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

/// The attachment a scene renders into: the surface handles, the frame's
/// presentation texture (or `None` for a readback-only render) and the
/// colour under the scene's content.
///
/// The render is transient — mounts and the engine surface die with the
/// call — and targets an SDR, single-scale display, the headless contract
/// `waterui-testing` drives.
#[derive(Debug)]
pub struct HydrolysisRenderTarget<'a> {
    /// The adapter the frame's device was requested on; the shared engine is
    /// created against what it can actually run.
    pub adapter: &'a wgpu::Adapter,
    /// The device the surface presents through.
    pub device: &'a wgpu::Device,
    /// The submission queue the surface presents through.
    pub queue: &'a wgpu::Queue,
    /// Reports this device lost; taken when the device was opened. Carries
    /// the device-creation chain the engine pool keys on.
    pub device_loss: crate::platform::DeviceLoss,
    /// The presentation attachment the frame is copied into.
    pub texture: &'a wgpu::Texture,
    /// The attachment's format: `Rgba8`/`Bgra8` unorm or an `Rgba` float
    /// format.
    pub format: wgpu::TextureFormat,
    /// Attachment size in device pixels.
    pub width: u32,
    /// The render target's height in pixels.
    pub height: u32,
    /// The colour under the scene's content.
    pub base_color: waterui_graphics::draw::WorkingColor,
}

/// The full frame description [`HydrolysisRenderer::render_scene_to_texture`]
/// works on: the public target plus the rendering parameters only an internal
/// host sets — the display's scale and HDR headroom, whether the window's
/// mounts and engine surface persist past the call, and the device-creation
/// chain the engine pool keys on.
pub struct FrameRenderTarget<'a> {
    pub adapter: &'a wgpu::Adapter,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub device_loss: crate::platform::DeviceLoss,
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
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub base_color: waterui_graphics::draw::WorkingColor,
}

impl<'a> HydrolysisRenderTarget<'a> {
    /// The transient, SDR, single-scale [`FrameRenderTarget`] a public
    /// [`HydrolysisRenderTarget`] describes, with the device-creation chain
    /// taken from `device_loss`'s context.
    fn into_frame_target(self) -> FrameRenderTarget<'a> {
        let context = self.device_loss.gpu_context();
        FrameRenderTarget {
            adapter: self.adapter,
            device: self.device,
            queue: self.queue,
            device_loss: self.device_loss,
            gpu_context_id: context.context_id,
            shared_device: context.shared_device,
            display_scale: 1.0,
            headroom: 1.0,
            persistent: false,
            format: self.format,
            width: self.width,
            height: self.height,
            base_color: self.base_color,
        }
    }
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

/// Where an install scope's ordered children parent: the surface root, or a
/// filtered mount's content layer.
#[derive(Clone, Copy)]
enum InstallScope {
    /// Children order under the surface root.
    Root,
    /// Children order under the `RenderKey` mount's content layer — the
    /// filtered subtree under its filter.
    Group(crate::renderer::retained::RenderKey),
}

impl InstallScope {
    /// The filtered group's key in group scope, `None` at the root — the
    /// mount scope a layer's [`HeldResources`](waterui_graphics::HeldResources)
    /// stores under.
    const fn parent_key(self) -> Option<crate::renderer::retained::RenderKey> {
        match self {
            Self::Root => None,
            Self::Group(key) => Some(key),
        }
    }
}

/// One `surface.update` pass: mounts the frame's `RenderLayer`s onto the
/// window's engine surface, recording scene content through
/// [`waterui_graphics::SceneContent::build_scene`] and installing `GpuContent`
/// once per view.
///
/// The walker is recursive: a [`RenderLayer::Filtered`]'s children install
/// with `InstallScope::Group`, parenting under the filter's own layer, so a
/// blur or shader covers its whole subtree — not each child separately.
struct FrameInstall<'a> {
    /// The window's engine surface the mounts hang off.
    surface: &'a cherenkov::Surface<cherenkov_gpu::Gpu>,
    /// The window's persistent mount table.
    mounts: &'a mut crate::renderer::retained::Mounts,
    /// The frame's shared engine, for `gpu_producer`/`frame_producer` pairs
    /// and filter registration.
    engine: &'a Rc<crate::engine::GpuEngine>,
    /// The renderer's frame filter telemetry, handed to every `EngineEffect`
    /// this frame registers.
    metrics: &'a std::sync::Arc<crate::renderer::effects::AppliedFilterMetrics>,
    /// The engine's shared resource table — the `build_scene` argument.
    resources: &'a Rc<crate::renderer::recording::SceneResources>,
    /// The host's display-link wake, installed on `GpuContent` producers and
    /// external-frame streams.
    wake: Option<RedrawHandle>,
    /// The frame's device and queue, for starting external-frame sources —
    /// planes are imported on the device the window presents through.
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    /// Whether segment recordings re-install their content this frame.
    rasterize: bool,
    /// Device pixels per logical unit on the target's display.
    display_scale: f64,
    /// Set when a `build_scene` or `view.frame()` asks for another frame.
    needs_redraw: bool,
    /// Every keyed mount presented this frame, at any group depth — the
    /// `sync_order` prune set.
    live_keys: FxHashSet<crate::renderer::retained::RenderKey>,
    /// Content installs this frame, for `recorded_view_contents` accounting.
    installs: u64,
}

impl FrameInstall<'_> {
    /// Installs `layers` in bottom-to-top order under `scope`, returning the
    /// mount slots in the order the scope should commit them.
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    fn install_scope(
        &mut self,
        tx: &mut cherenkov::Transaction<'_, cherenkov_gpu::Gpu>,
        layers: &[RenderLayer],
        scope: InstallScope,
    ) -> Vec<crate::renderer::retained::MountSlot> {
        use crate::renderer::retained::MountSlot;
        let mut order = Vec::with_capacity(layers.len());
        let mut segment_index = 0usize;
        for layer in layers {
            match layer {
                RenderLayer::Scene(recording) => {
                    let slot = MountSlot::Segment(segment_index);
                    segment_index += 1;
                    order.push(slot);
                    let layer = slot_layer(self.mounts, self.surface, scope, slot);
                    if self.rasterize {
                        let resources = self.resources.as_ref();
                        let mut held = None;
                        tx[layer].record(|recorder| {
                            held = Some(recording.record_on(recorder, resources));
                        });
                        self.mounts
                            .set_held(scope.parent_key(), slot, held.expect("record ran"));
                        self.installs += 1;
                    }
                }
                RenderLayer::SceneContent(layer) => {
                    let slot = MountSlot::Keyed(layer.key);
                    order.push(slot);
                    self.live_keys.insert(layer.key);
                    self.mounts.layer(self.surface, slot);
                    let scopes = ancestry_scopes(&layer.active_layers);
                    self.mounts
                        .set_ancestry(self.surface, tx, layer.key, &scopes);
                    let target = slot_layer(self.mounts, self.surface, scope, slot);
                    let content = Rc::clone(&layer.content);
                    // The retained content records only against the engine
                    // table it is associated with: an expired or replaced
                    // table forces `rebuild_for_engine` before any naming;
                    // a freshly constructed content records its identity
                    // without a reset, and the same table re-records freely.
                    let stale = match layer.association.borrow().as_ref() {
                        Some(weak) => match weak.upgrade() {
                            Some(table) => !Rc::ptr_eq(&table, self.resources),
                            None => true,
                        },
                        None => false,
                    };
                    if stale {
                        let mut content = content.borrow_mut();
                        content.rebuild_for_engine();
                        content.set_invalidator(Some(Rc::clone(&layer.invalidator)));
                    }
                    *layer.association.borrow_mut() = Some(Rc::downgrade(self.resources));
                    let mut names = self.resources.waterui().recording();
                    let needs_redraw = &mut self.needs_redraw;
                    #[allow(clippy::cast_possible_truncation)]
                    let (width, height) =
                        (layer.bounds.width() as f32, layer.bounds.height() as f32);
                    tx[target].record(|recorder| {
                        if content
                            .borrow_mut()
                            .build_scene(recorder, &mut names, width, height)
                        {
                            *needs_redraw = true;
                        }
                    });
                    let held = names.finish();
                    tx[target].transform(
                        layer.transform
                            * kurbo::Affine::translate((layer.bounds.x0, layer.bounds.y0)),
                    );
                    self.mounts
                        .set_held(scope.parent_key(), MountSlot::Keyed(layer.key), held);
                    self.installs += 1;
                }
                RenderLayer::GpuContent(layer) => {
                    let slot = MountSlot::Keyed(layer.key);
                    order.push(slot);
                    self.live_keys.insert(layer.key);
                    self.mounts.layer(self.surface, slot);
                    let pixels =
                        gpu_content_pixels(layer.transform, layer.bounds, self.display_scale);
                    let scopes = ancestry_scopes(&layer.active_layers);
                    self.mounts
                        .set_ancestry(self.surface, tx, layer.key, &scopes);
                    let target = slot_layer(self.mounts, self.surface, scope, slot);
                    // A fully transparent ancestry discards every pixel the
                    // content would draw — the producer stays uninstalled
                    // until it can become visible, so hidden GPU work never
                    // runs.
                    let visible = scopes.iter().all(|scope| scope.opacity != 0.0);
                    if visible {
                        let mut runtime = layer.runtime.borrow_mut();
                        if !runtime.installed {
                            let wake = self.wake.clone();
                            let content = runtime.view.take_engine_content(move || {
                                if let Some(wake) = &wake {
                                    wake.request_redraw();
                                }
                            });
                            let producer = self.engine.gpu_producer(content);
                            tx[target].content(producer.at(pixels));
                            runtime.producer = Some(producer);
                            runtime.bound_size = Some(pixels);
                            runtime.installed = true;
                            self.installs += 1;
                        }
                        // The UI-thread pump runs once per presented frame —
                        // producers flush their staged work here before the
                        // engine renders the layer.
                        runtime.view.frame();
                        if runtime.bound_size != Some(pixels) {
                            if let Some(producer) = &runtime.producer {
                                tx[target].content(producer.at(pixels));
                            }
                            runtime.bound_size = Some(pixels);
                        }
                    }
                    tx[target].transform(gpu_frame_transform(
                        layer.transform,
                        layer.bounds,
                        pixels,
                    ));
                }
                RenderLayer::ExternalFrame(layer) => {
                    let slot = MountSlot::Keyed(layer.key);
                    order.push(slot);
                    self.live_keys.insert(layer.key);
                    self.mounts.layer(self.surface, slot);
                    let scopes = ancestry_scopes(&layer.active_layers);
                    self.mounts
                        .set_ancestry(self.surface, tx, layer.key, &scopes);
                    let target = slot_layer(self.mounts, self.surface, scope, slot);
                    let visible = scopes.iter().all(|scope| scope.opacity != 0.0);
                    if visible {
                        let mut runtime = layer.runtime.borrow_mut();
                        if runtime.receiver.is_none() {
                            let redraw = self
                                .wake
                                .clone()
                                .unwrap_or_else(|| RedrawHandle::new(|| {}));
                            let (producer, sink) = self.engine.frame_producer();
                            runtime.producer = Some(producer);
                            runtime.sink = Some(sink);
                            runtime.receiver =
                                Some(runtime.view.stream().start(self.device, self.queue, redraw));
                            self.installs += 1;
                        }
                        // The mailbox keeps only the newest published frame:
                        // drain it here so one engine pass presents at most
                        // one frame, sampled in place with no copy.
                        if let Some(frame) = runtime
                            .receiver
                            .as_ref()
                            .and_then(waterui_graphics::gpu::FrameReceiver::take)
                        {
                            let pixels = external_frame_plane_size(&frame);
                            runtime.frame_pixels = Some(pixels);
                            if let Some(sink) = &runtime.sink {
                                sink.submit(frame);
                            }
                            if runtime.bound_size != Some(pixels) {
                                if let Some(producer) = &runtime.producer {
                                    tx[target].content(producer.at(pixels));
                                }
                                runtime.bound_size = Some(pixels);
                            }
                        }
                        if let Some(pixels) = runtime.frame_pixels {
                            tx[target].transform(gpu_frame_transform(
                                layer.transform,
                                layer.bounds,
                                pixels,
                            ));
                        }
                    }
                }
                RenderLayer::Filtered(layer) => {
                    let slot = MountSlot::Keyed(layer.key);
                    order.push(slot);
                    self.live_keys.insert(layer.key);
                    self.mounts.layer(self.surface, slot);
                    let scopes = ancestry_scopes(&layer.active_layers);
                    self.mounts
                        .set_ancestry(self.surface, tx, layer.key, &scopes);
                    {
                        let filter = layer
                            .runtime
                            .borrow_mut()
                            .filter(self.engine, self.metrics)
                            .clone();
                        let target = slot_layer(self.mounts, self.surface, scope, slot);
                        tx[target].filter(&filter);
                    }
                    let group_order =
                        self.install_scope(tx, &layer.children, InstallScope::Group(layer.key));
                    self.mounts.sync_group_order(tx, layer.key, &group_order);
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
        order
    }
}

/// The engine layer `slot` mounts on under `scope`: a segment under the root
/// is a shared content layer; under a filtered group it is one of the group's
/// own segment layers.
fn slot_layer<'a>(
    mounts: &'a mut crate::renderer::retained::Mounts,
    surface: &'a cherenkov::Surface<cherenkov_gpu::Gpu>,
    scope: InstallScope,
    slot: crate::renderer::retained::MountSlot,
) -> &'a cherenkov::Layer {
    use crate::renderer::retained::MountSlot;
    match (scope, slot) {
        (InstallScope::Group(parent), MountSlot::Segment(index)) => {
            mounts.group_layer(surface, parent, index)
        }
        (InstallScope::Group(parent), slot) => mounts.ordered_in_group(parent, slot),
        (InstallScope::Root, slot) => mounts.layer(surface, slot),
    }
}

/// The pixel size a `GpuContentView` renders at: its bounds under its
/// transform, scaled to device pixels.
fn gpu_content_pixels(
    transform: kurbo::Affine,
    bounds: kurbo::Rect,
    display_scale: f64,
) -> (u32, u32) {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let x_scale = a.hypot(b) * display_scale;
    let y_scale = c.hypot(d) * display_scale;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let size = (
        (bounds.width() * x_scale).round().max(1.0) as u32,
        (bounds.height() * y_scale).round().max(1.0) as u32,
    );
    size
}

impl HydrolysisRenderer {
    crate::engine::cfg_async_fn! {
        /// Renders the frame into `target`'s texture.
        ///
        /// Async on wasm32, where the surface render inside awaits the browser
        /// device.
        pub fn render_scene_to_texture(&mut self, target: HydrolysisRenderTarget<'_>) {
            let texture = target.texture;
            crate::engine::engine_await!(
                self.render_scene_to_surface_with_alpha_mode(
                    target.into_frame_target(), texture, false, true
                )
            );
        }
    }

    crate::engine::cfg_async_fn! {
        /// Renders the frame into `target`'s presentation surface.
        ///
        /// Async on wasm32, where the surface render inside awaits the browser
        /// device.
        pub fn render_scene_to_surface(&mut self, target: HydrolysisRenderTarget<'_>) {
            let texture = target.texture;
            crate::engine::engine_await!(
                self.render_scene_to_surface_with_alpha_mode(
                    target.into_frame_target(), texture, false, true
                )
            );
        }
    }

    /// Splits the frame's render layers around `NativeView` layers into the
    /// segments the hybrid compositor renders one surface at a time, taking
    /// the transient scene the last segment draws.
    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn take_hybrid_composition(&mut self) -> Option<HybridComposition> {
        self.flush_scene_layer();
        if !self
            .compositor
            .render_layers
            .iter()
            .any(|layer| matches!(layer, RenderLayer::NativeView(_)))
        {
            return None;
        }

        let mut segments = vec![HybridRenderSegment { layers: Vec::new() }];
        let mut native_views = Vec::new();
        for layer in core::mem::take(&mut self.compositor.render_layers) {
            match layer {
                RenderLayer::NativeView(layer) => {
                    native_views.push(layer);
                    segments.push(HybridRenderSegment { layers: Vec::new() });
                }
                layer => segments
                    .last_mut()
                    .expect("Hydrolysis hybrid composition must have a render segment")
                    .layers
                    .push(layer),
            }
        }
        assert!(
            segments.len() == native_views.len() + 1,
            "Hydrolysis hybrid composition segment count must bracket every native view"
        );
        Some(HybridComposition {
            segments,
            native_views,
            transient_scene: self.transient_scene.take(),
        })
    }

    /// Renders one hybrid segment through the engine for its own surface:
    /// the segment's layers become the frame's render layers for the call and
    /// return to the segment afterwards. The caller presents the returned
    /// frame.
    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn render_hybrid_segment(
        &mut self,
        segment: &mut HybridRenderSegment,
        transient_scene: Option<Recording>,
        target: FrameRenderTarget<'_>,
    ) -> EngineFrame {
        assert!(
            self.compositor.render_layers.is_empty(),
            "Hydrolysis hybrid composition cannot render over retained layers"
        );
        assert!(
            self.transient_scene.is_none(),
            "Hydrolysis hybrid composition cannot replace a transient scene"
        );
        self.compositor.render_layers = core::mem::take(&mut segment.layers);
        self.transient_scene = transient_scene;
        let frame = self.render_engine_frame(target, true);
        segment.layers = core::mem::take(&mut self.compositor.render_layers);
        assert!(
            self.transient_scene.is_none(),
            "Hydrolysis hybrid segment left a transient scene unconsumed"
        );
        frame
    }

    /// Reassembles the render layers a hybrid composition split, checking the
    /// rendering left the structure it took apart.
    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn restore_hybrid_composition(&mut self, composition: HybridComposition) {
        let HybridComposition {
            segments,
            native_views,
            transient_scene,
        } = composition;
        assert!(
            transient_scene.is_none(),
            "Hydrolysis hybrid composition restored before rendering its transient scene"
        );
        assert!(
            segments.len() == native_views.len() + 1,
            "Hydrolysis hybrid composition segment count changed during rendering"
        );
        let segment_count = segments.len();
        let mut native_views = native_views.into_iter();
        let mut layers = Vec::new();
        for (index, segment) in segments.into_iter().enumerate() {
            layers.extend(segment.layers);
            if index + 1 < segment_count
                && let Some(native_view) = native_views.next()
            {
                layers.push(RenderLayer::NativeView(native_view));
            }
        }
        assert!(
            native_views.next().is_none(),
            "Hydrolysis hybrid composition kept a native view unrendered"
        );
        self.compositor.render_layers = layers;
    }

    crate::engine::cfg_async_fn! {
        /// [`Self::render_scene_to_surface`] with the target's composite alpha
        /// convention made explicit: `premultiply_alpha` selects the alpha
        /// mode the presenter writes into `texture` — premultiplied for an OS
        /// surface configured `CompositeAlphaMode::PreMultiplied`, straight
        /// for offscreen/readback targets.
        ///
        /// `rasterize_scene_layers` is [`Self::render_engine_frame`]'s.
        ///
        /// Async on wasm32, where the engine calls inside await the browser
        /// device.
        pub(crate) fn render_scene_to_surface_with_alpha_mode(
            &mut self,
            target: FrameRenderTarget<'_>,
            texture: &wgpu::Texture,
            premultiply_alpha: bool,
            rasterize_scene_layers: bool,
        ) {
            let (device, queue) = (target.device, target.queue);
            let frame = crate::engine::engine_await!(
                self.render_engine_frame(target, rasterize_scene_layers)
            );
            self.present_engine_frame(
                frame,
                device,
                queue,
                texture,
                crate::engine::format_output_color(texture.format()),
                premultiply_alpha,
            );
        }
    }

    /// Copies the frame [`Self::render_engine_frame`] rendered into
    /// `texture`. Synchronous on every target, so a host presenting a
    /// swapchain image acquires it only after the engine render, and the
    /// image is never held across an await: a browser expires its canvas
    /// texture when the task that acquired it ends.
    pub(crate) fn present_engine_frame(
        &mut self,
        frame: EngineFrame,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        color: cherenkov_gpu::interop::OutputColor,
        premultiply_alpha: bool,
    ) {
        // The window map leaves `self` for the call, as in the render, so the
        // profiler mark may borrow the renderer.
        let mut windows = core::mem::take(&mut self.cherenkov_windows);
        let mut transient = frame.transient;
        let window = match &mut transient {
            Some(window) => window,
            None => windows.get_mut(&frame.context_id).expect(
                "hydrolysis renderer: the engine frame's window left the renderer before its present",
            ),
        };
        window.surface.present_into(
            device,
            queue,
            texture,
            color,
            premultiply_alpha,
            frame.headroom,
        );

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), device, queue, 2);

        self.cherenkov_windows = windows;
    }

    crate::engine::cfg_async_fn! {
        // one continuous frame-build sequence; splitting it would only mirror the pipeline stages artificially
        #[allow(clippy::too_many_lines)]
        /// Installs the frame's layers into the engine surface for
        /// `target`'s GPU context and renders it into the engine's retained
        /// output. [`Self::present_engine_frame`] then copies that output into
        /// the presentation attachment.
        ///
        /// `rasterize_scene_layers` skips only re-installing segment content:
        /// a frame whose pixels no consumer can read still mounts every layer
        /// and ticks `GpuContentView` frame hooks, but leaves each segment
        /// showing the content it already carries. Capture and presented
        /// frames always pass `true`.
        ///
        /// Async on wasm32, where the engine calls inside await the browser
        /// device.
        pub(crate) fn render_engine_frame(
            &mut self,
            target: FrameRenderTarget<'_>,
            rasterize_scene_layers: bool,
        ) -> EngineFrame {
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

        let render_layers = core::mem::take(&mut self.compositor.render_layers);
        let transient = self.transient_scene.take().filter(scene_has_content);

        // The shared engine for this frame's device context; `wake` is the
        // host's display-link wake the engine's redraw callback drives.
        let host_wake = self.host_redraw_handle.clone();
        let engine_wake = host_wake.clone();
        let state = crate::engine::engine_await!(crate::engine::shared_engine_state(
            target.gpu_context_id,
            target.adapter,
            target.shared_device.clone(),
            move || {
                if let Some(handle) = &engine_wake {
                    handle.request_redraw();
                }
            },
        ));

        // One window surface per GPU context this renderer presents through.
        // Entries whose device was reported lost are dropped — their engine
        // surface, mounts, resources and profiler all die with the dead
        // device — and a context change mid-session replaces the previous
        // window's mount state with a fresh one. The map leaves `self` for
        // the frame so the layer walk may borrow the renderer's other state.
        let mut windows = core::mem::take(&mut self.cherenkov_windows);
        windows.retain(|_, window| !window.device_loss.is_lost());
        let backend = target.adapter.get_info().backend;
        let context_id = target.gpu_context_id;
        // A persistent output keeps its window (and mounts) across frames; a
        // transient target's window lives for this call only — its mounts and
        // engine surface are created fresh and dropped with the capture.
        let mut transient_window = None;
        let window = if target.persistent {
            match windows.entry(context_id) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                    crate::engine::engine_await!(CherenkovWindow::new(
                        Rc::clone(&state),
                        target.device,
                        backend,
                        (target.width, target.height),
                        target.device_loss.clone(),
                    )),
                ),
            }
        } else {
            transient_window.insert(crate::engine::engine_await!(CherenkovWindow::new(
                Rc::clone(&state),
                target.device,
                backend,
                (target.width, target.height),
                target.device_loss.clone(),
            )))
        };

        // The first marker lands after context resolution because the
        // profiler is device-owned like the window it rides on: a replaced
        // device writes into its own query set, not the dead context's.
        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 0);

        window.surface.resize((target.width, target.height));
        window
            .surface
            .display(target.display_scale, target.headroom);
        window.surface.clear_color(target.base_color);

        // The frame's layer edits apply inside one surface update: keyed
        // mounts install or refresh their content, re-recorded content draws
        // through `build_scene`, and filtered mounts parent their children
        // under the filter's layer.
        let mut install = FrameInstall {
            surface: window.surface.engine_surface(),
            mounts: &mut window.mounts,
            engine: &state.engine,
            metrics: &self.applied_filter_metrics,
            resources: &window.state.resources,
            wake: host_wake,
            device: target.device,
            queue: target.queue,
            rasterize: rasterize_scene_layers,
            display_scale: target.display_scale,
            needs_redraw: false,
            live_keys: FxHashSet::default(),
            installs: 0,
        };
        install.surface.update(|tx| {
            let mut order = install.install_scope(tx, &render_layers, InstallScope::Root);
            if let Some(recording) = &transient {
                order.push(crate::renderer::retained::MountSlot::Overlay);
                let overlay = install.mounts.layer(
                    install.surface,
                    crate::renderer::retained::MountSlot::Overlay,
                );
                let mut held = None;
                tx[overlay].record(|recorder| {
                    held = Some(recording.record_on(recorder, install.resources.as_ref()));
                });
                install.mounts.set_held(
                    None,
                    crate::renderer::retained::MountSlot::Overlay,
                    held.expect("record ran"),
                );
            }
            install
                .mounts
                .sync_order(install.surface, tx, &order, &install.live_keys);
        });
        let needs_redraw = install.needs_redraw;
        let installs = install.installs;
        drop(install);
        let window = transient_window.as_mut().unwrap_or_else(|| {
            windows
                .get_mut(&context_id)
                .expect("hydrolysis renderer: window surface lost within a frame")
        });
        self.state.counters.recorded_view_contents += installs;
        let (created, removed) = window.mounts.take_frame_stats();
        self.state.counters.layer_creations += created;
        self.state.counters.layer_removals += removed;
        let (fonts, images) = window.state.resources.take_registration_stats();
        self.state.counters.font_registrations += fonts;
        self.state.counters.image_registrations += images;

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 1);

        self.applied_filter_metrics.reset();
        let next = crate::engine::engine_await!(window.surface.render());
        (
            self.frame_applied_filter_count,
            self.frame_applied_filter_effect,
        ) = self.applied_filter_metrics.snapshot();
        self.engine_next = Some(next);

        self.compositor.render_layers = render_layers;
        self.cherenkov_windows = windows;
        if needs_redraw {
            self.request_redraw();
        }
        EngineFrame {
            context_id,
            headroom: target.headroom,
            transient: transient_window,
        }
    }
    }
}

/// A frame [`HydrolysisRenderer::render_engine_frame`] rendered and
/// [`HydrolysisRenderer::present_engine_frame`] has yet to copy out: the GPU
/// context whose window holds the output, and the window itself when the
/// target was transient and owns no persistent entry.
#[must_use = "an engine frame is shown only once present_engine_frame copies it out"]
pub struct EngineFrame {
    context_id: u64,
    headroom: f32,
    transient: Option<CherenkovWindow>,
}

/// One window surface's engine-side state: the `TextureTarget` surface, the
/// stable mounts the frame's `RenderLayer`s show through, and the resource
/// registrations recorded content names.
pub struct CherenkovWindow {
    pub surface: crate::engine::CherenkovSurface,
    pub(crate) mounts: crate::renderer::retained::Mounts,
    /// The pooled engine state — its `Rc<SceneResources>` is the ONE
    /// registration table every mount on this context's engine shares;
    /// keeping the state here pins that table for the window's lifetime.
    pub(crate) state: Rc<crate::engine::SharedEngineState>,
    /// The device-loss token taken when this window's context was opened; a
    /// dead token prunes the entry so a recovered context gets a fresh mount
    /// set instead of reusing a surface on a dead device.
    device_loss: crate::platform::DeviceLoss,
    /// Timestamp-query state for the frame's GPU spans on this context's
    /// device; `None` when the device lacks `TIMESTAMP_QUERY` — GPU stages
    /// then report absent, never a guess. Dies with the window, so device
    /// replacement rebuilds it on the new device instead of leaving a query
    /// set registered on the dead one.
    #[cfg(feature = "frame-profile")]
    pub(crate) gpu_profiler: Option<GpuFrameProfiler>,
}

crate::engine::cfg_async_fn! {
    impl CherenkovWindow {
        pub(crate) fn new(
            state: Rc<crate::engine::SharedEngineState>,
            device: &wgpu::Device,
            backend: wgpu::Backend,
            size: (u32, u32),
            device_loss: crate::platform::DeviceLoss,
        ) -> Self {
            Self {
                surface: crate::engine::engine_await!(crate::engine::CherenkovSurface::new(
                    Rc::clone(&state.engine),
                    device,
                    backend,
                    size,
                )),
                mounts: crate::renderer::retained::Mounts::new(),
                state,
                device_loss,
                #[cfg(feature = "frame-profile")]
                gpu_profiler: GpuFrameProfiler::new(device),
            }
        }
    }
}

/// The placement transform of a `GpuContentView`'s produced texture: the
/// layer's own transform positions its bounds, then the produced pixel extent
/// is normalised onto those bounds so engine sampling maps one produced pixel
/// onto one bound area regardless of rounding.
fn gpu_frame_transform(
    transform: kurbo::Affine,
    bounds: kurbo::Rect,
    pixels: (u32, u32),
) -> kurbo::Affine {
    transform
        * kurbo::Affine::translate((bounds.x0, bounds.y0))
        * kurbo::Affine::scale_non_uniform(
            bounds.width() / f64::from(pixels.0),
            bounds.height() / f64::from(pixels.1),
        )
}

/// A frame's plane size in pixels: the luma plane for YUV, the plane for RGB
/// — the size the engine emits the frame's quad at in layer space.
fn external_frame_plane_size(frame: &cherenkov_gpu::interop::ExternalFrame) -> (u32, u32) {
    use cherenkov_gpu::interop::FramePlanes;
    match &frame.planes {
        FramePlanes::Yuv { y, .. } => (y.width(), y.height()),
        FramePlanes::Rgb { plane, .. } => (plane.width(), plane.height()),
        #[cfg(all(unix, not(target_vendor = "apple")))]
        FramePlanes::Native(frame) => frame.size(),
    }
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
                LayerShape::Rect(rect) => rect.to_path(waterui_graphics::draw::PATH_TOLERANCE),
                LayerShape::RoundedRect { path, .. } | LayerShape::Path(path) => path.clone(),
            };
            path.apply_affine(layer.transform);
            crate::renderer::retained::mount::AncestryScope {
                clip: Some(waterui_graphics::draw::ShapeData::of(&path)),
                opacity: layer.alpha,
            }
        })
        .collect()
}

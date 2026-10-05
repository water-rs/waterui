//! A test backend: [`Null`] draws nothing and reports every render-thread
//! call as an [`Event`] on a channel, so tests and the cross-backend
//! behaviour suite can assert what the front end committed.

/// The committed layer op a [`SurfaceTree`](crate::SurfaceTree) applies, for
/// tests that build a sampled tree without an engine.
pub use cherenkov_record::LayerOp;
use std::collections::HashSet;
use std::sync::mpsc::Sender;

use rustc_hash::FxHashMap;

use kurbo::{Affine, Vec2};

use crate::backend::{Backend, Display, Frame, FrameRedraw, Renderer, SurfaceInfo, Visibility};
use crate::capability::{ShaderPaint, ShaderSource};
use crate::config::MemoryUsage;
use crate::error::{EngineError, RenderError, ResourceError, SurfaceError};
use crate::frame::Readback;
use crate::glyph::FontId;
use crate::image::{ImageUpload, Rgba8, Rgba16F};
use cherenkov_record::{ContentOp, LayerId, ResourceId, SurfaceId};

use crate::message::FontData;
use crate::paint::{ImageId, ShaderId};
use crate::{Offscreen, Picture, Pressure, Uploads};

/// A render-thread event [`Null`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub enum Event {
    /// `create_surface` ran.
    CreateSurface(SurfaceId),
    /// `resize_surface` ran.
    ResizeSurface(SurfaceId, (u32, u32)),
    /// `destroy_surface` ran.
    DestroySurface(SurfaceId),
    /// `add_font` ran.
    AddFont(FontId),
    /// `remove_font` ran.
    RemoveFont(FontId),
    /// `add_image` ran.
    AddImage(ImageId),
    /// `replace_image` ran, with the new dimensions.
    ReplaceImage(ImageId, (u32, u32)),
    /// `remove_image` ran.
    RemoveImage(ImageId),
    /// `add_shader` ran.
    AddShader(ShaderId),
    /// `remove_shader` ran.
    RemoveShader(ShaderId),
    /// `set_content` ran, with the stored picture's address — `0` for
    /// an update or a clear, which store no new picture.
    SetContent(SurfaceId, LayerId, usize),
    /// `set_external_frame`-equivalent: a submitted frame landed on a
    /// bound layer.
    ProducerFrame(SurfaceId, LayerId),
    /// `add_gpu_producer` or `add_frame_producer` ran.
    AddProducer(crate::ProducerId),
    /// `bind_gpu_producer` ran, with the binding's producer.
    BindProducer(SurfaceId, LayerId, crate::ProducerId),
    /// `retire_gpu_producer` ran.
    RetireProducer(crate::ProducerId),
    /// `drain_gpu_producers` ran.
    DrainProducers,
    /// `remove_layer` ran.
    RemoveLayer(SurfaceId, LayerId),
    /// `set_visibility` ran.
    Visibility(SurfaceId, Visibility),
    /// One rendered surface, in `frame.surfaces` order.
    Frame(FrameRecord),
}

/// One surface's sampled tree, recorded per rendered frame.
#[derive(Debug)]
pub struct FrameRecord {
    /// The surface.
    pub surface: SurfaceId,
    /// The frame's `changed` flag for this surface.
    pub changed: bool,
    /// The frame's `plane_frames` for this surface — the layers that got
    /// a new external frame when those installs were the only change
    /// (#90), by layer id.
    pub plane_frames: Option<Vec<LayerId>>,
    /// The frame's `present_pending` flag for this surface: a display
    /// change re-presents without touching content (#98).
    pub present_pending: bool,
    /// The frame's `display_moved` flag for this surface: the host's
    /// display-move announcement, which re-enumerates output
    /// negotiation (#98).
    pub display_moved: bool,
    /// The display state the frame presented with.
    pub display: Display,
    /// Every layer's sampled state.
    pub layers: Vec<LayerSample>,
}

/// One layer's sampled state in a [`FrameRecord`].
#[derive(Debug)]
pub struct LayerSample {
    /// The layer.
    pub id: LayerId,
    /// The sampled transform.
    pub transform: Affine,
    /// The sampled opacity.
    pub opacity: f32,
    /// The sampled, pixel-snapped scroll offset.
    pub scroll_offset: Vec2,
    /// The layer's children, in paint order.
    pub children: Vec<LayerId>,
}

/// A surface target for the [`Null`] backend.
///
/// An [`Offscreen`] buffer or a window-like presenting target, so
/// presentation semantics — a `Display` update marking `present_pending`
/// — are testable without a real window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NullTarget {
    /// An offscreen buffer; `Display` updates never mark presentation.
    Offscreen(Offscreen),
    /// A window-like target: it reports presenting, so `Display` updates
    /// mark `SurfaceFrame::present_pending` exactly as a real swapchain
    /// target does.
    Window(Offscreen),
}

impl From<Offscreen> for NullTarget {
    fn from(target: Offscreen) -> Self {
        Self::Offscreen(target)
    }
}

/// A backend that draws nothing and reports every call. The `Config`
/// carries the test's probe channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Null;

/// Configuration for [`Null`]: the channel events are reported on.
#[derive(Debug)]
pub struct NullConfig {
    /// The event probe. A test keeps the matching `Receiver`.
    pub events: Sender<Event>,
    /// Registration kinds the backend refuses: the matching `add_*` or
    /// `create_surface` call returns its error without committing.
    pub reject: HashSet<NullReject>,
    /// The limits the backend reports and [`Engine::image_limits`] reads:
    /// registrations they do not admit fail on the calling thread with
    /// [`ResourceError::TooLarge`].
    pub image_limits: crate::ImageLimits,
}

/// A registration [`Null`] refuses, for failure-path tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NullReject {
    /// `create_surface` returns [`SurfaceError::UnsupportedTarget`].
    Surface,
    /// `add_image` returns [`ResourceError::Image`].
    Image,
    /// `add_shader` returns [`ResourceError::Shader`].
    Shader,
}

/// `Null`'s provenance: nothing to report.
pub type NullInfo = ();

/// The `Null` render-thread state, with strict removal.
///
/// A `remove_*` or `destroy_surface` for an id the backend does not hold
/// panics on the render thread, so a spurious removal fails the test that
/// caused it instead of passing as a no-op. So does a render while
/// installed content draws a resource the backend already removed.
#[derive(Debug)]
pub struct NullRenderer {
    events: Sender<Event>,
    reject: HashSet<NullReject>,
    image_limits: crate::ImageLimits,
    surfaces: HashSet<SurfaceId>,
    /// The surfaces the host announced hidden.
    hidden: HashSet<SurfaceId>,
    fonts: HashSet<FontId>,
    images: HashSet<ImageId>,
    shaders: HashSet<ShaderId>,
    pictures: FxHashMap<(SurfaceId, LayerId), Picture>,
    /// Every resource removed so far; ids are never reused.
    removed: HashSet<ResourceId>,
    /// The producers, so `submit_frame` names their bound layers.
    producers: FxHashMap<crate::ProducerId, NullProducer>,
    /// Retired before its registration reached the stream — the
    /// producer's own retire queue can beat its add; the add skips an
    /// id found here.
    pending_retire: HashSet<crate::ProducerId>,
}

/// A [`Null`] producer: rendered (`content` `Some`) or submitted-frame.
#[derive(Debug)]
struct NullProducer {
    content: Option<()>,
    /// The submitted frame's marker: `Some` once one landed, so a later
    /// bind reports its declared alpha like the GPU backend's slot.
    current: Option<()>,
    /// The bindings hold a [`GpuProducer`](crate::GpuProducer) clone
    /// like the GPU backend's `Binding` — a binding's unbind drops the
    /// clone on the render thread, so the last reference can die
    /// inside the transaction stream.
    bindings: FxHashMap<(SurfaceId, LayerId), crate::GpuProducer<Null>>,
}

impl NullRenderer {
    /// Releases the binding every producer holds on `(surface, layer)`,
    /// like `set_content` and `remove_layer` dropping the layer's other
    /// content.
    fn unbind(&mut self, surface: SurfaceId, layer: LayerId) {
        for producer in self.producers.values_mut() {
            producer.bindings.remove(&(surface, layer));
        }
    }

    fn new(config: NullConfig) -> Self {
        Self {
            events: config.events,
            reject: config.reject,
            image_limits: config.image_limits,
            surfaces: HashSet::new(),
            hidden: HashSet::new(),
            fonts: HashSet::new(),
            images: HashSet::new(),
            shaders: HashSet::new(),
            pictures: FxHashMap::default(),
            removed: HashSet::new(),
            producers: FxHashMap::default(),
            pending_retire: HashSet::new(),
        }
    }

    /// Panics when a frame lists a hidden surface, or when a surface's
    /// installed content draws a removed resource: the render loop leaves
    /// hidden surfaces out of every frame, and frees a resource only once
    /// no installed content draws it.
    fn assert_frame_contract(&self, frame: &Frame<'_>) {
        for surface in frame.surfaces {
            assert!(
                !self.hidden.contains(&surface.id),
                "frame lists hidden surface {}",
                surface.id.raw()
            );
            for resource in &self.removed {
                assert!(
                    !self.samples(surface.id, *resource),
                    "surface {} draws {resource} after its removal",
                    surface.id.raw()
                );
            }
        }
    }
}

impl cherenkov_record::Target for Null {
    type Queue = crate::EngineQueue<Self>;
    type Install = crate::InstallOp<Self>;
}

impl cherenkov_record::GpuInstalls for Null {}

impl Backend for Null {
    type Config = NullConfig;
    type Info = NullInfo;
    type Target = NullTarget;
    type Renderer = NullRenderer;

    #[cfg(not(target_arch = "wasm32"))]
    fn init(config: NullConfig) -> Result<(NullRenderer, NullInfo), EngineError> {
        Ok((NullRenderer::new(config), ()))
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(
        clippy::future_not_send,
        reason = "NullRenderer is single-threaded on wasm — its producers' bindings hold GpuProducer clones on the page event loop"
    )]
    fn init(
        config: NullConfig,
    ) -> impl core::future::Future<Output = Result<(NullRenderer, NullInfo), EngineError>> {
        core::future::ready(Ok((NullRenderer::new(config), ())))
    }
}

impl Renderer for NullRenderer {
    type Target = NullTarget;
    type Font = FontData;

    fn create_surface(
        &mut self,
        id: SurfaceId,
        target: NullTarget,
        _waker: crate::CompletionWaker,
    ) -> Result<SurfaceInfo, SurfaceError> {
        let (target, presents) = match &target {
            NullTarget::Offscreen(target) => (target, false),
            NullTarget::Window(target) => (target, true),
        };
        if target.size.0 == 0 || target.size.1 == 0 {
            return Err(SurfaceError::ZeroSize);
        }
        if self.reject.contains(&NullReject::Surface) {
            return Err(SurfaceError::UnsupportedTarget("injected rejection".into()));
        }
        self.surfaces.insert(id);
        let _ = self.events.send(Event::CreateSurface(id));
        Ok(SurfaceInfo {
            max_dimension: u32::MAX,
            size: target.size,
            readable: true,
            presents,
        })
    }

    fn resize_surface(&mut self, id: SurfaceId, size: (u32, u32)) {
        let _ = self.events.send(Event::ResizeSurface(id, size));
    }

    fn set_visibility(&mut self, id: SurfaceId, visibility: Visibility) {
        let changed = match visibility {
            Visibility::Hidden => self.hidden.insert(id),
            Visibility::Visible => self.hidden.remove(&id),
        };
        assert!(
            changed,
            "surface {} announced {visibility:?} twice",
            id.raw()
        );
        let _ = self.events.send(Event::Visibility(id, visibility));
    }

    fn destroy_surface(&mut self, id: SurfaceId) {
        assert!(
            self.surfaces.remove(&id),
            "destruction of unknown surface {}",
            id.raw()
        );
        self.pictures.retain(|(surface, _), _| *surface != id);
        self.hidden.remove(&id);
        let _ = self.events.send(Event::DestroySurface(id));
    }

    /// `Null` draws no glyphs, so any data is a font.
    fn prepare_font(font: FontData) -> Result<FontData, ResourceError> {
        Ok(font)
    }

    fn add_font(&mut self, id: FontId, _font: FontData) {
        self.fonts.insert(id);
        let _ = self.events.send(Event::AddFont(id));
    }

    fn remove_font(&mut self, id: FontId) {
        assert!(
            self.fonts.remove(&id),
            "removal of unregistered font {}",
            id.raw()
        );
        self.removed.insert(ResourceId::Font(id));
        let _ = self.events.send(Event::RemoveFont(id));
    }

    fn image_limits(&self) -> crate::ImageLimits {
        self.image_limits
    }

    fn add_image(&mut self, id: ImageId, _image: ImageUpload) -> Result<(), ResourceError> {
        if self.reject.contains(&NullReject::Image) {
            return Err(ResourceError::Image("injected rejection".into()));
        }
        self.images.insert(id);
        let _ = self.events.send(Event::AddImage(id));
        Ok(())
    }

    fn replace_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        assert!(
            self.images.contains(&id),
            "replace of unregistered image {}",
            id.raw()
        );
        let _ = self
            .events
            .send(Event::ReplaceImage(id, (image.width, image.height)));
        Ok(())
    }

    fn samples(&self, surface: SurfaceId, resource: ResourceId) -> bool {
        self.pictures.iter().any(|((owner, _), picture)| {
            *owner == surface && picture.display_list().references(resource)
        })
    }

    fn remove_image(&mut self, id: ImageId) {
        assert!(
            self.images.remove(&id),
            "removal of unregistered image {}",
            id.raw()
        );
        self.removed.insert(ResourceId::Image(id));
        let _ = self.events.send(Event::RemoveImage(id));
    }

    fn set_content(
        &mut self,
        surface: SurfaceId,
        layer: LayerId,
        content: Option<ContentOp>,
    ) -> Option<Picture> {
        let token = match &content {
            Some(ContentOp::Replace(picture) | ContentOp::Picture(picture)) => {
                std::ptr::from_ref(picture.display_list()) as usize
            }
            _ => 0,
        };
        let previous = match content {
            Some(ContentOp::Replace(picture) | ContentOp::Picture(picture)) => {
                self.pictures.insert((surface, layer), picture)
            }
            Some(ContentOp::Update(updates)) => {
                let _ = self
                    .pictures
                    .get_mut(&(surface, layer))
                    .expect("slot update targets a layer without content")
                    .apply(updates);
                None
            }
            None => self.pictures.remove(&(surface, layer)),
        };
        self.unbind(surface, layer);
        let _ = self.events.send(Event::SetContent(surface, layer, token));
        previous
    }

    fn remove_layer(&mut self, surface: SurfaceId, layer: LayerId) {
        self.pictures.remove(&(surface, layer));
        self.unbind(surface, layer);
        let _ = self.events.send(Event::RemoveLayer(surface, layer));
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        _stats: &mut crate::FrameStats,
    ) -> Result<FrameRedraw, RenderError> {
        self.assert_frame_contract(frame);
        for surface in frame.surfaces {
            let layers = surface
                .tree
                .layers()
                .map(|(id, node)| LayerSample {
                    id,
                    transform: node.transform,
                    opacity: node.opacity,
                    scroll_offset: node.scroll_offset,
                    children: node.children.clone(),
                })
                .collect();
            let _ = self.events.send(Event::Frame(FrameRecord {
                surface: surface.id,
                changed: surface.changed,
                plane_frames: surface.plane_frames.map(|frames| {
                    let mut frames: Vec<_> = frames.iter().copied().collect();
                    frames.sort_by_key(|layer| layer.raw());
                    frames
                }),
                present_pending: surface.present_pending,
                display_moved: surface.display_moved,
                display: surface.display,
                layers,
            }));
        }
        Ok(FrameRedraw::default())
    }

    /// The wasm render path shares the CPU-side contract of returning the
    /// owned `FrameRedraw`, whose request storage is `Rc` on this target —
    /// the engine is single-threaded on the page, so the future is
    /// deliberately not `Send`, matching `NullRenderer::init` and the
    /// browser engine's own futures.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the engine is single-threaded on wasm — FrameRedraw's shared request storage is Rc there"
    )]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        _stats: &mut crate::FrameStats,
    ) -> impl core::future::Future<Output = Result<FrameRedraw, RenderError>> {
        self.assert_frame_contract(frame);
        for surface in frame.surfaces {
            let layers = surface
                .tree
                .layers()
                .map(|(id, node)| LayerSample {
                    id,
                    transform: node.transform,
                    opacity: node.opacity,
                    scroll_offset: node.scroll_offset,
                    children: node.children.clone(),
                })
                .collect();
            let _ = self.events.send(Event::Frame(FrameRecord {
                surface: surface.id,
                changed: surface.changed,
                plane_frames: surface.plane_frames.map(|frames| {
                    let mut frames: Vec<_> = frames.iter().copied().collect();
                    frames.sort_by_key(|layer| layer.raw());
                    frames
                }),
                present_pending: surface.present_pending,
                display_moved: surface.display_moved,
                display: surface.display,
                layers,
            }));
        }
        core::future::ready(Ok(FrameRedraw::default()))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn readback(&mut self, surface: SurfaceId) -> Result<Readback, RenderError> {
        let _ = surface;
        Ok(Readback {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn readback(
        &mut self,
        surface: SurfaceId,
    ) -> impl core::future::Future<Output = Result<Readback, RenderError>> {
        let _ = surface;
        core::future::ready(Ok(Readback {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        }))
    }

    fn memory(&self) -> MemoryUsage {
        MemoryUsage::default()
    }

    fn trim(&mut self, _pressure: Pressure) {}

    /// `Null` imports no native frames — nothing to submit.
    fn submit_native_releases(&mut self) {}
}

impl ShaderPaint for Null {
    /// `Null` compiles nothing, so any source is valid.
    fn validate_shader(_source: &ShaderSource) -> Result<(), ResourceError> {
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn add_shader(
        r: &mut NullRenderer,
        id: ShaderId,
        _source: ShaderSource,
    ) -> Result<(), ResourceError> {
        if r.reject.contains(&NullReject::Shader) {
            return Err(ResourceError::Shader("injected rejection".into()));
        }
        r.shaders.insert(id);
        let _ = r.events.send(Event::AddShader(id));
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn add_shader(
        r: &mut NullRenderer,
        id: ShaderId,
        _source: ShaderSource,
    ) -> impl core::future::Future<Output = Result<(), ResourceError>> {
        if r.reject.contains(&NullReject::Shader) {
            return core::future::ready(Err(ResourceError::Shader("injected rejection".into())));
        }
        r.shaders.insert(id);
        let _ = r.events.send(Event::AddShader(id));
        core::future::ready(Ok(()))
    }

    fn remove_shader(r: &mut NullRenderer, id: ShaderId) {
        assert!(
            r.shaders.remove(&id),
            "removal of unregistered shader {}",
            id.raw()
        );
        r.removed.insert(ResourceId::Shader(id));
        let _ = r.events.send(Event::RemoveShader(id));
    }
}

impl Uploads<Rgba8> for Null {}
impl Uploads<Rgba16F> for Null {}

/// `Null` retains no producer state beyond the bindings `submit_frame`
/// needs to name its layers — enough for `frame_producer` to exercise the
/// render loop's plane-eligible change tracking (#90).
impl crate::GpuContent for Null {
    type Content = ();
    type Frame = ();

    fn frame_opaque(_frame: &()) -> bool {
        false
    }

    fn add_gpu_producer(r: &mut NullRenderer, id: crate::ProducerId, _content: ()) {
        if !r.pending_retire.remove(&id) {
            r.producers.insert(
                id,
                NullProducer {
                    content: Some(()),
                    current: None,
                    bindings: FxHashMap::default(),
                },
            );
        }
        let _ = r.events.send(Event::AddProducer(id));
    }

    fn add_frame_producer(
        r: &mut NullRenderer,
        id: crate::ProducerId,
        _dirty: std::sync::Arc<std::sync::atomic::AtomicBool>,
        _gate: std::sync::Arc<crate::WakeGate>,
    ) {
        if !r.pending_retire.remove(&id) {
            r.producers.insert(
                id,
                NullProducer {
                    content: None,
                    current: None,
                    bindings: FxHashMap::default(),
                },
            );
        }
        let _ = r.events.send(Event::AddProducer(id));
    }

    fn bind_gpu_producer(
        r: &mut NullRenderer,
        surface: SurfaceId,
        layer: LayerId,
        producer: &crate::GpuProducer<Self>,
        _size: (u32, u32),
    ) -> Option<bool> {
        let Some(entry) = r.producers.get_mut(&producer.id()) else {
            panic!("binding of an unknown producer {}", producer.id().raw());
        };
        entry.bindings.insert((surface, layer), producer.clone());
        let _ = r
            .events
            .send(Event::BindProducer(surface, layer, producer.id()));
        entry.current.as_ref().map(Self::frame_opaque)
    }

    fn submit_frame(
        r: &mut NullRenderer,
        id: crate::ProducerId,
        _frame: (),
    ) -> Vec<(SurfaceId, LayerId)> {
        // A submitted frame can reach the renderer after its producer
        // retired — the producer's own retire queue can beat a queued
        // `ProducerFrame` message; the frame lands nowhere.
        let Some(producer) = r.producers.get_mut(&id) else {
            return Vec::new();
        };
        producer.current = Some(());
        let mut bound: Vec<_> = producer.bindings.keys().copied().collect();
        bound.sort_by_key(|(surface, layer)| (surface.raw(), layer.raw()));
        for &(surface, layer) in &bound {
            let _ = r.events.send(Event::ProducerFrame(surface, layer));
        }
        bound
    }

    fn retire_gpu_producer(r: &mut NullRenderer, id: crate::ProducerId) {
        // A retirement that beat its producer's registration to the
        // stream is remembered: the add lands already retired.
        if r.producers.remove(&id).is_none() {
            r.pending_retire.insert(id);
        }
        let _ = r.events.send(Event::RetireProducer(id));
    }

    fn drain_gpu_producers(
        r: &mut NullRenderer,
    ) -> Vec<(crate::ProducerId, crate::DrainedProducer<Self>)> {
        let _ = r.events.send(Event::DrainProducers);
        r.producers
            .drain()
            .map(|(id, producer)| {
                (
                    id,
                    match producer.content {
                        Some(()) => crate::DrainedProducer::Rendered(()),
                        None => crate::DrainedProducer::Frame,
                    },
                )
            })
            .collect()
    }
}

/// Evaluates the expression directly on native targets and `.await`s it
/// on wasm32 — `Engine` calls are synchronous on one and futures on the
/// other, the same `cfg(target_arch = "wasm32")` split the library
/// itself makes. Exported so `behaviour_suite!` expansions and shared
/// backend test files write each call site once.
#[cfg(feature = "testing")]
#[doc(hidden)]
#[macro_export]
macro_rules! __engine_wait {
    ($e:expr) => {{
        #[cfg(target_arch = "wasm32")]
        let v = $e.await;
        #[cfg(not(target_arch = "wasm32"))]
        let v = $e;
        v
    }};
}

/// As [`__engine_wait`], but for futures that are async on every target
/// (raw `wgpu` adapter/device requests): `pollster::block_on` on native
/// targets, `.await` on wasm32.
#[cfg(feature = "testing")]
#[doc(hidden)]
#[macro_export]
macro_rules! __engine_block {
    ($e:expr) => {{
        #[cfg(target_arch = "wasm32")]
        let v = $e.await;
        #[cfg(not(target_arch = "wasm32"))]
        let v = ::pollster::block_on($e);
        v
    }};
}

/// Declares the function synchronous on native targets and `async` on
/// wasm32, for test helpers that make `Engine` calls; call sites read
/// `$crate::__engine_wait!(name(..))`.
#[cfg(feature = "testing")]
#[doc(hidden)]
#[macro_export]
macro_rules! __engine_fn {
    ($(#[$m:meta])* $vis:vis fn $name:ident $($rest:tt)*) => {
        $(#[$m])*
        #[cfg(not(target_arch = "wasm32"))]
        $vis fn $name $($rest)*

        $(#[$m])*
        #[cfg(target_arch = "wasm32")]
        #[allow(
            clippy::future_not_send,
            reason = "the macro emits both Send and non-Send futures, and the wasm32 harness runs on the single-threaded page event loop"
        )]
        $vis async fn $name $($rest)*
    };
}

/// As `__engine_fn`, but marks the function a test: `#[test]` on
/// native targets and `#[wasm_bindgen_test]` on wasm32.
#[cfg(feature = "testing")]
#[doc(hidden)]
#[macro_export]
macro_rules! __engine_test {
    ($(#[$m:meta])* fn $name:ident $($rest:tt)*) => {
        $(#[$m])*
        #[cfg(not(target_arch = "wasm32"))]
        #[test]
        fn $name $($rest)*

        $(#[$m])*
        #[cfg(target_arch = "wasm32")]
        #[allow(
            clippy::future_not_send,
            reason = "the macro emits both Send and non-Send futures, and the wasm32 harness runs on the single-threaded page event loop"
        )]
        #[::wasm_bindgen_test::wasm_bindgen_test]
        async fn $name $($rest)*
    };
}

/// Expands to one cross-backend behaviour suite.
///
/// The suite emits `#[test]` functions exercising the shared front end end
/// to end, observing only `Offscreen` readback pixels, so it is identical
/// for every backend.
///
/// Invoke once in a backend crate's `tests/behaviour.rs`:
///
/// ```ignore
/// cherenkov::behaviour_suite! {
///     backend: cherenkov_gpu::Gpu,
///     config: || cherenkov_gpu::GpuConfig::default(),
///     uploads: true,
/// }
/// ```
///
/// `uploads` gates the image-lifetime and image-replacement tests on
/// backends implementing `Uploads<Rgba8>`. GPU backends need the driver
/// environment set by the caller (on this box, lavapipe via `VK_ICD_FILENAMES`/`WGPU_BACKEND`).
/// Available with the `testing` feature.
#[cfg(feature = "testing")]
#[macro_export]
macro_rules! behaviour_suite {
    { backend: $backend:ty, config: $config:expr, uploads: true $(,)? } => {
        $crate::behaviour_suite! { @impl $backend, $config }
        /// Image-lifetime and image-replacement checks for backends
        /// implementing `Uploads<Rgba8>`.
        mod behaviour_suite_uploads {
            use std::time::Duration;
use $crate::Instant;

            use $crate::{Draw as _, Engine, FrameTime, ImageData, Layer, Offscreen, OffscreenFormat, Readback, Rgba8, Sampling, Surface, WorkingColor};
            use $crate::kurbo::{Affine, Rect, Vec2};

            /// The backend under test.
            type B = $backend;
            const TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);
            $crate::__engine_fn! {
fn engine() -> Option<Engine<B>> {
                $crate::__engine_wait!(Engine::<B>::new($config())).ok()
            }
            }

            /// The last `Image` clone's drop queues `remove_image`, which
            /// the backend frees on the next render.
            $crate::__engine_test! {
fn the_last_image_drop_frees_its_memory() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let _surface = $crate::__engine_wait!(engine
                    .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
                    .expect("surface");
                // A first render settles one-off allocations so the
                // baseline is stable.
                $crate::__engine_wait!(engine.render(FrameTime::at(Instant::now()))).expect("render");
                let before = $crate::__engine_wait!(engine.memory()).gpu.0 + $crate::__engine_wait!(engine.memory()).cpu.0;
                let data = vec![255u8; 64 * 64 * 4];
                let image = engine
                    .image(ImageData::<Rgba8>::new(64, 64, data).expect("image data"))
                    .expect("image");
                let clone = image.clone();
                $crate::__engine_wait!(engine
                    .render(FrameTime::at(Instant::now() + TICK)))
                    .expect("render");
                let with_image = $crate::__engine_wait!(engine.memory()).gpu.0 + $crate::__engine_wait!(engine.memory()).cpu.0;
                assert!(with_image > before, "memory {with_image} <= {before}");
                drop(image);
                drop(clone);
                $crate::__engine_wait!(engine
                    .render(FrameTime::at(Instant::now() + TICK * 2)))
                    .expect("render");
                let after = $crate::__engine_wait!(engine.memory()).gpu.0 + $crate::__engine_wait!(engine.memory()).cpu.0;
                assert!(after < with_image, "memory {after} >= {with_image}");
            }
            }

            /// A `width` × `height` image of one opaque texel.
            fn solid(width: u32, height: u32, texel: [u8; 4]) -> ImageData<Rgba8> {
                let data = texel.repeat(width as usize * height as usize);
                ImageData::<Rgba8>::new(width, height, data).expect("image data")
            }

            /// The centre pixel, premultiplied linear P3.
            fn centre(readback: &Readback) -> [f32; 4] {
                readback.pixels[(readback.height / 2 * readback.width + readback.width / 2) as usize]
            }

            /// Records `Draw::image` once over a red 16×16 image and renders
            /// frame A; replaces the image with a blue `width` × `height`
            /// one and renders frame B without re-recording. Then drops
            /// the drawing layer and the last image handle: the replaced
            /// image is still removed and its memory freed.
            $crate::__engine_fn! {
fn replace_redraws_the_recorded_image(width: u32, height: u32) {
                const RED: [u8; 4] = [255, 0, 0, 255];
                const BLUE: [u8; 4] = [0, 0, 255, 255];
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(engine
                    .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
                    .expect("surface");
                let t0 = Instant::now();
                let image = engine.image(solid(16, 16, RED)).expect("image");
                let layer = surface.layer();
                let content = surface.record(|c| {
                    c.image(image.id(), Rect::new(0.0, 0.0, 64.0, 64.0), Sampling::Nearest);
                });
                surface.update(|tx| {
                    tx[surface.root()].push(&layer);
                    tx[&layer].content(content);
                });
                $crate::__engine_wait!(engine.render(FrameTime::at(t0))).expect("render");
                let a = centre(&$crate::__engine_wait!(surface.readback()).expect("readback"));
                assert!(a[0] > 0.5 && a[2] < 0.1, "frame A {a:?} is not the red image");

                image.replace(solid(width, height, BLUE)).expect("replace");
                $crate::__engine_wait!(engine.render(FrameTime::at(t0 + TICK))).expect("render");
                let b = centre(&$crate::__engine_wait!(surface.readback()).expect("readback"));
                assert!(b[2] > 0.5 && b[0] < 0.1, "frame B {b:?} is not the blue replacement");

                drop(layer);
                $crate::__engine_wait!(engine.render(FrameTime::at(t0 + TICK * 2))).expect("render");
                let with_image = $crate::__engine_wait!(engine.memory()).gpu.0 + $crate::__engine_wait!(engine.memory()).cpu.0;
                drop(image);
                $crate::__engine_wait!(engine.render(FrameTime::at(t0 + TICK * 3))).expect("render");
                let after = $crate::__engine_wait!(engine.memory()).gpu.0 + $crate::__engine_wait!(engine.memory()).cpu.0;
                assert!(after < with_image, "memory {after} >= {with_image}");
            }
            }

            /// A same-size replacement reuses the storage behind the id.
            $crate::__engine_test! {
fn a_same_size_replacement_redraws_the_recorded_image() {
                $crate::__engine_wait!(replace_redraws_the_recorded_image(16, 16));
            }
            }

            /// A resized replacement reallocates behind the same id.
            $crate::__engine_test! {
fn a_resized_replacement_redraws_the_recorded_image() {
                $crate::__engine_wait!(replace_redraws_the_recorded_image(8, 32));
            }
            }
        }
    };
    { backend: $backend:ty, config: $config:expr, uploads: false $(,)? } => {
        $crate::behaviour_suite! { @impl $backend, $config }
    };
    { @impl $backend:ty, $config:expr } => {
        mod behaviour_suite {
            use std::time::Duration;
use $crate::Instant;

            use ::nami::SignalExt as _;
            use $crate::kurbo::{Affine, Rect, Vec2};
            use $crate::{
                Animation, Curve, Decay, Draw as _, Engine, FrameTime, ImageData, Layer, Next,
                Offscreen, OffscreenFormat, Readback, Rgba8, Spring, Surface, WorkingColor,
            };

            /// The backend under test.
            type B = $backend;

            /// One frame at 120 Hz, the suite's sampling step.
            const TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);
            /// An opaque white 16×16 square.
            const WHITE: WorkingColor = WorkingColor::WHITE;

            /// A new engine, or `None` when the backend cannot init here
            /// (a GPU backend without an adapter skips its tests).
            $crate::__engine_fn! {
fn engine() -> Option<Engine<B>> {
                $crate::__engine_wait!(Engine::<B>::new($config())).ok()
            }
            }

            /// A 256×64 `Offscreen` surface — wide enough that a scrolled
            /// or translated square stays in view.
            $crate::__engine_fn! {
fn surface(engine: &Engine<B>) -> Surface<B> {
                $crate::__engine_wait!(engine
                    .surface(Offscreen::new((256, 64), OffscreenFormat::LinearF16)))
                    .expect("surface")
            }
            }

            /// A child layer of `parent` holding an opaque square at
            /// `rect`, in the layer's local space.
            fn square(surface: &Surface<B>, parent: &Layer, rect: Rect) -> Layer {
                let layer = surface.layer();
                let content = surface.record(|c| c.fill(rect, WHITE));
                surface.update(|tx| {
                    tx[parent].push(&layer);
                    tx[&layer].content(content);
                });
                layer
            }

            /// The alpha-weighted centroid of pixels inside `region`
            /// (pixel centres, alpha above 0.25). `None` when the region
            /// holds no opaque pixels.
            fn square_center_in(readback: &Readback, region: Rect) -> Option<(f64, f64)> {
                let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
                for y in region.min_y().max(0.0) as u32..region.max_y() as u32 {
                    for x in region.min_x().max(0.0) as u32..region.max_x() as u32 {
                        let a = f64::from(readback.pixels[(y * readback.width + x) as usize][3]);
                        if a > 0.25 {
                            sx = a.mul_add(f64::from(x) + 0.5, sx);
                            sy = a.mul_add(f64::from(y) + 0.5, sy);
                            sw += a;
                        }
                    }
                }
                (sw > 0.5).then_some((sx / sw, sy / sw))
            }

            /// The alpha-weighted centroid of every opaque pixel.
            fn square_center(readback: &Readback) -> Option<(f64, f64)> {
                square_center_in(
                    readback,
                    Rect::new(0.0, 0.0, readback.width.into(), readback.height.into()),
                )
            }

            /// The alpha channel of one pixel.
            fn alpha_at(readback: &Readback, x: u32, y: u32) -> f64 {
                f64::from(readback.pixels[(y * readback.width + x) as usize][3])
            }

            /// Renders at `t` and returns the `Next`.
            $crate::__engine_fn! {
fn render_at(engine: &Engine<B>, t: Instant) -> Next {
                $crate::__engine_wait!(engine.render(FrameTime::at(t))).expect("render")
            }
            }

            /// Renders frames at `t`, `t + TICK`, … until `Next::Idle`
            /// (cap 2000 frames) and returns the last sampled position.
            $crate::__engine_fn! {
fn settle(engine: &Engine<B>, surface: &Surface<B>, t0: Instant) -> (f64, f64) {
                let mut t = t0;
                for _ in 0..2000 {
                    if $crate::__engine_wait!(render_at(engine, t)) == Next::Idle {
                        break;
                    }
                    t += TICK;
                }
                square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("a drawn square")
            }
            }

            $crate::__engine_test! {
fn layer_tree_edits_change_what_is_drawn() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                let window_a = Rect::new(4.0, 24.0, 20.0, 44.0);
                let window_b = Rect::new(64.0, 24.0, 80.0, 44.0);
                let t0 = Instant::now();
                let mut frame = 0u64;

                let a = square(&surface, &surface.root(), Rect::new(8.0, 28.0, 16.0, 40.0));
                frame += 1;
                $crate::__engine_wait!(render_at(&engine, t0 + TICK * frame as u32));
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                assert!(square_center_in(&rb, window_a).is_some(), "a not drawn");
                assert!(square_center_in(&rb, window_b).is_none(), "b drawn early");

                // `insert` at index 0 puts b first; both are drawn.
                let b = surface.layer();
                let content = surface.record(|c| c.fill(Rect::new(68.0, 28.0, 76.0, 40.0), WHITE));
                surface.update(|tx| {
                    tx[surface.root()].insert(0, &b);
                    tx[&b].content(content);
                });
                frame += 1;
                $crate::__engine_wait!(render_at(&engine, t0 + TICK * frame as u32));
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                assert!(square_center_in(&rb, window_a).is_some(), "a missing");
                assert!(square_center_in(&rb, window_b).is_some(), "b missing");

                // `remove` detaches a from the tree; only b is drawn.
                surface.update(|tx| {
                    tx[surface.root()].remove(&a);
                });
                frame += 1;
                $crate::__engine_wait!(render_at(&engine, t0 + TICK * frame as u32));
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                assert!(square_center_in(&rb, window_a).is_none(), "a still drawn");
                assert!(square_center_in(&rb, window_b).is_some(), "b missing");

                // Re-attach a with `push`, then drop b: only a remains.
                surface.update(|tx| {
                    tx[surface.root()].push(&a);
                });
                drop(b);
                frame += 1;
                $crate::__engine_wait!(render_at(&engine, t0 + TICK * frame as u32));
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                assert!(square_center_in(&rb, window_a).is_some(), "a missing");
                assert!(square_center_in(&rb, window_b).is_none(), "b still drawn");
            }
            }

            $crate::__engine_test! {
fn a_transform_spring_settles_at_its_target() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                // Square centre starts at (32, 32); the spring targets
                // translate(16, 0) → (48, 32).
                let layer = square(&surface, &surface.root(), Rect::new(24.0, 24.0, 40.0, 40.0));
                surface.update(|tx| {
                    tx[&layer].transform(Affine::IDENTITY);
                });
                surface.update_animated(
                    Spring {
                        response: 0.4,
                        damping: 1.0,
                    },
                    |tx| {
                        tx[&layer].transform(Affine::translate((16.0, 0.0)));
                    },
                );
                let t0 = Instant::now();
                $crate::__engine_wait!(render_at(&engine, t0));
                let (x0, y0) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                assert!((x0 - 32.0).abs() < 0.5 && (y0 - 32.0).abs() < 0.5, "start {x0},{y0}");

                $crate::__engine_wait!(render_at(&engine, t0 + TICK * 6));
                let (x1, _) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                assert!(x1 > x0 + 0.5 && x1 < 48.0, "mid {x1}");

                let (x2, y2) = $crate::__engine_wait!(settle(&engine, &surface, t0 + TICK * 6));
                assert!((x2 - 48.0).abs() < 0.5 && (y2 - 32.0).abs() < 0.5, "end {x2},{y2}");
            }
            }

            $crate::__engine_test! {
fn a_curve_hits_its_endpoints_exactly() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                let layer = square(&surface, &surface.root(), Rect::new(24.0, 24.0, 40.0, 40.0));
                surface.update(|tx| {
                    tx[&layer].transform(Affine::IDENTITY);
                });
                surface.update_animated(
                    Curve::ease_in_out(Duration::from_millis(160)),
                    |tx| {
                        tx[&layer].transform(Affine::translate((16.0, 0.0)));
                    },
                );
                let t0 = Instant::now();
                $crate::__engine_wait!(render_at(&engine, t0));
                let (x0, _) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                assert!((x0 - 32.0).abs() < 0.5, "start {x0}");

                // At and past the duration the value is exactly the target.
                let next = $crate::__engine_wait!(render_at(&engine, t0 + Duration::from_millis(200)));
                assert_eq!(next, Next::Idle, "{next:?}");
                let (x1, _) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                assert!((x1 - 48.0).abs() < 0.01, "end {x1}");
            }
            }

            $crate::__engine_test! {
fn a_retargeted_spring_keeps_its_velocity() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                let layer = square(&surface, &surface.root(), Rect::new(8.0, 24.0, 24.0, 40.0));
                // A linear curve runs at constant velocity — the
                // pre-retarget samples give v exactly, so the post-retarget
                // step can be checked against v·dt at 1%. (A spring carrier
                // drifts several % per frame across the 2-frame sampling
                // gap, making 1% unreachable through pixel centroids.)
                surface.update(|tx| {
                    tx[&layer].transform(Affine::IDENTITY);
                });
                surface.update_animated(
                    Curve::linear(Duration::from_millis(400)),
                    |tx| {
                        tx[&layer].transform(Affine::translate((160.0, 0.0)));
                    },
                );
                let t0 = Instant::now();
                // Mid-flight at t1: measure the incoming velocity from the
                // two samples just before it.
                let t1 = t0 + TICK * 8;
                $crate::__engine_wait!(render_at(&engine, t1 - TICK));
                let (c0, _) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                $crate::__engine_wait!(render_at(&engine, t1));
                let (c1, _) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                let dt = TICK.as_secs_f64();
                let v = (c1 - c0) / dt;
                assert!(v > 30.0, "not mid-flight: v={v}");

                // Retarget into a very soft, critically damped spring
                // (response 4 s, ω = 2π/4). It starts at the committed
                // position with the inherited velocity v and follows
                // x(t) = x_t + (C1 + C2·t)·e^(−ωt), C1 = x0 − x_t, C2 = v + ω·C1,
                // which over one frame is about 1% short of v·dt: the step
                // is checked against that trajectory, so it tests the
                // inherited velocity rather than the spring's own pull.
                surface.update(|tx| {
                    tx[&layer]
                        .transform(Affine::translate((160.0, 16.0)))
                        .animation(Spring {
                            response: 4.0,
                            damping: 1.0,
                        });
                });
                // The commit frame samples the new track at dt = 0; the
                // step after it must equal the incoming velocity · dt.
                $crate::__engine_wait!(render_at(&engine, t1 + TICK));
                let (c2, _) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                $crate::__engine_wait!(render_at(&engine, t1 + TICK * 2));
                let (c3, _) =
                    square_center(&$crate::__engine_wait!(surface.readback()).expect("readback")).expect("square");
                // The square's centre at the target: 16 + 160.
                let target = 176.0;
                let omega = std::f64::consts::TAU / 4.0;
                let offset = c2 - target;
                let expected = omega
                    .mul_add(offset, v)
                    .mul_add(dt, offset)
                    .mul_add((-omega * dt).exp(), -offset);
                // While the transform animates the layer's translation is
                // placed on the ¼-pixel grid, so each measured centre carries
                // up to ~¼ px of quantization and centroid bias; the same
                // uncertainty feeds back through v.
                assert!(
                    (c3 - c2 - expected).abs() <= (v * dt).abs().mul_add(0.01, 0.75),
                    "displacement {} vs the spring's trajectory {expected} (v·dt {})",
                    c3 - c2,
                    v * dt
                );
            }
            }

            $crate::__engine_test! {
fn a_scroll_decay_covers_velocity_over_deceleration() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                // Square centre (200, 32); a (600, 0)/k=4 decay covers
                // 150 px of scroll, moving the square 150 px left.
                let layer = square(&surface, &surface.root(), Rect::new(192.0, 24.0, 208.0, 40.0));
                surface.update(|tx| {
                    tx[&layer].scroll_offset(Vec2::ZERO);
                });
                surface.update(|tx| {
                    tx[&layer]
                        .scroll_offset(Vec2::ZERO)
                        .animation(Decay {
                            velocity: Vec2::new(600.0, 0.0),
                            deceleration: 4.0,
                            rubber_band: None,
                        });
                });
                let t0 = Instant::now();
                let (x2, y2) = $crate::__engine_wait!(settle(&engine, &surface, t0));
                // Pixel-snapped: 150 ± 1.
                assert!((x2 - 50.0).abs() < 1.0 && (y2 - 32.0).abs() < 1.0, "end {x2},{y2}");
            }
            }

            $crate::__engine_test! {
fn a_rubber_band_returns_to_the_bound_edge() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                let layer = square(&surface, &surface.root(), Rect::new(192.0, 24.0, 208.0, 40.0));
                // Bounds x ∈ [0, 100]: the decay overshoots to 150 and the
                // rubber band pulls it back to 100.
                let bounds = Rect::new(-10.0, -10.0, 100.0, 100.0);
                surface.update(|tx| {
                    tx[&layer]
                        .scroll_offset(Vec2::ZERO)
                        .animation(Decay::new(Vec2::new(600.0, 0.0)).rubber_band(bounds));
                });
                let t0 = Instant::now();
                let (x2, y2) = $crate::__engine_wait!(settle(&engine, &surface, t0));
                assert!((x2 - 100.0).abs() < 1.0 && (y2 - 32.0).abs() < 1.0, "end {x2},{y2}");
            }
            }

            $crate::__engine_test! {
fn a_bound_signal_updates_opacity_without_a_transaction() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                let layer = square(&surface, &surface.root(), Rect::new(24.0, 24.0, 40.0, 40.0));
                let opacity = ::nami::binding(1.0f32);
                surface.update(|tx| {
                    tx[&layer].opacity(opacity.clone());
                });
                let t0 = Instant::now();
                $crate::__engine_wait!(render_at(&engine, t0));
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                assert!((alpha_at(&rb, 32, 32) - 1.0).abs() < 0.05);

                // A plain signal change snaps the opacity.
                opacity.set(0.4f32);
                let next = $crate::__engine_wait!(render_at(&engine, t0 + TICK));
                assert_eq!(next, Next::Idle, "{next:?}");
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                assert!((alpha_at(&rb, 32, 32) - 0.4).abs() < 0.05, "alpha");

                // `with(Animation)` metadata animates the change: a sample
                // one frame in is strictly between the endpoints.
                let layer2 = square(&surface, &surface.root(), Rect::new(24.0, 24.0, 40.0, 40.0));
                surface.update(|tx| {
                    tx[surface.root()].remove(&layer);
                    tx[&layer2].opacity(opacity.clone().with(Animation::from(Spring::smooth())));
                });
                opacity.set(0.0f32);
                // The commit frame samples at dt = 0 → still 0.4.
                $crate::__engine_wait!(render_at(&engine, t0 + TICK * 3));
                let next = $crate::__engine_wait!(render_at(&engine, t0 + TICK * 8));
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                let a = alpha_at(&rb, 32, 32);
                assert!(a > 0.02 && a < 0.39, "interpolated alpha {a}");
                assert!(matches!(next, Next::At { .. }), "{next:?}");
            }
            }

            $crate::__engine_test! {
fn a_recorded_operand_animates_between_paints() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                let layer = surface.layer();
                let red = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
                let green = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);
                let blue = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
                let animated = ::nami::binding(red);
                let snapped = ::nami::binding(red);
                let content = surface.record(|c| {
                    c.fill(
                        Rect::new(8.0, 8.0, 24.0, 56.0),
                        animated.clone().with(Animation::from(Curve::linear(
                            Duration::from_millis(400),
                        ))),
                    );
                    c.fill(Rect::new(64.0, 8.0, 80.0, 56.0), snapped.clone());
                });
                surface.update(|tx| {
                    tx[surface.root()].push(&layer);
                    tx[&layer].content(content);
                });
                let t0 = Instant::now();
                $crate::__engine_wait!(render_at(&engine, t0));

                // The commit frame samples at dt = 0 — the animated operand
                // still shows `from` while the plain change snaps.
                animated.set(green);
                snapped.set(blue);
                let next = $crate::__engine_wait!(render_at(&engine, t0 + TICK));
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                let start = rb.pixels[(16 * rb.width + 16) as usize];
                assert!(start[0] > 0.95, "the operand starts at {start:?}");
                let snapped_px = rb.pixels[(16 * rb.width + 72) as usize];
                assert!(
                    snapped_px[2] > 0.95,
                    "the un-animated change snapped: {snapped_px:?}"
                );
                assert!(matches!(next, Next::At { .. }), "{next:?}");

                // Mid-flight the colour is strictly between the endpoints,
                // and only the animated command re-lowers per frame.
                let next = $crate::__engine_wait!(render_at(&engine, t0 + TICK * 25));
                assert_eq!(
                    engine.stats().commands_lowered,
                    1,
                    "a running operand animation re-lowers only its own command"
                );
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                let mid = rb.pixels[(16 * rb.width + 16) as usize];
                assert!(mid[0] < 0.95 && mid[1] > 0.05, "mid-flight {mid:?}");
                assert!(matches!(next, Next::At { .. }), "{next:?}");

                // Retargeting mid-flight keeps animating from the sampled
                // position: the colour lands on the new target and idles.
                animated.set(blue);
                let mut t = t0 + TICK * 25;
                for _ in 0..2000 {
                    if $crate::__engine_wait!(render_at(&engine, t)) == Next::Idle {
                        break;
                    }
                    t += TICK;
                }
                let rb = $crate::__engine_wait!(surface.readback()).expect("readback");
                let end = rb.pixels[(16 * rb.width + 16) as usize];
                assert!(end[2] > 0.95 && end[0] < 0.05, "settled at {end:?}");
            }
            }

            $crate::__engine_test! {
fn next_schedules_the_frame_rate() {
                let Some(engine) = $crate::__engine_wait!(engine()) else { return };
                let surface = $crate::__engine_wait!(surface(&engine));
                let layer = square(&surface, &surface.root(), Rect::new(8.0, 24.0, 24.0, 40.0));
                surface.update_animated(
                    Spring {
                        response: 0.4,
                        damping: 1.0,
                    },
                    |tx| {
                        tx[&layer].transform(Affine::translate((160.0, 0.0)));
                    },
                );
                let t0 = Instant::now();
                // A running spring wants the fast class.
                match $crate::__engine_wait!(render_at(&engine, t0 + TICK)) {
                    Next::At { rate, .. } => {
                        assert!(*rate.start() <= 60 && *rate.end() >= 120, "rate {rate:?}");
                    }
                    next => panic!("expected At while the spring runs, got {next:?}"),
                }

                // Once only a slow decay remains (under one device pixel
                // per 60 Hz frame), the rate drops to the slow class.
                let decay_layer =
                    square(&surface, &surface.root(), Rect::new(192.0, 24.0, 208.0, 40.0));
                let _ = layer; // keep the settled spring's layer alive
                surface.update(|tx| {
                    tx[&decay_layer]
                        .scroll_offset(Vec2::ZERO)
                        .animation(Decay::new(Vec2::new(30.0, 0.0)));
                });
                // Wait for the spring to settle; the decay outlives it
                // only briefly, so check the class on the first sample.
                let t2 = t0 + TICK * 200;
                let next = $crate::__engine_wait!(render_at(&engine, t2));
                match next {
                    Next::At { rate, .. } => {
                        assert!(*rate.end() <= 60, "rate {rate:?}");
                    }
                    Next::Idle => {}
                }
                // Everything comes to rest eventually.
                let _ = $crate::__engine_wait!(settle(&engine, &surface, t2));
                assert_eq!($crate::__engine_wait!(render_at(&engine, t2 + TICK * 400)), Next::Idle);
            }
            }

        }
    };
}

/// Registration lifetime checks shared by both executors: the committed
/// `Add*`/`Remove*` (`CreateSurface`/`DestroySurface`) events of each
/// resource kind must balance (#150).
#[cfg(test)]
mod balance {
    use std::sync::mpsc::Receiver;

    use super::Event;

    /// The registration transitions in `events`, as `kind raw-id` strings.
    /// Returns `(committed, released)`.
    pub fn transitions(events: &[Event]) -> (Vec<String>, Vec<String>) {
        let (mut commits, mut releases) = (Vec::new(), Vec::new());
        for event in events {
            let (kind, id, committed) = match event {
                Event::AddFont(id) => ("font", id.raw(), true),
                Event::RemoveFont(id) => ("font", id.raw(), false),
                Event::AddImage(id) => ("image", id.raw(), true),
                Event::RemoveImage(id) => ("image", id.raw(), false),
                Event::AddShader(id) => ("shader", id.raw(), true),
                Event::RemoveShader(id) => ("shader", id.raw(), false),
                Event::CreateSurface(id) => ("surface", id.raw(), true),
                Event::DestroySurface(id) => ("surface", id.raw(), false),
                _ => continue,
            };
            let entry = format!("{kind} {id}");
            if committed {
                commits.push(entry);
            } else {
                releases.push(entry);
            }
        }
        (commits, releases)
    }

    /// Drains the probe and asserts every committed registration was
    /// released exactly once, kind and id matching.
    pub fn assert_balanced(rx: &Receiver<Event>) {
        let events: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let (mut commits, mut releases) = transitions(&events);
        commits.sort();
        releases.sort();
        assert_eq!(
            commits, releases,
            "unbalanced registration events: {events:?}"
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::Instant;
    use std::time::Duration;

    use super::*;
    use crate::image::ImageData;
    use crate::resource::{FontSource, GpuProducer};
    use crate::{
        Decay, Engine, FrameTime, Image, Layer, Next, OffscreenFormat, ShaderSource, Spring,
        Surface,
    };

    fn engine() -> (Engine<Null>, std::sync::mpsc::Receiver<Event>) {
        engine_rejecting(HashSet::new())
    }

    fn engine_rejecting(
        reject: HashSet<NullReject>,
    ) -> (Engine<Null>, std::sync::mpsc::Receiver<Event>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events: tx,
            reject,
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .expect("init");
        (engine, rx)
    }

    /// An image the backend's [`ImageLimits`] does not admit is rejected
    /// on the calling thread: `Engine::image` returns `TooLarge` naming
    /// the limits and queues nothing, and an oversized `Image::replace`
    /// keeps the previous pixels.
    #[test]
    fn an_image_beyond_the_limits_fails_at_registration() {
        let (tx, rx) = std::sync::mpsc::channel();
        let limits = crate::ImageLimits {
            max_dimension: 4,
            max_texels: 16,
        };
        let engine = Engine::<Null>::new(NullConfig {
            events: tx,
            reject: HashSet::new(),
            image_limits: limits,
        })
        .expect("init");
        assert_eq!(engine.image_limits(), limits);

        // The dimension binds.
        match engine.image(ImageData::<Rgba8>::new(5, 1, vec![0u8; 20]).expect("image data")) {
            Err(ResourceError::TooLarge {
                width,
                height,
                limits: rejected,
            }) => {
                assert_eq!((width, height, rejected), (5, 1, limits));
            }
            other => panic!("an oversized image registered: {other:?}"),
        }
        // And the texel count does too.
        assert!(matches!(
            engine.image(ImageData::<Rgba8>::new(4, 5, vec![0u8; 80]).expect("image data")),
            Err(ResourceError::TooLarge { .. })
        ));

        let image = engine
            .image(ImageData::<Rgba8>::new(4, 1, vec![0u8; 16]).expect("image data"))
            .expect("image");
        // A replacement over the limits fails on the calling thread; an
        // admitted one still applies.
        match image.replace(ImageData::<Rgba8>::new(4, 5, vec![0u8; 80]).expect("image data")) {
            Err(ResourceError::TooLarge { .. }) => {}
            other => panic!("an oversized replacement applied: {other:?}"),
        }
        image
            .replace(ImageData::<Rgba8>::new(4, 1, vec![9u8; 16]).expect("image data"))
            .expect("replace");

        // `memory` round-trips the render thread, flushing the queue:
        // only the admitted registration and replacement reached it.
        let _ = engine.memory();
        let events: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.as_slice(),
                [Event::AddImage(_), Event::ReplaceImage(_, (4, 1))]
            ),
            "{events:?}"
        );
    }

    fn frames(rx: &std::sync::mpsc::Receiver<Event>) -> Vec<FrameRecord> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let Event::Frame(record) = event {
                out.push(record);
            }
        }
        out
    }

    fn layer(record: &FrameRecord, id: LayerId) -> &LayerSample {
        record
            .layers
            .iter()
            .find(|l| l.id == id)
            .expect("layer in record")
    }

    /// A scroll axis pinned to one value (`x0 == x1` in the bounds) is a
    /// legal bounds rect: `Rect::contains` is half-open, so the rubber
    /// band must not fire for an offset inside such a rect.
    #[test]
    fn rubber_band_with_a_zero_width_bounds_rect() {
        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
            .expect("surface");
        let layer_handle = surface.layer();
        let bounds = kurbo::Rect::new(0.0, 0.0, 0.0, 300.0);
        surface.update(|tx| {
            tx[surface.root()].push(&layer_handle);
            tx[&layer_handle]
                .scroll_offset(Vec2::new(0.0, 100.0))
                .animation(Decay::new(Vec2::new(0.0, 2000.0)).rubber_band(bounds));
        });
        let t0 = Instant::now();
        let mut t = t0;
        loop {
            t += Duration::from_millis(8);
            let next = engine.render(FrameTime::at(t)).expect("render");
            if matches!(next, Next::Idle) {
                break;
            }
            assert!(t - t0 < Duration::from_secs(10), "never settled");
        }
        let record = frames(&rx).pop().expect("records");
        let offset = layer(&record, layer_handle.id()).scroll_offset;
        assert_eq!(offset, Vec2::new(0.0, 300.0), "settled offset");
    }

    #[test]
    fn decay_starts_at_committed_value_and_stays_where_it_stops() {
        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
            .expect("surface");
        let layer_handle = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].push(&layer_handle);
            tx[&layer_handle]
                .scroll_offset(Vec2::ZERO)
                .animation(Decay::new(Vec2::new(600.0, 0.0)));
        });
        let t0 = Instant::now();
        let mut t = t0;
        // Unbounded decay: position = v/k = 600/4 = 150, then Idle.
        loop {
            t += Duration::from_millis(8);
            let next = engine.render(FrameTime::at(t)).expect("render");
            if matches!(next, Next::Idle) {
                break;
            }
            assert!(t - t0 < Duration::from_secs(10), "never settled");
        }
        let record = frames(&rx).pop().expect("records");
        let offset = layer(&record, layer_handle.id()).scroll_offset;
        assert!(
            (offset.x - 150.0).abs() < 0.5 && offset.y.abs() < 0.5,
            "offset {offset:?}, expected ≈(150, 0)"
        );
    }

    #[test]
    fn resource_drop_reaches_the_renderer() {
        let (engine, rx) = engine();
        let _surface = engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
            .expect("surface");
        {
            let image = engine
                .image(ImageData::<Rgba8>::new(2, 2, vec![0u8; 16]).expect("image data"))
                .expect("image");
            let _ = image.id();
        }
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events.iter().any(|e| matches!(e, Event::RemoveImage(_))),
            "no RemoveImage in {events:?}"
        );
    }

    /// Two surfaces — `drawing` draws `image`, `other` a fill — plus an
    /// `unused` image nothing draws, already settled: the last two rendered
    /// frames changed nothing and the event probe is drained.
    struct ImageScene {
        engine: Engine<Null>,
        rx: std::sync::mpsc::Receiver<Event>,
        drawing: Surface<Null>,
        other: Surface<Null>,
        image: Image<Rgba8>,
        unused: Image<Rgba8>,
        image_layer: Layer,
        /// Held only for its lifetime: dropping the layer queues a remove
        /// on `other` and fires the waker.
        fill_layer: Layer,
    }

    fn image_replacement_scene() -> ImageScene {
        use crate::{Draw as _, Sampling, WorkingColor};

        let (engine, rx) = engine();
        let drawing = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let other = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let unused = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let image_layer = drawing.layer();
        let content = drawing.record(|c| {
            c.image(
                image.id(),
                kurbo::Rect::new(0., 0., 8., 8.),
                Sampling::Nearest,
            );
        });
        drawing.update(|tx| {
            tx[drawing.root()].push(&image_layer);
            tx[&image_layer].content(content);
        });
        let fill_layer = other.layer();
        let content =
            other.record(|c| c.fill(kurbo::Rect::new(0., 0., 8., 8.), WorkingColor::WHITE));
        other.update(|tx| {
            tx[other.root()].push(&fill_layer);
            tx[&fill_layer].content(content);
        });
        engine.render(FrameTime::now()).expect("render");
        engine.render(FrameTime::now()).expect("render");
        let settled = frames(&rx);
        assert!(
            settled.iter().rev().take(2).all(|record| !record.changed),
            "nothing changed since the first render: {settled:?}"
        );
        ImageScene {
            engine,
            rx,
            drawing,
            other,
            image,
            unused,
            image_layer,
            fill_layer,
        }
    }

    /// `Image::replace` reaches the renderer with the new dimensions, marks
    /// changed only the surface whose content draws the image and wakes the
    /// host once between two renders; replacing an image nothing draws
    /// marks nothing. After the last drop the image is removed once the
    /// content stops drawing it.
    #[test]
    fn image_replacement_redraws_and_still_releases() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        use crate::{Draw as _, WorkingColor};

        let ImageScene {
            engine,
            rx,
            drawing,
            other,
            image,
            unused,
            image_layer,
            fill_layer: _fill_layer,
        } = image_replacement_scene();

        let wakes = Arc::new(AtomicU32::new(0));
        engine.set_waker({
            let wakes = Arc::clone(&wakes);
            move || {
                wakes.fetch_add(1, Ordering::Relaxed);
            }
        });
        image
            .replace(ImageData::<Rgba8>::new(2, 3, vec![0u8; 24]).expect("image data"))
            .expect("replace");
        // The render loop wakes the host once it applied the replacement;
        // the reply lands only after every earlier message was applied.
        let _ = engine.memory();
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "a replacement wakes the host"
        );
        unused
            .replace(ImageData::<Rgba8>::new(2, 2, vec![0u8; 16]).expect("image data"))
            .expect("replace");
        let _ = engine.memory();
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "at most one wake before a render"
        );
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::ReplaceImage(id, (2, 3)) if *id == image.id())),
            "no ReplaceImage in {events:?}"
        );
        let changed = |surface| {
            events.iter().find_map(|e| match e {
                Event::Frame(record) if record.surface == surface => Some(record.changed),
                _ => None,
            })
        };
        assert_eq!(
            changed(drawing.id()),
            Some(true),
            "the surface drawing the image was not marked changed: {events:?}"
        );
        assert_eq!(
            changed(other.id()),
            Some(false),
            "a surface not drawing the image was marked changed: {events:?}"
        );

        let id = image.id();
        drop(image);
        drawing.update(|tx| {
            tx[&image_layer]
                .record(|c| c.fill(kurbo::Rect::new(0., 0., 8., 8.), WorkingColor::WHITE));
        });
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::RemoveImage(removed) if *removed == id)),
            "no RemoveImage in {events:?}"
        );
    }

    /// A commit that only installs submitted frames reports the installed
    /// layers in the frame's `plane_frames`; a commit that changes anything
    /// else — a layer op, a content op — reports `None` (#90).
    #[test]
    fn frame_only_commits_fill_plane_frames() {
        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let video = surface.layer();
        let above = surface.layer();
        let (producer, sink) = engine.frame_producer();
        let content = |producer: &GpuProducer<Null>| producer.at((8, 8));
        // Creating and pushing the layers is an ordinary change, so the
        // frame that also binds the producer is not plane-only.
        surface.update(|tx| {
            tx[surface.root()].push(&video);
            tx[surface.root()].push(&above);
            tx[&video].content(content(&producer));
        });
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("a frame record");
        assert!(record.changed);
        assert_eq!(
            record.plane_frames, None,
            "a commit with layer ops is not plane-only"
        );

        // A new frame on the layer alone is the frame's only change.
        sink.submit(());
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("a frame record");
        assert!(record.changed);
        assert_eq!(record.plane_frames, Some(vec![video.id()]));

        // Two bindings' new frames commute to one set.
        surface.update(|tx| {
            tx[&above].content(content(&producer));
        });
        engine.render(FrameTime::now()).expect("render");
        sink.submit(());
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("a frame record");
        assert!(record.changed);
        assert_eq!(record.plane_frames, Some(vec![video.id(), above.id()]));

        // A frame submitted alongside a layer op is an ordinary change.
        surface.update(|tx| {
            tx[&above].opacity(0.5f32);
        });
        sink.submit(());
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("a frame record");
        assert!(record.changed);
        assert_eq!(record.plane_frames, None);

        // An untouched surface records neither.
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("a frame record");
        assert!(!record.changed);
        assert_eq!(record.plane_frames, None);
    }

    #[test]
    fn update_animated_fills_the_default() {
        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
            .expect("surface");
        let layer_handle = surface.layer();
        surface.update_animated(Spring::bouncy(), |tx| {
            tx[surface.root()].push(&layer_handle);
            tx[&layer_handle].opacity(0.5f32);
        });
        let next = engine.render(FrameTime::now()).expect("render");
        assert!(matches!(next, Next::At { .. }), "{next:?}");
        let _ = frames(&rx);
    }

    /// Installs a host wake callback that counts its calls.
    fn counting_waker(engine: &Engine<Null>) -> std::sync::Arc<std::sync::atomic::AtomicU32> {
        let wakes = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        engine.set_waker({
            let wakes = std::sync::Arc::clone(&wakes);
            move || {
                wakes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });
        wakes
    }

    /// A window-like surface with a running transform animation, a bound
    /// opacity and a live content that fills with a bound colour and draws
    /// an image, rendered twice so the host is idle and armed.
    struct HiddenScene {
        engine: Engine<Null>,
        rx: std::sync::mpsc::Receiver<Event>,
        surface: Surface<Null>,
        image: Image<Rgba8>,
        moving: Layer,
        drawing: Layer,
        opacity: nami::Binding<f32>,
        color: nami::Binding<crate::WorkingColor>,
        /// Where `moving` animates to, over 400 ms from `t0`.
        target: Affine,
        t0: Instant,
        wakes: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    fn hidden_scene() -> HiddenScene {
        use crate::{Curve, Draw as _, Sampling, WorkingColor};

        let (engine, rx) = engine();
        let surface = engine
            .surface(NullTarget::Window(Offscreen::new(
                (16, 16),
                OffscreenFormat::LinearF16,
            )))
            .expect("surface");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let moving = surface.layer();
        let drawing = surface.layer();
        let opacity = nami::binding(1.0f32);
        let color = nami::binding(WorkingColor::WHITE);
        let content = surface.record(|c| {
            c.fill(kurbo::Rect::new(0., 0., 8., 8.), color.clone());
            c.image(
                image.id(),
                kurbo::Rect::new(8., 8., 16., 16.),
                Sampling::Nearest,
            );
        });
        surface.update(|tx| {
            tx[surface.root()].push(&moving).push(&drawing);
            tx[&drawing].opacity(opacity.clone()).content(content);
        });
        let t0 = Instant::now();
        engine.render(FrameTime::at(t0)).expect("render");
        let target = Affine::translate((40.0, 0.0));
        surface.update(|tx| {
            tx[&moving]
                .transform(target)
                .animation(Curve::linear(Duration::from_millis(400)));
        });
        let next = engine
            .render(FrameTime::at(t0 + Duration::from_millis(8)))
            .expect("render");
        assert!(
            matches!(next, Next::At { .. }),
            "the animation runs: {next:?}"
        );
        let _ = frames(&rx);
        let wakes = counting_waker(&engine);
        HiddenScene {
            engine,
            rx,
            surface,
            image,
            moving,
            drawing,
            opacity,
            color,
            target,
            t0,
            wakes,
        }
    }

    /// The only frame record among `events`.
    fn single_frame(events: &[Event]) -> &FrameRecord {
        let records: Vec<&FrameRecord> = events
            .iter()
            .filter_map(|event| match event {
                Event::Frame(record) => Some(record),
                _ => None,
            })
            .collect();
        let [record] = records.as_slice() else {
            panic!("one frame: {events:?}");
        };
        record
    }

    /// A hidden surface asks for no frame. With an animation running, a
    /// bound signal, a live operand, a transaction, a new layer and an
    /// image replacement it draws wake no host; their changes reach the
    /// renderer as they are made, and nothing is drawn. Rendering while it
    /// is the engine's only surface is an error. Showing it wakes the host
    /// exactly once, and the next frame draws every change made while it
    /// was hidden, samples the animation at that frame's time and
    /// presents.
    #[test]
    fn hidden_surface_schedules_no_frames() {
        use std::sync::atomic::Ordering;

        use crate::{Draw as _, Visibility, WorkingColor};

        let HiddenScene {
            engine,
            rx,
            surface,
            image,
            moving,
            drawing,
            opacity,
            color,
            target,
            t0,
            wakes,
        } = hidden_scene();
        surface.visibility(Visibility::Hidden).expect("hide");
        surface
            .visibility(Visibility::Hidden)
            .expect("announcing the same visibility again");
        opacity.set(0.25);
        color.set(WorkingColor::BLACK);
        let added = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].push(&added);
            tx[&added].record(|c| c.fill(kurbo::Rect::new(0., 0., 4., 4.), WorkingColor::WHITE));
        });
        image
            .replace(ImageData::<Rgba8>::new(2, 2, vec![0u8; 16]).expect("image data"))
            .expect("replace");
        // The reply lands only after every earlier message was applied, so
        // a wake the render loop would fire has fired.
        let _ = engine.memory();
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            0,
            "a hidden surface woke the host"
        );
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.as_slice(),
                [
                    Event::Visibility(id, Visibility::Hidden),
                    Event::SetContent(operand, drawn, ..),
                    Event::SetContent(transaction, installed, ..),
                    Event::ReplaceImage(replaced, (2, 2)),
                ] if *id == surface.id()
                    && *operand == surface.id()
                    && *drawn == drawing.id()
                    && *transaction == surface.id()
                    && *installed == added.id()
                    && *replaced == image.id()
            ),
            "a hidden surface's changes reach the renderer as they are made, undrawn: {events:?}"
        );
        assert!(
            matches!(
                engine.render(FrameTime::at(t0 + Duration::from_millis(16))),
                Err(RenderError::Hidden)
            ),
            "rendering while every surface is hidden is an error"
        );

        surface.visibility(Visibility::Visible).expect("show");
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "showing the surface asks for exactly one frame"
        );
        surface
            .visibility(Visibility::Visible)
            .expect("announcing the same visibility again");
        assert_eq!(wakes.load(Ordering::Relaxed), 1, "no second wake");
        let next = engine
            .render(FrameTime::at(t0 + Duration::from_secs(10)))
            .expect("render");
        assert_eq!(
            next,
            Next::Idle,
            "the animation is sampled at the frame time, past its end"
        );
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let record = single_frame(&events);
        assert!(record.changed, "the shown surface redraws");
        assert!(record.present_pending, "the shown surface presents");
        assert_eq!(layer(record, moving.id()).transform, target, "no catch-up");
        assert!(
            (layer(record, drawing.id()).opacity - 0.25).abs() < f32::EPSILON,
            "the bound opacity changed while hidden"
        );
        assert_eq!(
            layer(record, surface.root().id()).children,
            [moving.id(), drawing.id(), added.id()],
            "the layer added while hidden"
        );
    }

    /// A host asked for a frame, then saw the surface hide and dropped the
    /// request: showing the surface still wakes it, though no render
    /// re-armed the waker in between.
    #[test]
    fn showing_wakes_a_host_that_dropped_its_request() {
        use std::sync::atomic::Ordering;

        use crate::Visibility;

        let scene = hidden_scene();
        scene.opacity.set(0.5);
        assert_eq!(
            scene.wakes.load(Ordering::Relaxed),
            1,
            "a visible change wakes"
        );
        scene.surface.visibility(Visibility::Hidden).expect("hide");
        scene.surface.visibility(Visibility::Visible).expect("show");
        assert_eq!(
            scene.wakes.load(Ordering::Relaxed),
            2,
            "showing wakes a host that dropped its request"
        );
        let _ = frames(&scene.rx);
    }

    /// A hidden surface is left out of the frames a visible surface still
    /// renders: it is not drawn, its running animation does not keep the
    /// engine at `Next::At`, and its changes wake nothing while the
    /// visible surface's do.
    #[test]
    fn hidden_surface_is_left_out_of_other_surfaces_frames() {
        use std::sync::atomic::Ordering;

        use crate::{Visibility, WorkingColor};

        let (engine, rx) = engine();
        let hidden = engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
            .expect("surface");
        let visible = engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
            .expect("surface");
        let spinning = hidden.layer();
        hidden.update_animated(Spring::smooth(), |tx| {
            tx[hidden.root()].push(&spinning);
            tx[&spinning].rotation(std::f64::consts::TAU);
        });
        let t0 = Instant::now();
        let next = engine.render(FrameTime::at(t0)).expect("render");
        assert!(matches!(next, Next::At { .. }), "the spring runs: {next:?}");
        hidden.visibility(Visibility::Hidden).expect("hide");
        let _ = frames(&rx);

        let wakes = counting_waker(&engine);
        hidden.clear_color(WorkingColor::BLACK);
        assert_eq!(wakes.load(Ordering::Relaxed), 0, "the hidden surface woke");
        visible.clear_color(WorkingColor::WHITE);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "the visible surface wakes"
        );
        let next = engine
            .render(FrameTime::at(t0 + Duration::from_millis(8)))
            .expect("render");
        assert_eq!(
            next,
            Next::Idle,
            "a hidden surface's animation asks for no frame"
        );
        let records = frames(&rx);
        assert_eq!(
            records
                .iter()
                .map(|record| record.surface)
                .collect::<Vec<_>>(),
            [visible.id()],
            "only the visible surface is in the frame"
        );
        assert!(records[0].changed);
    }

    /// A hidden surface applies its changes on the render thread as they
    /// are made, in order with every release, and samples nothing until it
    /// is shown. Content committed while hidden that draws an image keeps
    /// the image alive after its last handle drops; content that stops
    /// drawing it frees it while the surface is still hidden; an animation
    /// committed while hidden starts on the frame that shows the surface,
    /// and that frame draws what was committed (#204).
    #[test]
    fn hidden_changes_apply_in_order_with_releases() {
        use crate::{Curve, Draw as _, Sampling, Visibility, WorkingColor};

        let installed = |events: &[Event], surface: SurfaceId, layer: LayerId| {
            events.iter().any(
                |event| matches!(event, Event::SetContent(s, l, _) if *s == surface && *l == layer),
            )
        };
        let removed = |events: &[Event], image: ImageId| {
            events
                .iter()
                .any(|event| matches!(event, Event::RemoveImage(id) if *id == image))
        };
        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let layer_handle = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].push(&layer_handle);
        });
        engine.render(FrameTime::now()).expect("render");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let image_id = image.id();
        let rect = kurbo::Rect::new(0., 0., 8., 8.);
        let _ = std::iter::from_fn(|| rx.try_recv().ok()).count();

        surface.visibility(Visibility::Hidden).expect("hide");
        surface.update(|tx| {
            tx[&layer_handle].record(|c| c.image(image.id(), rect, Sampling::Nearest));
        });
        drop(image);
        // The reply lands only after every earlier message was applied.
        let _ = engine.memory();
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            installed(&events, surface.id(), layer_handle.id()),
            "content committed while hidden is installed as it is committed: {events:?}"
        );
        assert!(
            !removed(&events, image_id),
            "the image content committed while hidden draws was removed: {events:?}"
        );
        assert!(
            !events.iter().any(|event| matches!(event, Event::Frame(_))),
            "a hidden surface is not drawn: {events:?}"
        );

        surface.update(|tx| {
            tx[&layer_handle].record(|c| c.fill(rect, WorkingColor::WHITE));
        });
        let _ = engine.memory();
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            removed(&events, image_id),
            "content that stops drawing the image frees it while hidden: {events:?}"
        );

        surface.update_animated(Curve::linear(Duration::from_millis(400)), |tx| {
            tx[&layer_handle].opacity(0.0f32);
        });
        let shown = Instant::now() + Duration::from_secs(10);
        surface.visibility(Visibility::Visible).expect("show");
        let next = engine
            .render(FrameTime::at(shown))
            .expect("the shown frame draws what was committed while hidden");
        assert!(
            matches!(next, Next::At { .. }),
            "the animation committed while hidden runs: {next:?}"
        );
        let records = frames(&rx);
        let [record] = records.as_slice() else {
            panic!("one frame: {records:?}");
        };
        assert!(
            (layer(record, layer_handle.id()).opacity - 1.0).abs() < f32::EPSILON,
            "an animation committed while hidden starts on the frame that shows the surface"
        );
        engine
            .render(FrameTime::at(shown + Duration::from_millis(200)))
            .expect("render");
        let records = frames(&rx);
        let [record] = records.as_slice() else {
            panic!("one frame: {records:?}");
        };
        let opacity = layer(record, layer_handle.id()).opacity;
        assert!(
            (opacity - 0.5).abs() < 1.0e-3,
            "the animation runs from the frame that showed the surface: {opacity}"
        );
    }

    #[test]
    fn layer_drop_queues_remove() {
        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
            .expect("surface");
        {
            let layer_handle = surface.layer();
            surface.update(|tx| {
                tx[surface.root()].push(&layer_handle);
            });
        }
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events.iter().any(|e| matches!(e, Event::RemoveLayer(_, _))),
            "no RemoveLayer in {events:?}"
        );
    }

    #[test]
    fn live_recorded_operands_wake_the_idle_owner_and_disconnect_on_drop() {
        use crate::{Draw, WorkingColor};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        let (engine, _events) = engine();
        let surface = engine
            .surface(crate::Offscreen::new(
                (8, 8),
                crate::OffscreenFormat::LinearF16,
            ))
            .unwrap();
        let layer = surface.layer();
        let color = nami::binding(WorkingColor::WHITE);
        surface.update(|tx| {
            tx[surface.root()].push(&layer);
            tx[&layer].content(
                surface.record(|r| r.fill(kurbo::Rect::new(0., 0., 8., 8.), color.clone())),
            );
        });
        engine.render(FrameTime::now()).unwrap();
        let count = Arc::new(AtomicU32::new(0));
        let wakes = count.clone();
        engine.set_waker(move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        });
        color.set(WorkingColor::BLACK);
        color.set(WorkingColor::WHITE);
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "live updates coalesce without host transactions"
        );
        engine.render(FrameTime::now()).unwrap();
        color.set(WorkingColor::BLACK);
        assert_eq!(
            count.load(Ordering::Relaxed),
            2,
            "render re-arms host notification"
        );
        drop(layer);
        engine.render(FrameTime::now()).unwrap();
        let before = count.load(Ordering::Relaxed);
        color.set(WorkingColor::WHITE);
        assert_eq!(
            count.load(Ordering::Relaxed),
            before,
            "removed content cannot wake the engine"
        );
    }

    /// A registration releases its backend resource when the handle drops,
    /// including a handle dropped before the backend ran its registration.
    #[test]
    fn registrations_release_on_drop() {
        let (engine, rx) = engine();
        let font = engine.font(FontSource::bytes(vec![0u8; 8])).expect("font");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("shader");
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        drop(font);
        drop(image);
        drop(shader);
        drop(surface);
        // The reply lands only after every earlier message was applied.
        let _ = engine.memory();
        balance::assert_balanced(&rx);
    }

    /// A registration the backend refused enqueues no removal: nothing
    /// ever existed to release.
    #[test]
    fn rejected_registrations_enqueue_no_removal() {
        let reject = HashSet::from([NullReject::Surface, NullReject::Image, NullReject::Shader]);
        let (engine, rx) = engine_rejecting(reject);
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("the image is queued");
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("the shader is queued");
        assert!(
            engine
                .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
                .is_err(),
            "surface"
        );
        drop(image);
        drop(shader);
        // The render is applied after both releases, and fails when a
        // strict `Null` removal of the never-added resources panicked the
        // render thread.
        engine
            .render(FrameTime::now())
            .expect("no removal reached the backend");
        balance::assert_balanced(&rx);
    }

    /// An upload the backend rejects after its handle was returned fails
    /// every render that draws it, naming the resource and the backend's
    /// reason; renders that do not draw it succeed.
    #[test]
    fn a_backend_rejected_upload_fails_the_render_that_draws_it() {
        use crate::{Draw as _, RenderError, ResourceId, Sampling, ShaderPaint};

        let (engine, _rx) =
            engine_rejecting(HashSet::from([NullReject::Image, NullReject::Shader]));
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("the image is queued");
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("the shader is queued");
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let rect = kurbo::Rect::new(0., 0., 8., 8.);
        engine
            .render(FrameTime::now())
            .expect("a render drawing neither resource succeeds");

        let assert_rejected =
            |expected: ResourceId, reason: &str| match engine.render(FrameTime::now()) {
                Err(RenderError::Rejected {
                    resource,
                    reason: actual,
                }) => {
                    assert_eq!(resource, expected);
                    assert_eq!(actual.to_string(), reason);
                }
                other => panic!("{expected} drawn: {other:?}"),
            };
        surface.update(|tx| {
            tx[surface.root()].record(|c| c.image(image.id(), rect, Sampling::Nearest));
        });
        assert_rejected(ResourceId::Image(image.id()), "image: injected rejection");
        assert_rejected(ResourceId::Image(image.id()), "image: injected rejection");
        // A replacement of a rejected image registers it anew; the backend
        // rejects that too.
        image
            .replace(ImageData::<Rgba8>::new(2, 2, vec![0u8; 16]).expect("image data"))
            .expect("the replacement is queued");
        assert_rejected(ResourceId::Image(image.id()), "image: injected rejection");

        surface.update(|tx| {
            tx[surface.root()].record(|c| {
                c.fill(
                    rect,
                    ShaderPaint {
                        shader: shader.id(),
                        uniforms: vec![],
                    },
                );
            });
        });
        assert_rejected(
            ResourceId::Shader(shader.id()),
            "shader: injected rejection",
        );

        surface.update(|tx| {
            tx[surface.root()].record(|c| c.fill(rect, crate::WorkingColor::WHITE));
        });
        engine
            .render(FrameTime::now())
            .expect("content that no longer draws a rejected resource renders");
    }

    /// The position of the first event matching `f`.
    fn position(events: &[Event], f: impl Fn(&Event) -> bool) -> Option<usize> {
        events.iter().position(f)
    }

    /// A one-glyph run in `font`.
    fn one_glyph(font: FontId) -> crate::glyph::GlyphRun {
        use crate::glyph::{Glyph, GlyphRun, GlyphStyle};
        GlyphRun {
            font,
            size: 8.0,
            coords: std::sync::Arc::new([]),
            glyphs: std::sync::Arc::new([Glyph {
                id: 1,
                x: 0.0,
                y: 8.0,
                transform: None,
            }]),
            style: GlyphStyle::Fill,
        }
    }

    /// A released resource that installed content still draws stays
    /// registered, and the render before the content changes draws it
    /// (`Null` panics on a frame drawing a removed resource). The commit
    /// that stops drawing it frees it before that commit's frame, and a
    /// layer removal frees what only that layer drew (#199).
    #[test]
    fn a_release_waits_for_installed_content() {
        use crate::{Draw as _, Sampling, ShaderPaint, WorkingColor};

        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let font = engine.font(FontSource::bytes(vec![0u8; 8])).expect("font");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("shader");
        let (font_id, image_id, shader_id) = (font.id(), image.id(), shader.id());
        let rect = kurbo::Rect::new(0., 0., 8., 8.);
        let text = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].push(&text);
            tx[surface.root()].record(|c| {
                c.image(image.id(), rect, Sampling::Nearest);
                c.fill(
                    rect,
                    ShaderPaint {
                        shader: shader.id(),
                        uniforms: vec![],
                    },
                );
            });
            tx[&text].record(|c| c.glyphs(one_glyph(font.id()), WorkingColor::WHITE));
        });
        engine.render(FrameTime::now()).expect("render");
        drop(font);
        drop(image);
        drop(shader);
        engine
            .render(FrameTime::now())
            .expect("the installed content still draws the released resources");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::RemoveFont(_) | Event::RemoveImage(_) | Event::RemoveShader(_)
            )),
            "a resource installed content draws was removed: {events:?}"
        );

        surface.update(|tx| {
            tx[surface.root()].record(|c| c.fill(rect, WorkingColor::WHITE));
        });
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let installed = position(&events, |e| matches!(e, Event::SetContent(..)))
            .expect("the new content was installed");
        let frame = position(&events, |e| matches!(e, Event::Frame(_))).expect("frame");
        for (removal, name) in [
            (
                position(
                    &events,
                    |e| matches!(e, Event::RemoveImage(id) if *id == image_id),
                ),
                "image",
            ),
            (
                position(
                    &events,
                    |e| matches!(e, Event::RemoveShader(id) if *id == shader_id),
                ),
                "shader",
            ),
        ] {
            let removal = removal.unwrap_or_else(|| panic!("no {name} removal in {events:?}"));
            assert!(
                installed < removal && removal < frame,
                "the {name} is removed between installing the content that stopped drawing it and that frame: {events:?}"
            );
        }
        assert!(
            !events.iter().any(|e| matches!(e, Event::RemoveFont(_))),
            "the font the text layer still draws was removed: {events:?}"
        );

        drop(text);
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let removed = position(&events, |e| matches!(e, Event::RemoveLayer(..)))
            .expect("the text layer was removed");
        let freed = position(
            &events,
            |e| matches!(e, Event::RemoveFont(id) if *id == font_id),
        )
        .unwrap_or_else(|| panic!("no font removal in {events:?}"));
        let frame = position(&events, |e| matches!(e, Event::Frame(_))).expect("frame");
        assert!(
            removed < freed && freed < frame,
            "the font is removed with the layer that drew it, before the frame: {events:?}"
        );
    }

    /// Content installed after the handle dropped that names a pending
    /// resource keeps it alive: a kept recording on a second surface still
    /// draws the image after the first surface stops drawing it, and the
    /// removal lands only once the second surface stops too (#199).
    #[test]
    fn a_kept_recording_keeps_a_pending_resource_alive() {
        use crate::{Draw as _, Sampling, WorkingColor};

        let (engine, rx) = engine();
        let first = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let second = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let rect = kurbo::Rect::new(0., 0., 8., 8.);
        first.update(|tx| {
            tx[first.root()].record(|c| c.image(image.id(), rect, Sampling::Nearest));
        });
        let kept = second.record(|c| c.image(image.id(), rect, Sampling::Nearest));
        engine.render(FrameTime::now()).expect("render");
        drop(image);
        engine.render(FrameTime::now()).expect("render");
        second.update(|tx| {
            tx[second.root()].content(kept);
        });
        engine.render(FrameTime::now()).expect("render");
        first.update(|tx| {
            tx[first.root()].record(|c| c.fill(rect, WorkingColor::WHITE));
        });
        engine
            .render(FrameTime::now())
            .expect("the second surface still draws the image");
        let removed = |events: &[Event]| events.iter().any(|e| matches!(e, Event::RemoveImage(_)));
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !removed(&events),
            "the image was removed while the second surface draws it: {events:?}"
        );

        second.update(|tx| {
            tx[second.root()].record(|c| c.fill(rect, WorkingColor::WHITE));
        });
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            removed(&events),
            "no image removal once no surface draws it: {events:?}"
        );
    }

    /// Dropping the only surface whose installed content draws a released
    /// resource carries out its pending release (#199).
    #[test]
    fn pending_releases_run_when_the_surface_drops() {
        use crate::{Draw as _, Sampling};

        let (engine, rx) = engine();
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        surface.update(|tx| {
            tx[surface.root()].record(|c| {
                c.image(
                    image.id(),
                    kurbo::Rect::new(0., 0., 8., 8.),
                    Sampling::Nearest,
                );
            });
        });
        engine.render(FrameTime::now()).expect("render");
        drop(image);
        engine.render(FrameTime::now()).expect("render");
        drop(surface);
        engine.render(FrameTime::now()).expect("render");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let destroyed = position(&events, |e| matches!(e, Event::DestroySurface(_)))
            .unwrap_or_else(|| panic!("no surface destruction in {events:?}"));
        let freed = position(&events, |e| matches!(e, Event::RemoveImage(_)))
            .unwrap_or_else(|| panic!("no image removal in {events:?}"));
        assert!(
            destroyed < freed,
            "the image outlives the surface that drew it: {events:?}"
        );
        let (mut commits, mut releases) = balance::transitions(&events);
        commits.sort();
        releases.sort();
        assert_eq!(
            commits, releases,
            "unbalanced registration events: {events:?}"
        );
    }

    #[test]
    fn retired_live_content_does_not_update_or_wake() {
        use crate::{Draw, WorkingColor};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let (engine, rx) = engine();
        let surface = engine
            .surface(crate::Offscreen::new(
                (8, 8),
                crate::OffscreenFormat::LinearF16,
            ))
            .unwrap();
        let layer = surface.layer();
        let color = nami::binding(WorkingColor::WHITE);
        surface.update(|tx| {
            tx[&layer].content(
                surface.record(|r| r.fill(kurbo::Rect::new(0., 0., 8., 8.), color.clone())),
            );
        });
        engine.render(FrameTime::now()).unwrap();
        let _ = frames(&rx);

        let count = Arc::new(AtomicU32::new(0));
        let wakes = Arc::clone(&count);
        engine.set_waker(move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        });
        surface.update(|tx| {
            tx[&layer].record(|r| {
                r.fill(kurbo::Rect::new(0., 0., 8., 8.), WorkingColor::BLACK);
            });
        });
        engine.render(FrameTime::now()).unwrap();
        let _ = frames(&rx);

        let before = count.load(Ordering::Relaxed);
        color.set(WorkingColor::BLACK);
        assert_eq!(
            count.load(Ordering::Relaxed),
            before,
            "retired content cannot wake the engine"
        );

        engine.render(FrameTime::now()).unwrap();
        let record = frames(&rx).pop().expect("frame");
        assert!(!record.changed, "retired content cannot queue updates");
    }

    /// A `Surface::display` update on a surface that does not present
    /// never marks `present_pending`, while a window-like target in the
    /// same engine does: headroom and pending-present state belong only
    /// to surfaces that present (#98). Before the confinement, a display
    /// update on an `Offscreen` surface left `present_pending` set,
    /// which a presenting backend consumed as an unhandled case.
    #[test]
    fn display_updates_mark_present_only_on_presenting_surfaces() {
        use crate::Display;

        let (engine, rx) = engine();
        let offscreen = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("offscreen surface");
        let window = engine
            .surface(NullTarget::Window(Offscreen::new(
                (8, 8),
                OffscreenFormat::LinearF16,
            )))
            .expect("window-like surface");
        engine.render(FrameTime::now()).expect("render");
        let _ = frames(&rx);

        offscreen
            .display(Display {
                scale: 1.0,
                headroom: 4.0,
            })
            .expect("display");
        window
            .display(Display {
                scale: 1.0,
                headroom: 4.0,
            })
            .expect("display");
        engine.render(FrameTime::now()).expect("render");
        let records = frames(&rx);
        let offscreen_record = records
            .iter()
            .find(|record| record.surface == offscreen.id())
            .expect("offscreen frame record");
        let window_record = records
            .iter()
            .find(|record| record.surface == window.id())
            .expect("window frame record");
        assert!(
            !offscreen_record.present_pending,
            "a non-presenting surface cannot be pending a present"
        );
        assert!(
            window_record.present_pending,
            "a presenting surface is pending after a headroom update"
        );
        // The display value itself lands on both: only the pending
        // present is confined.
        assert_eq!(
            offscreen_record.display.headroom.to_bits(),
            4.0f32.to_bits()
        );
        assert_eq!(window_record.display.headroom.to_bits(), 4.0f32.to_bits());
    }

    /// A headroom-only `Surface::display` sequence (4 → 2 → 1 → 4, the
    /// corpus's headroom sequence) marks `present_pending` on every frame
    /// — never `changed`, so no scene re-generation or local-cache work —
    /// and reaches the backend with the new headroom. A scale change
    /// still marks `changed` (#98 C4).
    #[test]
    fn headroom_updates_present_without_regenerating_content() {
        use crate::{Display, Draw, WorkingColor};

        let (engine, rx) = engine();
        let surface = engine
            .surface(NullTarget::Window(Offscreen::new(
                (8, 8),
                OffscreenFormat::LinearF16,
            )))
            .expect("surface");
        let layer = surface.layer();
        let content =
            surface.record(|c| c.fill(kurbo::Rect::new(0., 0., 8., 8.), WorkingColor::WHITE));
        surface.update(|tx| {
            tx[surface.root()].push(&layer);
            tx[&layer].content(content);
        });
        engine.render(FrameTime::now()).expect("render");
        let _ = frames(&rx);

        for headroom in [4.0f32, 2.0, 1.0, 4.0] {
            surface
                .display(Display {
                    scale: 1.0,
                    headroom,
                })
                .expect("display");
            engine.render(FrameTime::now()).expect("render");
            let record = frames(&rx).pop().expect("frame record");
            assert!(
                !record.changed,
                "a headroom-only update must not mark changed at headroom {headroom}"
            );
            assert!(
                record.present_pending,
                "a headroom update must mark presentation pending at headroom {headroom}"
            );
            assert_eq!(
                record.display.headroom.to_bits(),
                headroom.to_bits(),
                "the frame must carry the new headroom"
            );
            let stats = engine.stats();
            assert_eq!(
                stats.commands_lowered, 0,
                "a headroom-only update regenerates no scene content"
            );
        }

        surface
            .display(Display {
                scale: 2.0,
                headroom: 4.0,
            })
            .expect("display");
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("frame record");
        assert!(
            record.changed && record.present_pending,
            "a scale change is content, not presentation-only"
        );
    }

    /// `Surface::display_moved` rides to the frame as `display_moved`
    /// and marks a present on a presenting surface; a headroom-only
    /// `display` update never sets it, and the flag is consumed by one
    /// frame (#98).
    #[test]
    fn display_moves_reach_the_frame_once() {
        use crate::Display;

        let (engine, rx) = engine();
        let surface = engine
            .surface(NullTarget::Window(Offscreen::new(
                (8, 8),
                OffscreenFormat::LinearF16,
            )))
            .expect("surface");
        engine.render(FrameTime::now()).expect("render");
        let _ = frames(&rx);

        surface.display_moved().expect("display move");
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("frame record");
        assert!(
            record.display_moved && record.present_pending,
            "a display move re-enumerates and presents"
        );

        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("frame record");
        assert!(
            !record.display_moved,
            "the move flag is consumed by one frame"
        );

        surface
            .display(Display {
                scale: 1.0,
                headroom: 3.0,
            })
            .expect("display");
        engine.render(FrameTime::now()).expect("render");
        let record = frames(&rx).pop().expect("frame record");
        assert!(
            !record.display_moved,
            "a headroom-only update never moves the display"
        );
        assert!(record.present_pending);
    }
}

/// Retained-lowering equivalence checks for first-party backends.
pub mod incremental;

/// The browser executor's registration lifetimes (#150). Resource
/// registration is synchronous and queued in order, so its handle owns the
/// id from the start; surface creation is a future that owns its id from
/// the moment the request is enqueued, so dropping it at any point still
/// destroys what the backend committed. Run under the Node
/// `wasm-bindgen-test-runner` (or a browser):
///
/// ```console
/// CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
/// cargo test -p cherenkov --features testing --lib \
///     --target wasm32-unknown-unknown
/// ```
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use std::collections::HashSet;
    use std::fmt::Debug;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::mpsc::Receiver;
    use std::task::{Context, Waker};

    use wasm_bindgen_test::wasm_bindgen_test;

    use super::balance;
    use super::{Event, Null, NullConfig, NullReject};
    use crate::image::ImageData;
    use crate::resource::FontSource;
    use crate::{
        Draw as _, Engine, FrameTime, Offscreen, OffscreenFormat, RenderError, ResourceId, Rgba8,
        Sampling, ShaderPaint, ShaderSource, WorkingColor,
    };

    #[expect(
        clippy::future_not_send,
        reason = "the engine is main-thread on wasm, so its futures are !Send by design"
    )]
    async fn engine(reject: HashSet<NullReject>) -> (Engine<Null>, Receiver<Event>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events: tx,
            reject,
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .await
        .expect("init");
        (engine, rx)
    }

    /// Runs the creation future until it has enqueued its request and is
    /// parked on the reply.
    fn enqueued<F: Future>(creation: Pin<&mut F>) {
        assert!(
            creation
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending(),
            "creation returned before its reply await"
        );
    }

    /// Everything enqueued before this call has been applied: the
    /// `Memory` reply lands after them in the serial executor.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn flush(engine: &Engine<Null>) {
        let _ = engine.memory().await;
    }

    /// Window (a): dropped after enqueue, before the backend ran.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn drop_after_enqueue<F, O, E>(engine: &Engine<Null>, create: F)
    where
        F: Future<Output = Result<O, E>>,
        E: Debug,
    {
        let mut creation = Box::pin(create);
        enqueued(creation.as_mut());
        drop(creation);
        flush(engine).await;
    }

    /// Window (b): dropped after the backend committed, before the reply
    /// was adopted. The first `flush` returns once the commit is done.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn drop_after_commit<F, O, E>(engine: &Engine<Null>, create: F)
    where
        F: Future<Output = Result<O, E>>,
        E: Debug,
    {
        let mut creation = Box::pin(create);
        enqueued(creation.as_mut());
        flush(engine).await;
        drop(creation);
        flush(engine).await;
    }

    /// Window (c): the reply was adopted and the handle dropped.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn drop_after_reply<F, O, E>(engine: &Engine<Null>, create: F)
    where
        F: Future<Output = Result<O, E>>,
        E: Debug,
    {
        drop(create.await.expect("creation"));
        flush(engine).await;
    }

    #[wasm_bindgen_test]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn dropped_surface_creation_still_destroys_it() {
        let (engine, rx) = engine(HashSet::default()).await;
        let create = || engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16));
        drop_after_enqueue(&engine, create()).await;
        drop_after_commit(&engine, create()).await;
        drop_after_reply(&engine, create()).await;
        balance::assert_balanced(&rx);
    }

    /// The browser executor shares the hidden-surface contract: a hidden
    /// surface's transaction wakes no host and is applied as it is made,
    /// undrawn, rendering while it is the only surface fails with
    /// `Hidden`, showing it wakes the host exactly once, and the next frame
    /// draws what was committed while it was hidden.
    #[wasm_bindgen_test]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn hidden_surface_schedules_no_frames() {
        use std::cell::Cell;
        use std::rc::Rc;

        use crate::{Next, Visibility};

        let (engine, rx) = engine(HashSet::default()).await;
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .await
            .expect("surface");
        let layer = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].push(&layer);
        });
        engine.render(FrameTime::now()).await.expect("render");
        let _: Vec<Event> = rx.try_iter().collect();

        let wakes = Rc::new(Cell::new(0u32));
        engine.set_waker({
            let wakes = Rc::clone(&wakes);
            move || wakes.set(wakes.get() + 1)
        });
        surface.visibility(Visibility::Hidden).expect("hide");
        surface.update(|tx| {
            tx[&layer].record(|c| c.fill(kurbo::Rect::new(0., 0., 8., 8.), WorkingColor::WHITE));
        });
        flush(&engine).await;
        assert_eq!(wakes.get(), 0, "a hidden surface woke the host");
        assert!(
            matches!(
                engine.render(FrameTime::now()).await,
                Err(RenderError::Hidden)
            ),
            "rendering while every surface is hidden is an error"
        );
        let events: Vec<Event> = rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [
                    Event::Visibility(id, Visibility::Hidden),
                    Event::SetContent(set, layer_id, ..),
                ] if *id == surface.id() && *set == surface.id() && *layer_id == layer.id()
            ),
            "the hidden surface's transaction is applied as it is made, undrawn: {events:?}"
        );

        surface.visibility(Visibility::Visible).expect("show");
        assert_eq!(wakes.get(), 1, "showing the surface asks for one frame");
        assert_eq!(
            engine.render(FrameTime::now()).await.expect("render"),
            Next::Idle
        );
        let events: Vec<Event> = rx.try_iter().collect();
        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::Frame(record) if record.surface == surface.id() && record.changed
            )),
            "the shown surface redraws: {events:?}"
        );
    }

    /// A registration releases its backend resource when the handle
    /// drops, including a handle dropped before the executor ran its
    /// registration.
    #[wasm_bindgen_test]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn registrations_release_on_drop() {
        let (engine, rx) = engine(HashSet::default()).await;
        let font = engine.font(FontSource::bytes(vec![0u8; 8])).expect("font");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("shader");
        drop(font);
        drop(image);
        drop(shader);
        flush(&engine).await;
        balance::assert_balanced(&rx);
    }

    /// A registration the backend refused enqueues no removal — the
    /// resource never existed — whether the caller awaited the error or
    /// dropped the future on the way.
    #[wasm_bindgen_test]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn rejected_registrations_enqueue_no_removal() {
        let reject = HashSet::from([NullReject::Surface, NullReject::Image, NullReject::Shader]);
        let (engine, rx) = engine(reject).await;
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("the image is queued");
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("the shader is queued");
        assert!(
            engine
                .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
                .await
                .is_err(),
            "surface"
        );
        drop(image);
        drop(shader);
        // Dropped before the rejected reply: the enqueued destruction finds
        // no committed surface and reports nothing.
        drop_after_enqueue(
            &engine,
            engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)),
        )
        .await;
        flush(&engine).await;
        balance::assert_balanced(&rx);
    }

    /// An upload the backend rejects after its handle was returned fails
    /// the render that draws it, naming the resource and the backend's
    /// reason. Registration and recording happen with no await between.
    #[wasm_bindgen_test]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn a_backend_rejected_upload_fails_the_render_that_draws_it() {
        let reject = HashSet::from([NullReject::Image, NullReject::Shader]);
        let (engine, _rx) = engine(reject).await;
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .await
            .expect("surface");
        let rect = kurbo::Rect::new(0., 0., 8., 8.);
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("the image is queued");
        surface.update(|tx| {
            tx[surface.root()].record(|c| c.image(image.id(), rect, Sampling::Nearest));
        });
        match engine.render(FrameTime::now()).await {
            Err(RenderError::Rejected { resource, reason }) => {
                assert_eq!(resource, ResourceId::Image(image.id()));
                assert_eq!(reason.to_string(), "image: injected rejection");
            }
            other => panic!("rejected image drawn: {other:?}"),
        }
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("the shader is queued");
        surface.update(|tx| {
            tx[surface.root()].record(|c| {
                c.fill(
                    rect,
                    ShaderPaint {
                        shader: shader.id(),
                        uniforms: vec![],
                    },
                );
            });
        });
        match engine.render(FrameTime::now()).await {
            Err(RenderError::Rejected { resource, reason }) => {
                assert_eq!(resource, ResourceId::Shader(shader.id()));
                assert_eq!(reason.to_string(), "shader: injected rejection");
            }
            other => panic!("rejected shader drawn: {other:?}"),
        }
    }

    /// Dropping the only surface whose installed content draws a released
    /// resource carries out its pending release, after the surface is
    /// destroyed (#199).
    #[wasm_bindgen_test]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn pending_releases_run_when_the_surface_drops() {
        let (engine, rx) = engine(HashSet::default()).await;
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .await
            .expect("surface");
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        surface.update(|tx| {
            tx[surface.root()].record(|c| {
                c.image(
                    image.id(),
                    kurbo::Rect::new(0., 0., 8., 8.),
                    Sampling::Nearest,
                );
            });
        });
        engine.render(FrameTime::now()).await.expect("render");
        drop(image);
        engine.render(FrameTime::now()).await.expect("render");
        drop(surface);
        engine.render(FrameTime::now()).await.expect("render");
        let events: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let destroyed = events
            .iter()
            .position(|e| matches!(e, Event::DestroySurface(_)))
            .unwrap_or_else(|| panic!("no surface destruction in {events:?}"));
        let freed = events
            .iter()
            .position(|e| matches!(e, Event::RemoveImage(_)))
            .unwrap_or_else(|| panic!("no image removal in {events:?}"));
        assert!(
            destroyed < freed,
            "the image outlives the surface that drew it: {events:?}"
        );
        let (mut commits, mut releases) = balance::transitions(&events);
        commits.sort();
        releases.sort();
        assert_eq!(
            commits, releases,
            "unbalanced registration events: {events:?}"
        );
    }

    /// A released resource that installed content still draws stays
    /// registered: the render that still draws it succeeds (`Null` panics
    /// on a frame drawing a removed resource). The release is carried out
    /// when the content stops drawing it, or when the only surface drawing
    /// it drops (#199).
    #[wasm_bindgen_test]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn a_release_waits_for_installed_content() {
        let (engine, rx) = engine(HashSet::default()).await;
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .await
            .expect("surface");
        let other = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .await
            .expect("surface");
        let rect = kurbo::Rect::new(0., 0., 8., 8.);
        let image = engine
            .image(ImageData::<Rgba8>::new(1, 1, vec![0u8; 4]).expect("image data"))
            .expect("image");
        let shader = engine
            .shader(ShaderSource::wgsl("@fragment fn f() { }"))
            .expect("shader");
        surface.update(|tx| {
            tx[surface.root()].record(|c| c.image(image.id(), rect, Sampling::Nearest));
        });
        other.update(|tx| {
            tx[other.root()].record(|c| {
                c.fill(
                    rect,
                    ShaderPaint {
                        shader: shader.id(),
                        uniforms: vec![],
                    },
                );
            });
        });
        engine.render(FrameTime::now()).await.expect("render");
        drop(image);
        drop(shader);
        engine
            .render(FrameTime::now())
            .await
            .expect("the installed content still draws the released resources");
        let removals = || {
            let events: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            (
                events.iter().any(|e| matches!(e, Event::RemoveImage(_))),
                events.iter().any(|e| matches!(e, Event::RemoveShader(_))),
            )
        };
        assert_eq!(removals(), (false, false), "nothing is freed while drawn");

        surface.update(|tx| {
            tx[surface.root()].record(|c| c.fill(rect, WorkingColor::WHITE));
        });
        engine.render(FrameTime::now()).await.expect("render");
        assert_eq!(
            removals(),
            (true, false),
            "the image is freed with the content that drew it"
        );

        drop(other);
        flush(&engine).await;
        assert_eq!(
            removals(),
            (false, true),
            "the shader is freed with the surface that drew it"
        );
    }
}

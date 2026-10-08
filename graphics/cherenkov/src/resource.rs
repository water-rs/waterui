//! Resource handles: [`Font`], [`Image`], [`Shader`] and [`Filter`] are
//! `Clone` over an `Rc`; the last drop queues the release on the render
//! thread. A font, image, shader or backdrop shader is freed there once no
//! surface's installed content draws it. An [`Image`] also queues in-place
//! pixel replacements.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use crate::ShaderId;
use crate::error::ResourceError;
use crate::glyph::FontId;
use crate::image::{Format, ImageData, ImageUpload};
use cherenkov_record::{
    BackdropEffect, BackdropId, BackdropSample, BackdropShaderId, LayerContent,
};

use crate::paint::ImageId;
use crate::style::FilterId;

/// The data of a font to register with the engine.
#[derive(Clone)]
pub struct FontSource {
    /// The raw font data.
    pub data: Arc<[u8]>,
    /// The font index inside a collection.
    pub index: u32,
}

impl std::fmt::Debug for FontSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontSource")
            .field("len", &self.data.len())
            .field("index", &self.index)
            .finish()
    }
}

impl FontSource {
    /// A font already in memory.
    pub fn bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            data: bytes.into(),
            index: 0,
        }
    }

    /// A font read from a file.
    ///
    /// This reads the whole file into memory; it does not memory-map yet,
    /// so very large fonts are copied once.
    ///
    /// # Errors
    /// Any [`std::io::Error`] from reading the file.
    pub fn mapped(path: impl AsRef<Path>) -> std::io::Result<Self> {
        std::fs::read(path).map(Self::bytes)
    }

    /// Selects a font index inside a collection.
    #[must_use]
    pub fn with_index(self, index: u32) -> Self {
        Self { index, ..self }
    }
}

/// Queues a replacement of an image's pixels, ordered with every other
/// message the engine sends.
pub type ReplaceImage = Rc<dyn Fn(ImageId, ImageUpload) -> Result<(), ResourceError>>;

/// The shared state of a resource handle: the last `Rc` drop runs
/// `on_drop`, which queues the resource's `remove_*` op. `ops` carries the
/// render-thread operations a kind queues while it lives (an image's
/// replacement); kinds without any use `()`.
struct Inner<I, O = ()> {
    id: I,
    ops: O,
    on_drop: Option<Box<dyn FnOnce()>>,
}

impl<I, O> std::fmt::Debug for Inner<I, O>
where
    I: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<I, O> Drop for Inner<I, O> {
    fn drop(&mut self) {
        if let Some(on_drop) = self.on_drop.take() {
            on_drop();
        }
    }
}

fn handle<I>(id: I, on_drop: impl FnOnce() + 'static) -> Rc<Inner<I>> {
    handle_with(id, (), on_drop)
}

fn handle_with<I, O>(id: I, ops: O, on_drop: impl FnOnce() + 'static) -> Rc<Inner<I, O>> {
    Rc::new(Inner {
        id,
        ops,
        on_drop: Some(Box::new(on_drop)),
    })
}

/// A font registered with an engine. Dropping the last clone unregisters
/// the font once no installed content draws it.
#[derive(Debug)]
pub struct Font {
    inner: Rc<Inner<FontId>>,
}

impl Clone for Font {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl Font {
    pub(crate) fn new(id: FontId, on_drop: impl FnOnce() + 'static) -> Self {
        Self {
            inner: handle(id, on_drop),
        }
    }

    /// The identifier glyph runs reference.
    #[must_use]
    pub fn id(&self) -> FontId {
        self.inner.id
    }
}

/// An image registered with an engine, typed by its storage [`Format`].
///
/// [`Image::replace`] swaps its pixels behind the same id. Dropping the last
/// clone unregisters the image once no installed content draws it.
#[derive(Debug)]
pub struct Image<F: Format> {
    inner: Rc<Inner<ImageId, ReplaceImage>>,
    format: std::marker::PhantomData<F>,
}

impl<F: Format> Clone for Image<F> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
            format: std::marker::PhantomData,
        }
    }
}

impl<F: Format> Image<F> {
    pub(crate) fn new(
        id: ImageId,
        replace: ReplaceImage,
        on_drop: impl FnOnce() + 'static,
    ) -> Self {
        Self {
            inner: handle_with(id, replace, on_drop),
            format: std::marker::PhantomData,
        }
    }

    /// The identifier image draws and image paints reference.
    #[must_use]
    pub fn id(&self) -> ImageId {
        self.inner.id
    }

    /// Replaces the image's pixels in place. The replacement is queued
    /// and applied in order with every render: it does not wait for the
    /// backend.
    ///
    /// The id is unchanged, so every recording that names this image draws
    /// the new pixels from the next frame on, without re-recording. No
    /// frame samples a partly written image. The same dimensions reuse the
    /// backing storage; different dimensions reallocate it behind the same
    /// id. The next render redraws the surfaces whose content draws this
    /// image, and each of those surfaces wakes its host to request that
    /// render.
    ///
    /// `image` is validated by [`ImageData::new`], and its size is checked
    /// against the engine's [`ImageLimits`](crate::ImageLimits) on the
    /// calling thread: an image the device cannot hold fails here with
    /// [`ResourceError::TooLarge`] and keeps the previous pixels. A
    /// rejection only the backend can detect keeps the previous pixels
    /// too, and fails every render that draws the image with
    /// [`RenderError::Rejected`], until a later replacement succeeds, as
    /// for [`Engine::image`](crate::Engine::image).
    ///
    /// # Errors
    /// [`ResourceError::TooLarge`] when the image exceeds the engine's
    /// image limits, [`ResourceError::Lost`] when the render thread is
    /// gone.
    ///
    /// [`RenderError::Rejected`]: crate::RenderError::Rejected
    pub fn replace(&self, image: ImageData<F>) -> Result<(), ResourceError> {
        (self.inner.ops)(self.inner.id, image.into_upload())
    }
}

/// A shader registered with an engine. Dropping the last clone unregisters
/// the shader once no installed content draws it.
#[derive(Debug)]
pub struct Shader {
    inner: Rc<Inner<ShaderId>>,
}

impl Clone for Shader {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl Shader {
    pub(crate) fn new(id: ShaderId, on_drop: impl FnOnce() + 'static) -> Self {
        Self {
            inner: handle(id, on_drop),
        }
    }

    /// The identifier [`ShaderPaint`](crate::paint::ShaderPaint) references.
    #[must_use]
    pub fn id(&self) -> ShaderId {
        self.inner.id
    }
}

/// A filter or effect registered with an engine. Dropping the last clone
/// unregisters it.
#[derive(Debug)]
pub struct Filter {
    inner: Rc<Inner<FilterId>>,
}

impl Clone for Filter {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl Filter {
    pub(crate) fn new(id: FilterId, on_drop: impl FnOnce() + 'static) -> Self {
        Self {
            inner: handle(id, on_drop),
        }
    }

    /// The identifier [`LayerEdit::filter`](crate::LayerEdit::filter)
    /// references.
    #[must_use]
    pub fn id(&self) -> FilterId {
        self.inner.id
    }
}

/// A backdrop group: one capture and one spatial filter chain shared by
/// its members. `!Send`; dropping it unregisters the group, and a member
/// still sampling it makes the frame fail.
#[derive(Debug)]
pub struct BackdropGroup {
    inner: Rc<Inner<BackdropId>>,
}

impl BackdropGroup {
    pub(crate) fn new(id: BackdropId, on_drop: impl FnOnce() + 'static) -> Self {
        Self {
            inner: handle(id, on_drop),
        }
    }

    /// The group's identifier.
    #[must_use]
    pub fn id(&self) -> BackdropId {
        self.inner.id
    }

    /// A sample of this group for [`LayerEdit::backdrop`](crate::LayerEdit::backdrop).
    #[must_use]
    pub fn sample(&self) -> BackdropSample {
        BackdropSample::new(self.id())
    }

    /// A sample of this group with a per-member effect, evaluated in the
    /// member's composite against the shared filtered capture.
    #[must_use]
    pub fn sample_with(&self, effect: impl Into<BackdropEffect>) -> BackdropSample {
        BackdropSample::with_effect(self.id(), effect)
    }
}

/// A backdrop effect shader registered with an engine.
///
/// Made by [`Engine::backdrop_shader`](crate::Engine::backdrop_shader).
/// Dropping the last clone unregisters it once no layer samples it.
#[derive(Debug)]
pub struct BackdropShader {
    inner: Rc<Inner<BackdropShaderId>>,
    reach: f32,
}

impl Clone for BackdropShader {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
            reach: self.reach,
        }
    }
}

impl BackdropShader {
    pub(crate) fn new(id: BackdropShaderId, reach: f32, on_drop: impl FnOnce() + 'static) -> Self {
        Self {
            inner: handle(id, on_drop),
            reach,
        }
    }

    /// The identifier [`BackdropShaderEffect`](crate::BackdropShaderEffect) references.
    #[must_use]
    pub fn id(&self) -> BackdropShaderId {
        self.inner.id
    }

    /// A [`BackdropEffect::Shader`] for
    /// [`BackdropGroup::sample_with`], with `uniforms` in the shader's
    /// declared order (at most 64 finite values, packed four per `vec4`).
    #[must_use]
    pub fn effect(&self, uniforms: Vec<f32>) -> crate::BackdropShaderEffect {
        crate::BackdropShaderEffect::new(self.id(), uniforms, self.reach)
    }
}

/// The channel a [`FrameSink`] submits through — the engine's bounded
/// transaction stream, so a decoder thread's submits keep backpressure.
#[cfg(not(target_arch = "wasm32"))]
type ProducerChannel<B> = crossbeam_channel::Sender<crate::message::Message<B>>;
#[cfg(target_arch = "wasm32")]
type ProducerChannel<B> = crate::local::Sender<crate::message::Message<B>>;

/// The channel a [`GpuProducer`]'s last drop posts its retirement
/// through. A [`ProducerShared`] drop runs on any thread — including
/// the render thread when a binding's last handle dies inside `unbind`,
/// surface destroy or `drain_gpu_producers` — so it can never ride the
/// bounded transaction channel: a render thread blocking on `send`
/// waits on a channel it alone drains. On native the retirement is an
/// unbounded channel the render loop waits on alongside the transaction
/// stream — a retirement wakes the loop by itself — and still drains
/// after each applied batch; on wasm the local sender is already
/// unbounded. Ordering is safe: every pending bind closure holds a
/// clone, so the last drop follows every bind of the producer, and the
/// renderer records a retirement that beats the producer's registration
/// to the stream.
#[cfg(not(target_arch = "wasm32"))]
type RetireChannel<B> = crossbeam_channel::Sender<crate::message::ResOp<B>>;
#[cfg(target_arch = "wasm32")]
type RetireChannel<B> = crate::local::Sender<crate::message::Message<B>>;

/// What a [`GpuProducer`] shares with the copies its binding holds on the
/// render thread: the last `Arc` drop — handle or binding — retires the
/// producer through the retirement queue.
struct ProducerShared<B: crate::GpuContent> {
    id: crate::message::ProducerId,
    retire: RetireChannel<B>,
}

impl<B: crate::GpuContent> Drop for ProducerShared<B> {
    fn drop(&mut self) {
        let id = self.id;
        let op: crate::message::ResOp<B> = Box::new(move |r: &mut B::Renderer| {
            B::retire_gpu_producer(r, id);
        });
        // Unbounded, so the send cannot block the render thread it may
        // run on. A lost render loop leaves nothing to retire.
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.retire.send(op);
        #[cfg(target_arch = "wasm32")]
        let _ = self.retire.send(crate::message::Message::Resource(op));
    }
}

/// A GPU producer shared across the surfaces of one engine, created by
/// [`Engine::gpu_producer`](crate::Engine::gpu_producer).
///
/// The one view instance owns a handle; `Clone`s share the producer's
/// renderer state. [`at`](Self::at) binds it to a layer at the pixel size
/// that layer needs — any number of the engine's surfaces may bind it, a
/// persistent one and a transient capture target alike; the layout, clip
/// and transform stay the layer's own. There is no cache keyed by content:
/// a second `gpu_producer` call is a second producer.
///
/// The renderer draws the producer's frame at most once per frame, sized
/// to the componentwise largest frame its drawn bindings requested, into
/// the surface compositor's ring — the ring buffer is the producer's
/// current frame, and every binding samples `ImageSource::Content` of the
/// producer's [`id`](Self::id). Dropping the last handle retires the
/// producer through its own unbounded queue — a last drop on the render
/// thread never blocks on the bounded transaction channel.
pub struct GpuProducer<B: crate::GpuContent> {
    shared: Arc<ProducerShared<B>>,
}

impl<B: crate::GpuContent> std::fmt::Debug for GpuProducer<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuProducer")
            .field("id", &self.id())
            .finish()
    }
}

impl<B: crate::GpuContent> Clone for GpuProducer<B> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<B: crate::GpuContent> GpuProducer<B> {
    pub(crate) fn new(id: crate::message::ProducerId, retire: RetireChannel<B>) -> Self {
        Self {
            shared: Arc::new(ProducerShared { id, retire }),
        }
    }

    /// The producer's identifier: what every binding's
    /// `ImageSource::Content` names.
    #[must_use]
    pub fn id(&self) -> crate::message::ProducerId {
        self.shared.id
    }

    /// Binds the producer to a layer at `size` pixels — the pixel size
    /// that layer needs. Returns the layer content
    /// [`LayerEdit::content`](crate::LayerEdit::content) installs, the
    /// sealed [`LayerContent::install`] payload naming this producer at
    /// `size`; the `GpuInstalls` bound `GpuContent` carries is what keeps
    /// a target without GPU installs — the engine-free consumer — from
    /// ever requesting one. Binding the layer again is a new binding.
    ///
    /// # Panics
    /// At apply time, when the producer is bound on an engine other than
    /// the one that made it.
    #[must_use]
    pub fn at(&self, size: (u32, u32)) -> cherenkov_record::LayerContent<B> {
        let producer = self.clone();
        let install: crate::message::InstallOp<B> = Box::new(move |r, surface, layer| {
            B::bind_gpu_producer(r, surface, layer, &producer, size)
        });
        LayerContent::install(install)
    }
}

/// A system layer the host supplies, shown in the layer tree on a
/// system-compositor plane of its own ([`HostedLayers`](crate::HostedLayers)).
///
/// The host keeps the platform object's owner — on Apple the view that
/// owns the `CALayer` stays in the host's view hierarchy, so it keeps
/// receiving events and first-responder status — and goes on drawing into
/// it; the engine owns only where it sits: its order among the layers
/// painted below and above it, and the transform, clip and scroll of its
/// path, committed with the rest of the frame.
pub struct Hosted<B: crate::HostedLayers> {
    object: B::Object,
}

impl<B: crate::HostedLayers> std::fmt::Debug for Hosted<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `B::Object` is a platform object, not required to be `Debug`.
        f.write_str("Hosted(..)")
    }
}

impl<B: crate::HostedLayers> Hosted<B> {
    /// Wraps the platform object.
    #[must_use]
    pub const fn new(object: B::Object) -> Self {
        Self { object }
    }

    /// Binds the hosted object to a layer with `size` as its extent in the
    /// layer's content coordinates — the object's own coordinates, so a
    /// `CALayer`'s points or a `SurfaceControl`'s pixels are the layer's
    /// content units. Returns the layer content
    /// [`LayerEdit::content`](crate::LayerEdit::content) installs.
    ///
    /// The object shows in one place: binding it again on the same layer
    /// at a new size moves only its geometry, and binding it on a second
    /// layer moves it there. Its pixels are the host's, so the layer is
    /// never known to be opaque.
    ///
    /// # Panics
    /// When `size` is not finite or has a negative side.
    #[must_use]
    pub fn at(&self, size: kurbo::Size) -> LayerContent<B> {
        assert!(
            size.is_finite() && size.width >= 0.0 && size.height >= 0.0,
            "a hosted extent is a finite, non-negative size, not {size:?}"
        );
        let object = self.object.clone();
        let install: crate::message::InstallOp<B> = Box::new(move |r, surface, layer| {
            B::bind_hosted(r, surface, layer, object, size);
            Some(false)
        });
        LayerContent::install(install)
    }
}

/// The input end of a submitted-frame producer, created by
/// [`Engine::frame_producer`](crate::Engine::frame_producer) together with
/// its [`GpuProducer`].
///
/// [`submit`](Self::submit) installs the frame as the producer's current
/// frame — the one every binding samples — and wakes the host of every
/// visible surface a binding is on. The sink is `Send`: a decoder thread
/// may submit while the bindings stay on the engine's surfaces.
///
/// A frame producer has no setup: a device replacement drops its frame
/// with the old renderer, and the next `submit` supplies the first frame
/// on the new device
/// ([`drain_gpu_producers`](crate::GpuContent::drain_gpu_producers) is the
/// device-replacement contract).
pub struct FrameSink<B: crate::GpuContent> {
    tx: ProducerChannel<B>,
    id: crate::message::ProducerId,
}

impl<B: crate::GpuContent> std::fmt::Debug for FrameSink<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameSink")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<B: crate::GpuContent> FrameSink<B> {
    pub(crate) const fn new(id: crate::message::ProducerId, tx: ProducerChannel<B>) -> Self {
        Self { tx, id }
    }

    /// Installs `frame` as the producer's current frame. The submit
    /// travels in order with the engine's messages; each surface a
    /// binding of the producer is on treats it as the layer's frame swap.
    /// Once the frame lands on the render loop, each of those surfaces
    /// wakes its host through its own wake — every frame, whenever it
    /// lands relative to a render in flight — coalesced per surface until
    /// the surface next renders; a hidden surface wakes nothing.
    pub fn submit(&self, frame: impl Into<B::Frame>) {
        let frame = frame.into();
        let opaque = B::frame_opaque(&frame);
        let id = self.id;
        let _ = self.tx.send(crate::message::Message::ProducerFrame {
            producer: id,
            opaque,
            apply: Box::new(move |r| B::submit_frame(r, id, frame)),
        });
    }
}

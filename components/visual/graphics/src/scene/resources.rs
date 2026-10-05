//! Target-scoped resource registration for scene content.
//!
//! A host builds one [`SceneResources`] over the render target it already
//! selected — the `SceneBackend` its `cherenkov` engine fills, or another
//! target's own registry — and keeps it for as long as that target lives.
//! Each recording of scene content borrows it as [`RecordingResources`],
//! which is what [`SceneContent::build_scene`] receives.
//!
//! # Who holds a registration
//!
//! A registration lives exactly as long as somebody holds it, and three
//! parties do, one for each way a resource is still in use:
//!
//! - **The content** registers a font, an image or a shader paint in the
//!   frame that first draws it and keeps the returned [`Registered`] handle
//!   for as long as it goes on drawing the resource.
//! - **The recording** holds every resource it names. A handle gives out the
//!   id a recorder draws with only through [`RecordingResources::name`],
//!   which puts the handle in that recording's [`HeldResources`], so a
//!   recording cannot name a resource without holding it.
//! - **The host** keeps each installed recording's [`HeldResources`] until a
//!   recording that replaces it has been installed.
//!
//! The table itself holds nothing: it keeps a weak entry per registration for
//! deduplication, and when the last holder lets go the entry leaves the table
//! and the target's own handle drops, which unregisters the resource. So
//! content can drop a handle in the very call that records a drawing without
//! the resource: the recording still installed keeps it until the host
//! installs the one that no longer names it — even when the host renders in
//! between, or discards the new recording instead of installing it.
//!
//! The type deliberately carries no drawing methods, no layer operations and
//! no backend choice: the target is the host's, selected before this exists.
//! What this adds on top of `SceneBackend` is ownership that follows what is
//! drawn, exact deduplication — two contents drawing the same font or image
//! while both hold it share one registration — and a uniform surface for
//! targets with differing capabilities: a shader paint on a target without
//! shader support is an explicit [`ResourceError::Unsupported`], never a
//! silent miss; a source in a language the target does not draw is a
//! [`ResourceError::Shader`] naming the language it got.
//!
//! [`SceneContent::build_scene`]: crate::scene_view::SceneContent::build_scene
//! [`SceneResources`]: crate::resources::SceneResources
//! [`RecordingResources`]: crate::resources::RecordingResources
//! [`RecordingResources::name`]: crate::resources::RecordingResources::name
//! [`Registered`]: crate::resources::Registered
//! [`HeldResources`]: crate::resources::HeldResources
//! [`ResourceError::Unsupported`]: crate::source::ResourceError::Unsupported
//! [`ResourceError::Shader`]: crate::source::ResourceError::Shader

use alloc::borrow::Cow;
use alloc::rc::{Rc, Weak};
use alloc::sync::Arc;
use core::cell::RefCell;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::discriminant;
use core::ptr;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;

use crate::draw::{FontId, ImageId, ResourceId, ShaderId};
use crate::scene::source::{
    FontSource, Format, ImageColorSpace, ImageData, ResourceError, Rgba8, Rgba16F, ShaderLanguage,
    ShaderSource,
};

/// The bytes a registration was made from, kept by its entry so that a later
/// request is compared against them exactly.
#[derive(Clone)]
enum SourceBytes {
    /// Font and image data — the very allocation the request handed over —
    /// and owned shader text, copied once into an allocation of its own.
    Shared(Arc<[u8]>),
    /// Shader text compiled into the binary, which is never freed.
    Static(&'static [u8]),
}

impl SourceBytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Shared(bytes) => bytes,
            Self::Static(bytes) => bytes,
        }
    }
}

/// Where a source's bytes live.
///
/// Two byte slices that are alive at the same time, start at the same address
/// and have the same length are the same bytes. Every listed entry keeps its
/// own source alive, so its address cannot be reused by other bytes while the
/// entry is listed: a request found at a listed address is that entry's
/// source, and matching it needs neither a hash nor a comparison.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Address {
    start: usize,
    len: usize,
}

impl Address {
    fn of(bytes: &[u8]) -> Self {
        Self {
            start: bytes.as_ptr().addr(),
            len: bytes.len(),
        }
    }
}

/// A source identified by its content.
///
/// The hash only chooses the bucket: equal hashes still compare the bytes, so
/// two different sources never share a registration, whatever their hashes.
/// The hash is computed once, when the source is first compared, and is what
/// the map rehashes on growth — never the bytes.
#[derive(Clone)]
struct Content {
    hash: u64,
    bytes: SourceBytes,
}

impl Content {
    fn new(bytes: SourceBytes) -> Self {
        let mut hasher = DefaultHasher::new();
        bytes.as_slice().hash(&mut hasher);
        Self {
            hash: hasher.finish(),
            bytes,
        }
    }
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        let (ours, theirs) = (self.bytes.as_slice(), other.bytes.as_slice());
        self.hash == other.hash && (Address::of(ours) == Address::of(theirs) || ours == theirs)
    }
}

impl Eq for Content {}

impl Hash for Content {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

/// Everything besides its bytes that makes an image upload a different
/// image: the same bytes at another size, in another colour space or with the
/// other alpha convention draw differently.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ImageShape {
    width: u32,
    height: u32,
    color_space: ImageColorSpace,
    premultiplied: bool,
}

impl ImageShape {
    const fn of<F: Format>(data: &ImageData<F>) -> Self {
        Self {
            width: data.width(),
            height: data.height(),
            color_space: data.color_space,
            premultiplied: data.premultiplied,
        }
    }
}

impl Hash for ImageShape {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.width.hash(state);
        self.height.hash(state);
        discriminant(&self.color_space).hash(state);
        self.premultiplied.hash(state);
    }
}

/// Everything besides its bytes that makes a font source a different font:
/// the face's index in a byte-backed collection, or that the source names a
/// system family rather than bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum FontShape {
    /// A face index into a font file or collection.
    Face(u32),
    /// A system-font reference; the bytes slot carries the family name.
    System,
}

/// Everything besides its text that makes a shader source a different
/// shader: the same text in another language or with the other `animated`
/// flag compiles to a different program.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ShaderShape {
    animated: bool,
    language: ShaderLanguage,
}

/// A registration's exact identity: its source's shape — a font's index in
/// its collection, an image's [`ImageShape`], a shader's [`ShaderShape`], any
/// of which makes the same bytes a different resource — and its bytes.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key<S> {
    shape: S,
    content: Content,
}

/// Which table entry a registration occupies, so its last handle can take
/// the entry out when it drops.
enum EntryKey {
    Font(Key<FontShape>),
    Rgba8(Key<ImageShape>),
    Rgba16F(Key<ImageShape>),
    Shader(Key<ShaderShape>),
}

/// The registration a render target always provides: fonts and image
/// uploads.
///
/// A target returns a [`Handle`] for each registered resource — a font
/// object, a texture — naming the plain id a recording draws with.
/// `SceneResources` keeps the registry behind its private storage so the
/// table's deduplication is target-agnostic, and the [`Registered`] that
/// wraps the [`Handle`], so the resource lives exactly as long as the
/// sharing handles and the recordings that name it do.
pub trait SceneBackend: 'static {
    /// Registers `source` with the target.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Font`] when the data cannot be used — a `System`
    /// source on a target without a platform font stack, or bytes the
    /// target cannot parse — [`ResourceError::Lost`] when the renderer is
    /// gone.
    fn register_font(&self, source: FontSource) -> Result<Handle<FontId>, ResourceError>;

    /// Uploads `data` as an `Rgba8` image.
    ///
    /// # Errors
    ///
    /// [`ResourceError::TooLarge`] when the image exceeds
    /// [`image_limits`](Self::image_limits), [`ResourceError::Image`] when
    /// the target rejects the upload for another cause,
    /// [`ResourceError::Lost`] when the renderer is gone.
    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Handle<ImageId>, ResourceError>;

    /// Uploads `data` in the HDR/linear format.
    ///
    /// # Errors
    ///
    /// [`ResourceError::TooLarge`] when the image exceeds
    /// [`image_limits`](Self::image_limits), [`ResourceError::Image`] when
    /// the target rejects the upload for another cause,
    /// [`ResourceError::Lost`] when the renderer is gone.
    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Handle<ImageId>, ResourceError>;

    /// The largest image the target admits, in each dimension and in
    /// total texels — the device's texture limit, or the per-image share
    /// of the target's memory budget. Fixed for the target's life: an
    /// image it does not admit fails at registration with
    /// [`ResourceError::TooLarge`] instead of failing every render that
    /// draws it.
    fn image_limits(&self) -> crate::draw::ImageLimits;
}

/// The registration only a target that draws shader paints provides.
///
/// A source whose [`ShaderLanguage`] the target does not draw is rejected
/// here, at registration — the error names the language it got — never
/// dropped silently into a paint that renders nothing.
pub trait ShaderBackend: SceneBackend {
    /// Registers `source` with the target.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Shader`] when the source is not in a language the
    /// target draws or fails its validation, [`ResourceError::Lost`] when
    /// the renderer is gone.
    fn register_shader(&self, source: ShaderSource) -> Result<Handle<ShaderId>, ResourceError>;
}

/// A target's own handle to a registered resource — what [`Handle::new`]
/// wraps.
///
/// A `cherenkov::Font`, a platform font object, a texture — whatever its
/// registration returns. The registration behind a handle lives exactly as
/// long as the handle does, and a handle names the plain id a recording
/// draws the resource by.
pub trait ResourceHandle: 'static {
    /// The plain id a recording names this resource by.
    type Id: PlainId;

    /// This resource's plain id.
    fn id(&self) -> Self::Id;
}

/// A registration on its target — what `SceneBackend`'s methods return
/// and what the [`Registered`] a request shares holds.
///
/// `I` is the plain id a recording names the resource by. The target's own
/// handle lives inside behind a private trait object, so the type names no
/// target type in the public signature; dropping it releases the
/// registration on the target.
pub struct Handle<I: PlainId> {
    id: I,
    _owner: Box<dyn sealed::AnyHandle>,
}

impl<I: PlainId> Handle<I> {
    /// Wraps the target's own `handle`, keeping its plain id.
    pub fn new<H: ResourceHandle<Id = I>>(handle: H) -> Self {
        Self {
            id: handle.id(),
            _owner: Box::new(handle),
        }
    }

    /// The plain id a recording names this registration by.
    pub const fn id(&self) -> I {
        self.id
    }
}

impl<I: PlainId> fmt::Debug for Handle<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Handle")
            .field(&self.id.into_resource_id())
            .finish()
    }
}

#[cfg(any(feature = "cherenkov", test))]
impl ResourceHandle for cherenkov::Font {
    type Id = FontId;

    fn id(&self) -> FontId {
        Self::id(self)
    }
}

#[cfg(any(feature = "cherenkov", test))]
impl<F: cherenkov::Format> ResourceHandle for cherenkov::Image<F> {
    type Id = ImageId;

    fn id(&self) -> ImageId {
        Self::id(self)
    }
}

#[cfg(any(feature = "cherenkov", test))]
impl ResourceHandle for cherenkov::Shader {
    type Id = ShaderId;

    fn id(&self) -> ShaderId {
        Self::id(self)
    }
}

#[cfg(any(feature = "cherenkov", test))]
const fn engine_color_space(color_space: ImageColorSpace) -> cherenkov::ImageColorSpace {
    match color_space {
        ImageColorSpace::Srgb => cherenkov::ImageColorSpace::Srgb,
        ImageColorSpace::DisplayP3 => cherenkov::ImageColorSpace::DisplayP3,
        ImageColorSpace::LinearSrgb => cherenkov::ImageColorSpace::LinearSrgb,
        ImageColorSpace::LinearP3 => cherenkov::ImageColorSpace::LinearP3,
    }
}

/// `ImageData` rebuilt as the engine's typed upload: same texels, same
/// colour space and alpha convention, its own format marker.
#[cfg(any(feature = "cherenkov", test))]
fn engine_image<F: Format, E: cherenkov::Format>(data: &ImageData<F>) -> cherenkov::ImageData<E> {
    let upload =
        cherenkov::ImageData::<E>::new(data.width(), data.height(), Arc::clone(data.data()))
            .expect("a validated ImageData stays valid")
            .color_space(engine_color_space(data.color_space));
    if data.premultiplied {
        upload.premultiplied()
    } else {
        upload
    }
}

#[cfg(any(feature = "cherenkov", test))]
impl<B> SceneBackend for cherenkov::Engine<B>
where
    B: cherenkov::Backend
        + cherenkov::Uploads<cherenkov::Rgba8>
        + cherenkov::Uploads<cherenkov::Rgba16F>,
{
    fn register_font(&self, source: FontSource) -> Result<Handle<FontId>, ResourceError> {
        match source {
            FontSource::Bytes { data, index } => {
                let font = Self::font(self, cherenkov::FontSource { data, index })?;
                Ok(Handle::new(font))
            }
            FontSource::System { family } => Err(ResourceError::Font(format!(
                "system font '{family}' is not registered: the Cherenkov engine registers fonts from bytes"
            ))),
        }
    }

    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Handle<ImageId>, ResourceError> {
        let image = Self::image(self, engine_image::<Rgba8, cherenkov::Rgba8>(&data))?;
        Ok(Handle::new(image))
    }

    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Handle<ImageId>, ResourceError> {
        let image = Self::image(self, engine_image::<Rgba16F, cherenkov::Rgba16F>(&data))?;
        Ok(Handle::new(image))
    }

    fn image_limits(&self) -> crate::draw::ImageLimits {
        Self::image_limits(self)
    }
}

#[cfg(any(feature = "cherenkov", test))]
impl<B> ShaderBackend for cherenkov::Engine<B>
where
    B: cherenkov::Backend
        + cherenkov::Uploads<cherenkov::Rgba8>
        + cherenkov::Uploads<cherenkov::Rgba16F>
        + cherenkov::ShaderPaintCapability,
{
    fn register_shader(&self, source: ShaderSource) -> Result<Handle<ShaderId>, ResourceError> {
        match source.language {
            ShaderLanguage::Wgsl => {
                let shader = Self::shader(
                    self,
                    cherenkov::ShaderSource {
                        source: source.source,
                        animated: source.animated,
                    },
                )?;
                Ok(Handle::new(shader))
            }
            language @ ShaderLanguage::Agsl => Err(ResourceError::Shader(format!(
                "{language} shader source is not registered: the Cherenkov engine draws WGSL shader paint"
            ))),
        }
    }
}

/// What a Cherenkov backend offers beyond the unconditional registration,
/// resolved on the backend type so a host's `SceneResources` construction
/// carries no `Option` decision of its own.
#[cfg(feature = "cherenkov")]
mod sealed_caps {
    use alloc::rc::Rc;

    use cherenkov::{Backend, Engine};

    /// Per-backend capabilities [`SceneResources`](super::SceneResources)
    /// surfaces: which registry the backend's engine offers.
    ///
    /// A backend declares them here; under-declaring simply reports
    /// `Unsupported` at registration.
    pub trait SceneCaps: Backend {
        /// The registration table over `engine`. The default registers
        /// fonts and image uploads but no shader paints; a backend that
        /// draws them overrides it with
        /// [`SceneResources::with_shaders`](super::SceneResources::with_shaders).
        fn resources(engine: Rc<Engine<Self>>) -> super::SceneResources
        where
            Self: cherenkov::Uploads<cherenkov::Rgba8> + cherenkov::Uploads<cherenkov::Rgba16F>,
        {
            super::SceneResources::new(engine)
        }
    }
}

#[cfg(feature = "cherenkov")]
pub use sealed_caps::SceneCaps;

#[cfg(feature = "gpu")]
impl SceneCaps for cherenkov_gpu::Gpu {
    fn resources(engine: Rc<cherenkov::Engine<Self>>) -> SceneResources {
        SceneResources::with_shaders(engine.clone(), engine)
    }
}

#[cfg(feature = "cpu")]
impl SceneCaps for cherenkov_cpu::Raster {}

/// One live registration: the target's handle, erased to its kind, plus the
/// table entry that dedupes it. Dropping it takes the entry out of the
/// table, then drops the target handle, which unregisters the resource.
struct Entry {
    handle: Box<dyn sealed::HeldHandle>,
    key: EntryKey,
    table: Weak<Table>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(table) = self.table.upgrade() {
            table.unlist(&self.key);
        }
    }
}

mod sealed {
    /// Seals [`PlainId`](super::PlainId) to the kinds the registry has.
    pub trait Sealed {}
    impl Sealed for crate::draw::FontId {}
    impl Sealed for crate::draw::ImageId {}
    impl Sealed for crate::draw::ShaderId {}

    /// The erased target handle a [`Handle`](super::Handle) owns: dropping
    /// the object releases the registration. A marker only — the wrapper
    /// reads the plain id once, when it is built.
    pub trait AnyHandle: 'static {}
    impl<T: super::ResourceHandle> AnyHandle for T {}

    /// The erased handle an `Entry` keeps: a [`Handle`](super::Handle) the
    /// registry returned.
    pub trait HeldHandle {
        /// The plain id the erased handle names.
        fn id(&self) -> crate::draw::ResourceId;
    }
}

impl<I: PlainId> sealed::HeldHandle for Handle<I> {
    fn id(&self) -> ResourceId {
        self.id.into_resource_id()
    }
}

/// The plain resource identifier a [`Registered`] names — one of `FontId`,
/// `ImageId` or `ShaderId`, the ids recordings draw with.
///
/// Sealed: only the kinds [`SceneResources`] registers exist.
pub trait PlainId: sealed::Sealed + Copy + 'static {
    /// Wraps this id in its [`ResourceId`] variant.
    fn into_resource_id(self) -> ResourceId;

    /// Reads this id back out of its [`ResourceId`] variant.
    ///
    /// The entry always stores the matching variant — a `Registered` is
    /// made by the registration method for its kind — so a mismatch is a
    /// bug in the table, not a condition a caller checks.
    fn from_resource_id(id: ResourceId) -> Self;
}

impl PlainId for FontId {
    fn into_resource_id(self) -> ResourceId {
        ResourceId::Font(self)
    }

    fn from_resource_id(id: ResourceId) -> Self {
        match id {
            ResourceId::Font(id) => id,
            _ => unreachable!("the entry stores a font id"),
        }
    }
}

impl PlainId for ImageId {
    fn into_resource_id(self) -> ResourceId {
        ResourceId::Image(self)
    }

    fn from_resource_id(id: ResourceId) -> Self {
        match id {
            ResourceId::Image(id) => id,
            _ => unreachable!("the entry stores an image id"),
        }
    }
}

impl PlainId for ShaderId {
    fn into_resource_id(self) -> ResourceId {
        ResourceId::Shader(self)
    }

    fn from_resource_id(id: ResourceId) -> Self {
        match id {
            ResourceId::Shader(id) => id,
            _ => unreachable!("the entry stores a shader id"),
        }
    }
}

/// A resource registered through [`SceneResources`], shared by everyone who
/// asked for the same source while it was held.
///
/// `I` is the plain id the registration names — `FontId`, `ImageId` or
/// `ShaderId` — the id a recording draws the resource by. The handle
/// inside is the target's own, so this is the content's share of the
/// registration: cloning it shares the registration, and once the last
/// clone and every recording holding it are gone, the table's entry leaves
/// and the target unregisters the resource.
///
/// It deliberately does not hand out the resource's id. A recording names the
/// resource through [`RecordingResources::name`], which holds the
/// registration for as long as that recording may be drawn — so the id a
/// recorder draws with never outlives the registration behind it. Two handles
/// compare equal when they share one registration.
pub struct Registered<I: PlainId> {
    entry: Rc<Entry>,
    marker: PhantomData<I>,
}

impl<I: PlainId> Clone for Registered<I> {
    fn clone(&self) -> Self {
        Self {
            entry: Rc::clone(&self.entry),
            marker: PhantomData,
        }
    }
}

impl<I: PlainId> PartialEq for Registered<I> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.entry, &other.entry)
    }
}

impl<I: PlainId> Eq for Registered<I> {}

impl<I: PlainId> fmt::Debug for Registered<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Registered")
            .field(&self.entry.handle.id())
            .finish()
    }
}

/// A registration a recording holds, whatever its kind.
trait HeldEntry {}

impl HeldEntry for Entry {}

/// Where a held registration lives, which identifies it.
fn entry_address<T: ?Sized>(entry: &Rc<T>) -> usize {
    Rc::as_ptr(entry).cast::<()>().addr()
}

/// The registrations one recording names, held for as long as the recording
/// may be drawn.
///
/// [`RecordingResources::finish`] produces it beside the recording it belongs
/// to. The host keeps it for as long as that recording is installed and drops
/// it only once a recording that replaces it has been installed, for example
/// right after the `Surface::update` that installs the replacement: the
/// target applies an install before it draws
/// again, so no frame draws the replaced recording after its resources are
/// released. A recording that is discarded rather than installed takes its
/// set with it and releases nothing the installed recording draws.
///
/// Cloning shares the set.
#[derive(Clone)]
pub struct HeldResources {
    table: Weak<Table>,
    entries: Rc<[Rc<dyn HeldEntry>]>,
}

impl HeldResources {
    /// The set of a recording that names no resource.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            table: Weak::new(),
            entries: Rc::from([]),
        }
    }
}

impl fmt::Debug for HeldResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeldResources")
            .field("held", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// The resource side of one recording: registration through the engine's
/// [`SceneResources`], and the set of every registration the recording names.
///
/// It forwards the table's registration methods and nothing else. In
/// particular it offers no way to begin another recording, whose names would
/// be held only by a set that nobody installs:
///
/// ```compile_fail
/// # use waterui_graphics::draw::ImageId;
/// # use waterui_graphics::{RecordingResources, Registered};
/// fn stale_id(resources: &RecordingResources<'_>, image: &Registered<ImageId>) {
///     // The id would be held by a temporary that drops at the semicolon.
///     let _ = resources.recording().name(image);
/// }
/// ```
///
/// A host begins one with [`SceneResources::recording`] for each recording it
/// makes, hands it to every [`SceneContent::build_scene`] that records into
/// that recording, and takes the set with [`finish`](Self::finish).
///
/// [`SceneContent::build_scene`]: crate::scene_view::SceneContent::build_scene
pub struct RecordingResources<'a> {
    resources: &'a SceneResources,
    held: HashMap<usize, Rc<dyn HeldEntry>>,
}

impl RecordingResources<'_> {
    /// The engine-scoped table this recording borrows.
    pub(crate) const fn scene_resources(&self) -> &SceneResources {
        self.resources
    }
    /// The id to record `resource` by, holding the registration for as long
    /// as this recording may be drawn.
    ///
    /// Name a resource in every recording that draws it — an id kept from an
    /// earlier recording is not held by this one.
    ///
    /// # Panics
    ///
    /// When `resource` was registered through another target's
    /// [`SceneResources`]: its id means nothing, or something else, on this
    /// one.
    pub fn name<I: PlainId>(&mut self, resource: &Registered<I>) -> I {
        assert!(
            ptr::eq(
                resource.entry.table.as_ptr(),
                Rc::as_ptr(&self.resources.table)
            ),
            "a resource registered on another target was named in this recording"
        );
        let entry: Rc<dyn HeldEntry> = Rc::<Entry>::clone(&resource.entry);
        self.held.entry(entry_address(&entry)).or_insert(entry);
        I::from_resource_id(resource.entry.handle.id())
    }

    /// Holds every registration in `held` for this recording too — for a
    /// recording that draws another one, such as a picture recorded with
    /// [`Picture::record_with`](crate::picture::Picture::record_with).
    ///
    /// # Panics
    ///
    /// When `held` belongs to another target's [`SceneResources`].
    pub fn hold(&mut self, held: &HeldResources) {
        if held.entries.is_empty() {
            return;
        }
        assert!(
            ptr::eq(held.table.as_ptr(), Rc::as_ptr(&self.resources.table)),
            "a recording naming another target's resources was drawn in this recording"
        );
        for entry in held.entries.iter() {
            self.held
                .entry(entry_address(entry))
                .or_insert_with(|| Rc::clone(entry));
        }
    }

    /// Registers `source`; see [`SceneResources::font`].
    ///
    /// # Errors
    ///
    /// As [`SceneResources::font`].
    pub fn font(&self, source: FontSource) -> Result<Registered<FontId>, ResourceError> {
        self.resources.font(source)
    }

    /// Registers `Rgba8` image data; see [`SceneResources::image`].
    ///
    /// # Errors
    ///
    /// As [`SceneResources::image`].
    pub fn image(&self, data: ImageData<Rgba8>) -> Result<Registered<ImageId>, ResourceError> {
        self.resources.image(data)
    }

    /// Registers `Rgba16Float` image data; see [`SceneResources::image16f`].
    ///
    /// # Errors
    ///
    /// As [`SceneResources::image16f`].
    pub fn image16f(&self, data: ImageData<Rgba16F>) -> Result<Registered<ImageId>, ResourceError> {
        self.resources.image16f(data)
    }

    /// Registers a shader paint's source; see [`SceneResources::shader`].
    ///
    /// # Errors
    ///
    /// As [`SceneResources::shader`].
    pub fn shader(&self, source: ShaderSource) -> Result<Registered<ShaderId>, ResourceError> {
        self.resources.shader(source)
    }

    /// The largest image the target admits; see
    /// [`SceneResources::image_limits`].
    #[must_use]
    pub fn image_limits(&self) -> crate::draw::ImageLimits {
        self.resources.image_limits()
    }

    /// The registrations this recording names, for the host to keep beside
    /// it; see [`HeldResources`].
    #[must_use = "the recording names these resources; keep them for as long as it is installed"]
    pub fn finish(self) -> HeldResources {
        HeldResources {
            table: Rc::downgrade(&self.resources.table),
            entries: self.held.into_values().collect(),
        }
    }
}

impl fmt::Debug for RecordingResources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordingResources")
            .field("held", &self.held.len())
            .finish_non_exhaustive()
    }
}

/// The live registrations of one kind, found either by the allocation their
/// source bytes live in — a request that hands over a source the table already
/// holds costs one lookup, without reading its bytes — or by exact content.
struct Listing<S> {
    by_address: HashMap<(S, Address), Weak<Entry>>,
    by_content: HashMap<Key<S>, Weak<Entry>>,
}

impl<S: Copy + Eq + Hash> Listing<S> {
    fn new() -> RefCell<Self> {
        RefCell::new(Self {
            by_address: HashMap::new(),
            by_content: HashMap::new(),
        })
    }

    fn find_address(&self, shape: S, address: Address) -> Option<Rc<Entry>> {
        self.by_address
            .get(&(shape, address))
            .and_then(Weak::upgrade)
    }

    fn find_content(&self, key: &Key<S>) -> Option<Rc<Entry>> {
        self.by_content.get(key).and_then(Weak::upgrade)
    }

    fn list(&mut self, key: Key<S>, entry: &Rc<Entry>) {
        let address = Address::of(key.content.bytes.as_slice());
        self.by_address
            .insert((key.shape, address), Rc::downgrade(entry));
        self.by_content.insert(key, Rc::downgrade(entry));
    }

    fn unlist(&mut self, key: &Key<S>) {
        let address = Address::of(key.content.bytes.as_slice());
        self.by_address.remove(&(key.shape, address));
        self.by_content.remove(key);
    }

    /// How many registrations are listed, dead entries included; both
    /// indexes must list each one exactly once.
    #[cfg(test)]
    fn len(&self) -> usize {
        assert_eq!(
            self.by_address.len(),
            self.by_content.len(),
            "the address and content indexes disagree"
        );
        self.by_content.len()
    }
}

/// The shared state behind [`SceneResources`]; entries reach it weakly to
/// take themselves out.
struct Table {
    backend: Rc<dyn SceneBackend>,
    shaders: Option<Rc<dyn ShaderBackend>>,
    fonts: RefCell<Listing<FontShape>>,
    images_rgba8: RefCell<Listing<ImageShape>>,
    images_rgba16f: RefCell<Listing<ImageShape>>,
    shaders_cache: RefCell<Listing<ShaderShape>>,
}

impl Table {
    /// Removes the entry of a registration whose last handle is dropping.
    fn unlist(&self, key: &EntryKey) {
        match key {
            EntryKey::Font(key) => self.fonts.borrow_mut().unlist(key),
            EntryKey::Rgba8(key) => self.images_rgba8.borrow_mut().unlist(key),
            EntryKey::Rgba16F(key) => self.images_rgba16f.borrow_mut().unlist(key),
            EntryKey::Shader(key) => self.shaders_cache.borrow_mut().unlist(key),
        }
    }
}

/// Resource registration over one render target, shared by the content
/// drawn on it.
///
/// Constructed once from the host's already-selected target — the
/// `SceneBackend` its registry fills, with its `ShaderBackend` when the
/// target draws shader paints — and lent, as [`RecordingResources`], to
/// every [`SceneContent::build_scene`] on that target; see the module
/// documentation for the ownership contract. The table holds the registry
/// strongly and its registrations weakly: it never keeps a resource alive,
/// so it can live as long as the target does without pinning anything that
/// is no longer drawn.
///
/// Deduplication is exact. Each live registration keeps the bytes it was made
/// from — the `Arc` the request handed over, not a copy; owned shader text is
/// copied once — and only a request with identical bytes and an identical
/// shape shares it. A request that hands over the allocation a registration
/// already keeps is found by address alone, so content that asks again every
/// frame with the source it holds costs one lookup; identical bytes from
/// another allocation are hashed once and compared.
///
/// # Blocking
///
/// A request for a source whose registration is live returns without
/// touching the target. Any other request — [`font`](Self::font),
/// [`image`](Self::image), [`image16f`](Self::image16f) or
/// [`shader`](Self::shader) — validates on the calling thread and queues
/// the registration on the target, in order ahead of every render that
/// could draw it, so the returned id is valid the moment it is recorded.
/// The calling thread's only waits are inside that validation — a font's
/// parse, a shader's pre-validation, an image's check against
/// [`image_limits`](Self::image_limits): an image the limits do not
/// admit fails here with [`ResourceError::TooLarge`] before anything is
/// queued, never as a failed render.
///
/// [`SceneContent::build_scene`]: crate::scene_view::SceneContent::build_scene
pub struct SceneResources {
    table: Rc<Table>,
}

impl SceneResources {
    /// Resource registration over `backend`, the registry of the host's
    /// already-selected render target — fonts and image uploads, no shader
    /// paints: a shader source reports [`ResourceError::Unsupported`] at
    /// registration. A target that draws them is built with
    /// [`with_shaders`](Self::with_shaders).
    ///
    /// [`ResourceError::Unsupported`]: crate::source::ResourceError::Unsupported
    pub fn new<B: SceneBackend>(backend: Rc<B>) -> Self {
        Self::over(backend, None)
    }

    /// Registration over `backend`, with `shaders` as the registry shader
    /// paints register through — usually the target's same object as
    /// `backend`. For a Cherenkov engine the `SceneCaps` bound builds this
    /// on the backend type: `Gpu` hands its engine over, `Raster` takes
    /// the default [`new`](Self::new).
    pub fn with_shaders<B: SceneBackend, S: ShaderBackend>(backend: Rc<B>, shaders: Rc<S>) -> Self {
        Self::over(backend, Some(shaders))
    }

    /// The table's storage: the erased registry objects the friendly
    /// constructors take concretely.
    fn over(backend: Rc<dyn SceneBackend>, shaders: Option<Rc<dyn ShaderBackend>>) -> Self {
        Self {
            table: Rc::new(Table {
                backend,
                shaders,
                fonts: Listing::new(),
                images_rgba8: Listing::new(),
                images_rgba16f: Listing::new(),
                shaders_cache: Listing::new(),
            }),
        }
    }

    /// Begins the resource side of one recording; see
    /// [`RecordingResources`].
    #[must_use]
    pub fn recording(&self) -> RecordingResources<'_> {
        RecordingResources {
            resources: self,
            held: HashMap::new(),
        }
    }

    /// The largest image the target admits, in each dimension and in
    /// total texels — what it reports through
    /// [`SceneBackend::image_limits`]. Fixed for the target's life.
    /// [`ImageLimits::fit`](crate::draw::ImageLimits::fit) gives the
    /// largest admitted size at a kept aspect ratio, for content that
    /// scales an oversized source down itself instead of taking the
    /// [`ResourceError::TooLarge`].
    #[must_use]
    pub fn image_limits(&self) -> crate::draw::ImageLimits {
        self.table.backend.image_limits()
    }

    /// The live registration of `bytes` in `shape`, or a new one from
    /// `register` listed while it lives.
    ///
    /// A source the table already holds is found by its address alone; any
    /// other source is hashed once and, on a hash match, compared byte for
    /// byte, so only identical sources ever share a registration.
    fn intern<S, I>(
        &self,
        listing: fn(&Table) -> &RefCell<Listing<S>>,
        shape: S,
        bytes: SourceBytes,
        entry_key: fn(Key<S>) -> EntryKey,
        register: impl FnOnce(&Table) -> Result<Handle<I>, ResourceError>,
    ) -> Result<Registered<I>, ResourceError>
    where
        S: Copy + Eq + Hash,
        I: PlainId,
    {
        let listing = listing(&self.table);
        let held = listing
            .borrow()
            .find_address(shape, Address::of(bytes.as_slice()));
        if let Some(entry) = held {
            return Ok(Registered {
                entry,
                marker: PhantomData,
            });
        }
        let key = Key {
            shape,
            content: Content::new(bytes),
        };
        let identical = listing.borrow().find_content(&key);
        if let Some(entry) = identical {
            return Ok(Registered {
                entry,
                marker: PhantomData,
            });
        }
        let entry = Rc::new(Entry {
            handle: Box::new(register(&self.table)?),
            key: entry_key(key.clone()),
            table: Rc::downgrade(&self.table),
        });
        listing.borrow_mut().list(key, &entry);
        Ok(Registered {
            entry,
            marker: PhantomData,
        })
    }

    /// Registers `source` with the engine, returning the shared handle.
    ///
    /// While any handle to it is held, identical sources — the same bytes and
    /// the same collection index — map to that one registration. Asking again
    /// with the `Arc` the registration was made from is a single lookup; an
    /// identical font in another allocation is hashed and compared.
    ///
    /// A new registration validates on the calling thread and is queued
    /// on the target; see [Blocking](Self#blocking).
    ///
    /// # Errors
    ///
    /// [`ResourceError::Font`] when the data cannot be used,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn font(&self, source: FontSource) -> Result<Registered<FontId>, ResourceError> {
        let (shape, bytes) = match &source {
            FontSource::Bytes { data, index } => (
                FontShape::Face(*index),
                SourceBytes::Shared(Arc::clone(data)),
            ),
            FontSource::System { family } => (
                FontShape::System,
                SourceBytes::Shared(Arc::from(family.as_bytes())),
            ),
        };
        self.intern(
            |table| &table.fonts,
            shape,
            bytes,
            EntryKey::Font,
            |table| table.backend.register_font(source),
        )
    }

    /// Registers `Rgba8` image data.
    ///
    /// [`ImageData::new`] validates the dimensions and byte length before the
    /// engine ever sees the upload. While any handle to it is held, identical
    /// images — the same bytes, size, colour space and alpha convention —
    /// share one registration; as with fonts, asking again with the same
    /// `Arc` is a single lookup.
    ///
    /// A new registration is checked against
    /// [`image_limits`](Self::image_limits) — before the source is even
    /// hashed — and queued on the target; see [Blocking](Self#blocking).
    ///
    /// # Errors
    ///
    /// [`ResourceError::TooLarge`] when the image exceeds the target's
    /// image limits, [`ResourceError::Image`] when the target rejects the
    /// upload for another cause, [`ResourceError::Lost`] when the render
    /// thread is gone.
    pub fn image(&self, data: ImageData<Rgba8>) -> Result<Registered<ImageId>, ResourceError> {
        let limits = self.image_limits();
        if !limits.admits(data.width, data.height) {
            return Err(ResourceError::TooLarge {
                width: data.width,
                height: data.height,
                limits,
            });
        }
        self.intern(
            |table| &table.images_rgba8,
            ImageShape::of(&data),
            SourceBytes::Shared(Arc::clone(data.data())),
            EntryKey::Rgba8,
            |table| table.backend.register_rgba8(data),
        )
    }

    /// Registers `Rgba16Float` image data — the format HDR and linear-space
    /// sources upload as — deduplicated as [`image`](Self::image) is.
    ///
    /// A new registration is checked against
    /// [`image_limits`](Self::image_limits) — before the source is even
    /// hashed — and queued on the target; see [Blocking](Self#blocking).
    ///
    /// # Errors
    ///
    /// [`ResourceError::TooLarge`] when the image exceeds the target's
    /// image limits, [`ResourceError::Image`] when the target rejects the
    /// upload for another cause, [`ResourceError::Lost`] when the render
    /// thread is gone.
    pub fn image16f(&self, data: ImageData<Rgba16F>) -> Result<Registered<ImageId>, ResourceError> {
        let limits = self.image_limits();
        if !limits.admits(data.width, data.height) {
            return Err(ResourceError::TooLarge {
                width: data.width,
                height: data.height,
                limits,
            });
        }
        self.intern(
            |table| &table.images_rgba16f,
            ImageShape::of(&data),
            SourceBytes::Shared(Arc::clone(data.data())),
            EntryKey::Rgba16F,
            |table| table.backend.register_rgba16f(data),
        )
    }

    /// Registers a shader paint's WGSL source.
    ///
    /// While any handle to it is held, the same source text with the same
    /// `animated` flag maps to that one registration. Static text asked for
    /// again is a single lookup; owned text is hashed and compared.
    ///
    /// A new registration validates on the calling thread and is queued
    /// on the target; see [Blocking](Self#blocking).
    ///
    /// # Errors
    ///
    /// [`ResourceError::Unsupported`] when the engine's backend does not draw
    /// shader paints, [`ResourceError::Shader`] when the source fails
    /// validation, [`ResourceError::Lost`] when the render thread is gone.
    pub fn shader(&self, source: ShaderSource) -> Result<Registered<ShaderId>, ResourceError> {
        let Some(backend) = self.table.shaders.clone() else {
            return Err(ResourceError::Unsupported("shader paint"));
        };
        let bytes = match &source.source {
            Cow::Borrowed(text) => SourceBytes::Static(text.as_bytes()),
            Cow::Owned(text) => SourceBytes::Shared(Arc::from(text.as_bytes())),
        };
        self.intern(
            |table| &table.shaders_cache,
            ShaderShape {
                animated: source.animated,
                language: source.language,
            },
            bytes,
            EntryKey::Shader,
            move |_| backend.register_shader(source),
        )
    }

    /// How many registrations the table lists across every kind — dead
    /// entries included, which is what a test of the cleanup has to see.
    #[cfg(test)]
    fn listed(&self) -> usize {
        let table = &self.table;
        table.fonts.borrow().len()
            + table.images_rgba8.borrow().len()
            + table.images_rgba16f.borrow().len()
            + table.shaders_cache.borrow().len()
    }
}

impl fmt::Debug for SceneResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneResources")
            .field("shaders", &self.table.shaders.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::rc::Rc;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use core::cell::RefCell;
    use std::collections::HashSet;
    use std::sync::mpsc::{Receiver, channel};

    use cherenkov::testing::{Event, Null, NullConfig};
    use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat, Surface};

    use crate::draw::{
        Command, Content as Recording, Draw as _, ImageId, Recorder, Sampling, WorkingColor,
        kurbo::Rect,
    };
    use crate::scene::source::{ImageColorSpace, ImageData, Rgba8};

    use super::{
        Content, HeldResources, RecordingResources, Registered, SceneResources, SourceBytes,
    };
    use crate::scene_view::SceneContent;

    fn null_engine() -> (Rc<Engine<Null>>, Receiver<Event>) {
        let (events, probe) = channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: HashSet::new(),
            image_limits: cherenkov::ImageLimits::UNLIMITED,
        })
        .expect("the null engine failed to start");
        (Rc::new(engine), probe)
    }

    /// A registration table over a `Null` engine, with the engine's event
    /// probe, for tests that draw content without caring about pixels.
    pub fn null_resources() -> (SceneResources, Receiver<Event>) {
        let (engine, probe) = null_engine();
        (SceneResources::with_shaders(engine.clone(), engine), probe)
    }

    pub fn one_pixel() -> ImageData<Rgba8> {
        ImageData::<Rgba8>::new(1, 1, Vec::from([255, 0, 0, 255])).expect("valid image")
    }

    #[test]
    fn sources_whose_hashes_collide_are_still_different() {
        let content = |hash, byte: u8| Content {
            hash,
            bytes: SourceBytes::Shared(Arc::from([byte])),
        };
        assert!(
            content(7, 1) != content(7, 2),
            "equal hashes must not make different bytes one registration"
        );
        assert!(content(7, 1) == content(7, 1));
    }

    #[test]
    fn only_identical_sources_share_a_registration() {
        let (resources, _events) = null_resources();
        let held = resources.image(one_pixel()).expect("image registration");
        // `one_pixel` allocates afresh on every call: this is the content
        // path, not the allocation one.
        let identical = resources.image(one_pixel()).expect("image registration");
        assert_eq!(identical, held);
        let linear = resources
            .image(one_pixel().color_space(ImageColorSpace::LinearSrgb))
            .expect("image registration");
        assert_ne!(
            linear, held,
            "the same bytes in another colour space are another image"
        );
        let premultiplied = resources
            .image(one_pixel().premultiplied())
            .expect("image registration");
        assert_ne!(
            premultiplied, held,
            "the same bytes under the other alpha convention are another image"
        );
    }

    #[test]
    fn a_registration_from_an_old_engine_cannot_be_named_in_a_new_recording() {
        let (old_resources, _old_events) = null_resources();
        let (new_resources, _new_events) = null_resources();
        let old_image = old_resources.image(one_pixel()).expect("old image");
        let mut new_recording = new_resources.recording();
        let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            new_recording.name(&old_image);
        }));
        assert!(
            rejected.is_err(),
            "a recording on a replacement engine must reject an old resource handle"
        );
        let new_image = new_resources.image(one_pixel()).expect("new image");
        let _new_id = new_recording.name(&new_image);
    }

    fn added_images(events: &[Event]) -> Vec<ImageId> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::AddImage(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    pub fn removed_images(events: &[Event]) -> Vec<ImageId> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::RemoveImage(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Content whose first two frames are a plain fill, whose third frame
    /// first draws an image, which draws it again up to its `last` frame, and
    /// which draws the fill alone — and lets the image go — after that.
    struct LateImage {
        frame: u32,
        last: u32,
        image: Option<Registered<ImageId>>,
    }

    impl LateImage {
        /// Draws the image on its third and fourth frames.
        const fn new() -> Self {
            Self::drawing_until(4)
        }

        const fn drawing_until(last: u32) -> Self {
            Self {
                frame: 0,
                last,
                image: None,
            }
        }
    }

    impl SceneContent for LateImage {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            self.frame += 1;
            let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
            recorder.fill(bounds, WorkingColor::BLACK);
            if (3..=self.last).contains(&self.frame) {
                let image = self.image.get_or_insert_with(|| {
                    resources
                        .image(one_pixel())
                        .expect("image registration failed")
                });
                recorder.image(resources.name(image), bounds, Sampling::Nearest);
            } else {
                self.image = None;
            }
            false
        }

        fn rebuild_for_engine(&mut self) {
            self.image = None;
        }
    }

    /// A recording the host has made but not installed yet, with the
    /// registrations it names and the image ids it draws.
    pub struct Recorded {
        content: Recording,
        held: HeldResources,
        drawn: Vec<ImageId>,
    }

    /// Every image id `commands` draws, counting the ones inside recorded
    /// pictures: drawing a `PictureRecording` emits one `Command::Picture`
    /// whose shared list carries the images, so a flat scan reports nothing
    /// for exactly the content these tests mount.
    fn drawn_images(commands: &[Command], drawn: &mut Vec<ImageId>) {
        for command in commands {
            match command {
                Command::Image { image, .. } => drawn.push(*image),
                Command::Picture { picture, .. } => {
                    drawn_images(picture.display_list().commands(), drawn);
                }
                _ => {}
            }
        }
    }

    /// What one frame did: the image ids the installed recording draws, and
    /// the render-thread events from recording it through rendering it.
    pub struct FrameReport {
        pub drawn: Vec<ImageId>,
        pub events: Vec<Event>,
    }

    /// A host mounting one content on a `Null` engine's surface root, holding
    /// the installed recording's resources the way a real host does.
    pub struct Mount {
        engine: Rc<Engine<Null>>,
        probe: Receiver<Event>,
        pub resources: SceneResources,
        surface: Surface<Null>,
        installed: RefCell<HeldResources>,
    }

    impl Mount {
        pub fn new() -> Self {
            let (engine, probe) = null_engine();
            let surface = engine
                .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
                .expect("surface");
            let _ = probe.try_iter().count();
            Self {
                resources: SceneResources::with_shaders(engine.clone(), engine.clone()),
                engine,
                probe,
                surface,
                installed: RefCell::new(HeldResources::empty()),
            }
        }

        /// Records `content` without installing the recording.
        pub fn record(&self, content: &mut dyn SceneContent) -> Recorded {
            let mut resources = self.resources.recording();
            let mut recorded = self.surface.record(|recorder| {
                content.build_scene(recorder, &mut resources, 8.0, 8.0);
            });
            let mut drawn = Vec::new();
            drawn_images(recorded.snapshot().commands(), &mut drawn);
            Recorded {
                content: recorded,
                held: resources.finish(),
                drawn,
            }
        }

        /// Installs `recorded` on the mounted layer, then lets go of what the
        /// recording it replaces held.
        pub fn install(&self, recorded: Recorded) {
            self.surface.update(|tx| {
                tx[self.surface.root()].content(recorded.content);
            });
            drop(self.installed.replace(recorded.held));
        }

        /// Renders one frame and returns every render-thread event since the
        /// last render.
        pub fn render(&self) -> Vec<Event> {
            self.engine.render(FrameTime::now()).expect("render");
            self.probe.try_iter().collect()
        }

        /// One host frame: record, install the recording, render.
        pub fn frame(&self, content: &mut dyn SceneContent) -> FrameReport {
            let recorded = self.record(content);
            let drawn = recorded.drawn.clone();
            self.install(recorded);
            FrameReport {
                drawn,
                events: self.render(),
            }
        }
    }

    fn position(events: &[Event], wanted: impl Fn(&Event) -> bool) -> usize {
        events
            .iter()
            .position(wanted)
            .unwrap_or_else(|| panic!("missing event in {events:?}"))
    }

    /// The frame registered `id`, then installed and rendered a recording
    /// naming it — in that order on the render thread.
    fn assert_registered_then_drawn(report: &FrameReport, id: ImageId) {
        assert_eq!(
            report.drawn,
            [id],
            "the frame draws the image it registered"
        );
        let events = &report.events;
        let added = position(events, |event| matches!(event, Event::AddImage(_)));
        let installed = position(events, |event| matches!(event, Event::SetContent(..)));
        let rendered = position(events, |event| matches!(event, Event::Frame(_)));
        assert!(
            added < installed && installed < rendered,
            "the image must reach the engine before the recording naming it: {events:?}"
        );
    }

    /// The frame installed a recording that no longer names `id` and released
    /// the registration too, both inside the frame's event batch: the release
    /// commit and the content install travel on different queues, so either
    /// may land first — what matters is that no frame renders between them.
    fn assert_released_then_replaced(report: &FrameReport, id: ImageId) {
        let events = &report.events;
        assert_eq!(removed_images(events), [id], "{events:?}");
        let released = position(events, |event| matches!(event, Event::RemoveImage(_)));
        let installed = position(events, |event| matches!(event, Event::SetContent(..)));
        let rendered = position(events, |event| matches!(event, Event::Frame(_)));
        assert!(
            released < rendered && installed < rendered,
            "a frame rendered between the release and the install replacing its recording: {events:?}"
        );
    }

    #[test]
    fn content_registers_an_image_on_its_third_frame_and_releases_it_while_mounted() {
        let mount = Mount::new();
        let mut content = LateImage::new();

        for _ in 0..2 {
            let report = mount.frame(&mut content);
            assert_eq!(report.drawn, Vec::<ImageId>::new());
            assert!(
                added_images(&report.events).is_empty(),
                "nothing is registered before the content draws it"
            );
        }

        let third = mount.frame(&mut content);
        let registered = added_images(&third.events);
        assert_eq!(
            registered.len(),
            1,
            "the third frame registers its image exactly once: {:?}",
            third.events
        );
        let id = registered[0];
        assert_registered_then_drawn(&third, id);
        assert_eq!(removed_images(&third.events), Vec::<ImageId>::new());

        // Asking for the same source while it is held shares the
        // registration instead of uploading it again.
        let shared = mount
            .resources
            .image(one_pixel())
            .expect("image registration");
        assert_eq!(Some(&shared), content.image.as_ref());
        drop(shared);

        let fourth = mount.frame(&mut content);
        assert_eq!(fourth.drawn, [id], "a held image keeps drawing");
        assert!(
            added_images(&fourth.events).is_empty(),
            "a held image is not registered again"
        );
        assert!(
            removed_images(&fourth.events).is_empty(),
            "dropping a shared clone must not release a registration the content still holds"
        );

        let fifth = mount.frame(&mut content);
        assert_eq!(fifth.drawn, Vec::<ImageId>::new());
        assert!(content.image.is_none());
        assert_released_then_replaced(&fifth, id);
        assert_eq!(
            mount.resources.listed(),
            0,
            "a released registration must leave the table, not linger as a dead entry"
        );

        // The same source is a new registration now, not the released one.
        let fresh = mount
            .resources
            .image(one_pixel())
            .expect("image registration");
        let fresh_id = mount.resources.recording().name(&fresh);
        assert_ne!(fresh_id, id);
        assert_eq!(added_images(&mount.render()), [fresh_id]);
        assert_eq!(mount.resources.listed(), 1);
        drop(fresh);
        assert_eq!(
            mount.resources.listed(),
            0,
            "dropping the last handle takes the entry out of the table"
        );
    }

    #[test]
    fn a_recording_not_yet_installed_cannot_release_what_the_installed_one_draws() {
        let mount = Mount::new();
        let mut content = LateImage::drawing_until(5);
        for _ in 0..3 {
            mount.frame(&mut content);
        }
        let fourth = mount.frame(&mut content);
        let [id] = fourth.drawn[..] else {
            panic!("the fourth frame draws the image: {:?}", fourth.drawn);
        };

        // The fifth recording names the image too, so its set shares the
        // registration the installed fourth recording holds. The host
        // discards it: dropping its share must not release what the
        // installed recording draws.
        let fifth = mount.record(&mut content);
        assert_eq!(fifth.drawn, [id]);
        assert!(
            !fifth.held.entries.is_empty(),
            "the discarded recording must hold the image for this step to test anything"
        );
        drop(fifth);
        assert!(
            removed_images(&mount.render()).is_empty(),
            "a discarded recording must not release what the installed one draws"
        );

        // The sixth recording lets the image go, and the host renders before
        // installing it: the fourth recording is still the one drawn.
        let sixth = mount.record(&mut content);
        assert_eq!(sixth.drawn, Vec::<ImageId>::new());
        assert!(content.image.is_none());
        let rendered = mount.render();
        assert!(
            rendered
                .iter()
                .any(|event| matches!(event, Event::Frame(_))),
            "{rendered:?}"
        );
        assert!(
            removed_images(&rendered).is_empty(),
            "the image was released while the installed recording still drew it: {rendered:?}"
        );

        // Installing it stops drawing the image, and releases it.
        mount.install(sixth);
        let events = mount.render();
        assert_released_then_replaced(
            &FrameReport {
                drawn: Vec::new(),
                events,
            },
            id,
        );
    }

    #[test]
    #[should_panic(expected = "registered on another target")]
    fn a_resource_from_another_target_cannot_be_named() {
        let (theirs, _events) = null_resources();
        let (ours, _events) = null_resources();
        let image = theirs.image(one_pixel()).expect("image registration");
        let _ = ours.recording().name(&image);
    }
}

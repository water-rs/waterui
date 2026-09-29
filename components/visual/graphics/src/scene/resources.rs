//! Engine-scoped resource registration for mounted scene content.
//!
//! A host builds one [`SceneResources`] from the engine it already selected —
//! the GPU engine a surface renders through, or the CPU raster engine an
//! offscreen rasterizer owns — and hands it to mounted content through
//! [`SceneContent::prepare_resources`]. Content registers its fonts, images
//! and shader paints once, keeps the returned handles for as long as it draws,
//! and the registration ends when the last handle drops: a content that
//! detaches and discards its handles unregisters its resources.
//!
//! The type deliberately carries no drawing methods, no layer operations and
//! no backend choice: the engine is the host's, selected before this exists.
//! What this adds on top of `Engine` is the mount's deduplication — two draws
//! of the same font or image inside one mounted scope share one registration
//! — and a uniform surface for backends with differing capabilities: a shader
//! paint on a backend without shader support is an explicit
//! [`ResourceError::Unsupported`], never a silent miss.
//!
//! [`SceneContent::prepare_resources`]: crate::scene_view::SceneContent::prepare_resources

use core::cell::RefCell;
use core::hash::{Hash, Hasher};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;

use cherenkov::{
    Backend, Engine, Font, FontSource, Image, ImageData, ResourceError, Rgba8, Rgba16F, Shader,
    ShaderPaintCapability, ShaderSource, Uploads,
};

/// Identity of a registered font: the data's content hash plus the index in
/// its collection, so two `Arc` copies of the same bytes share one
/// registration while a reused `Arc` address can never alias one.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FontKey {
    hash: u64,
    index: u32,
}

/// Identity of a registered image: the data's content hash plus the upload's
/// full shape — the same bytes at a different size are a different image.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ImageKey {
    hash: u64,
    width: u32,
    height: u32,
}

/// Identity of a registered shader: the source text's content hash plus the
/// `animated` flag — a static shader re-registered as animated would freeze
/// mid-frame if the two collided.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ShaderKey {
    hash: u64,
    animated: bool,
}

fn content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// The registration an engine always provides: fonts and image uploads.
///
/// Implemented for `cherenkov::Engine<B>` where `B` uploads both image
/// formats; `SceneResources` holds it as a trait object so the mount's
/// deduplication is backend-agnostic.
pub trait SceneBackend {
    /// Registers `source` with the engine, minting a live [`Font`] handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Font`] when the data cannot be used,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_font(&self, source: FontSource) -> Result<Font, ResourceError>;

    /// Uploads `data`, minting a live [`Image`] handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError>;

    /// Uploads `data` in the HDR/linear format, minting a live [`Image`]
    /// handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Image<Rgba16F>, ResourceError>;
}

impl<B> SceneBackend for Engine<B>
where
    B: Backend + Uploads<Rgba8> + Uploads<Rgba16F>,
{
    fn register_font(&self, source: FontSource) -> Result<Font, ResourceError> {
        Self::font(self, source)
    }

    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError> {
        Self::image(self, data)
    }

    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Image<Rgba16F>, ResourceError> {
        Self::image(self, data)
    }
}

/// The registration only a shader-paint backend provides.
pub trait ShaderBackend: SceneBackend {
    /// Registers `source` with the engine, minting a live [`Shader`] handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Shader`] when the source fails validation,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_shader(&self, source: ShaderSource) -> Result<Shader, ResourceError>;
}

impl<B> ShaderBackend for Engine<B>
where
    B: Backend + Uploads<Rgba8> + Uploads<Rgba16F> + ShaderPaintCapability,
{
    fn register_shader(&self, source: ShaderSource) -> Result<Shader, ResourceError> {
        Self::shader(self, source)
    }
}

/// What a backend offers beyond the unconditional registration, resolved on
/// the backend type so one `SceneResources::new` serves every engine.
mod sealed {
    use cherenkov::{Backend, Engine};

    /// Per-backend capabilities [`SceneResources`](super::SceneResources)
    /// surfaces.
    ///
    /// A backend declares them here; under-declaring simply reports
    /// `Unsupported` at registration.
    pub trait SceneCaps: Backend {
        /// The shader registry, when `Self` accepts shader paints.
        fn shaders(engine: &Engine<Self>) -> Option<&dyn super::ShaderBackend> {
            let _ = engine;
            None
        }
    }
}

pub use sealed::SceneCaps;

#[cfg(feature = "gpu")]
impl SceneCaps for cherenkov_gpu::Gpu {
    fn shaders(engine: &Engine<Self>) -> Option<&dyn ShaderBackend> {
        Some(engine)
    }
}

#[cfg(feature = "cpu")]
impl SceneCaps for cherenkov_cpu::Raster {}

#[cfg(test)]
impl SceneCaps for cherenkov::testing::Null {
    fn shaders(engine: &Engine<Self>) -> Option<&dyn ShaderBackend> {
        Some(engine)
    }
}

/// Resource registration shared by the content mounted on one engine.
///
/// Constructed from the host's already-selected engine; see the module
/// documentation for the ownership contract.
///
/// The table is mount-scoped: it keeps every registration it vends alive for
/// its own lifetime, because a `Weak` side of an engine handle does not exist
/// and a half-dead cache entry is worse than a dead one. A host therefore
/// drops the table when the mounted content detaches, which drops the last
/// handles and ends every registration — the mount/release contract.
pub struct SceneResources<'e> {
    backend: &'e dyn SceneBackend,
    shaders: Option<&'e dyn ShaderBackend>,
    fonts: RefCell<HashMap<FontKey, Font>>,
    images_rgba8: RefCell<HashMap<ImageKey, Image<Rgba8>>>,
    images_rgba16f: RefCell<HashMap<ImageKey, Image<Rgba16F>>>,
    shaders_cache: RefCell<HashMap<ShaderKey, Shader>>,
}

impl<'e> SceneResources<'e> {
    /// Resource registration over `engine`, the host's already-selected one.
    ///
    /// `B` declares which optional registries exist through its capability
    /// implementations: an engine that accepts shader paints gets them, an
    /// engine without the capability reports
    /// [`ResourceError::Unsupported`] instead.
    pub fn new<B>(engine: &'e Engine<B>) -> Self
    where
        B: SceneCaps + Uploads<Rgba8> + Uploads<Rgba16F>,
    {
        Self {
            backend: engine,
            shaders: B::shaders(engine),
            fonts: RefCell::new(HashMap::new()),
            images_rgba8: RefCell::new(HashMap::new()),
            images_rgba16f: RefCell::new(HashMap::new()),
            shaders_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Registers `source` with the engine, returning the shared handle.
    ///
    /// Within one `SceneResources`, identical sources — same data and same
    /// collection index — map to one registration, and callers keep the
    /// returned handle for as long as they draw with it.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Font`] when the data cannot be used,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn font(&self, source: FontSource) -> Result<Font, ResourceError> {
        let key = FontKey {
            hash: content_hash(&source.data),
            index: source.index,
        };
        if let Some(font) = self.fonts.borrow().get(&key) {
            return Ok(font.clone());
        }
        let font = self.backend.register_font(source)?;
        self.fonts.borrow_mut().insert(key, font.clone());
        Ok(font)
    }

    /// Registers `Rgba8` image data.
    ///
    /// [`ImageData::new`] validates the dimensions and byte length before the
    /// engine ever sees the upload; identical images inside one
    /// `SceneResources` share one registration.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn image(&self, data: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError> {
        let key = ImageKey {
            hash: content_hash(&data.data),
            width: data.width,
            height: data.height,
        };
        if let Some(image) = self.images_rgba8.borrow().get(&key) {
            return Ok(image.clone());
        }
        let image = self.backend.register_rgba8(data)?;
        self.images_rgba8.borrow_mut().insert(key, image.clone());
        Ok(image)
    }

    /// Registers `Rgba16Float` image data — the format HDR and linear-space
    /// sources upload as.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn image16f(&self, data: ImageData<Rgba16F>) -> Result<Image<Rgba16F>, ResourceError> {
        let key = ImageKey {
            hash: content_hash(&data.data),
            width: data.width,
            height: data.height,
        };
        if let Some(image) = self.images_rgba16f.borrow().get(&key) {
            return Ok(image.clone());
        }
        let image = self.backend.register_rgba16f(data)?;
        self.images_rgba16f.borrow_mut().insert(key, image.clone());
        Ok(image)
    }

    /// Registers a shader paint's WGSL source.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Unsupported`] when the engine's backend does not draw
    /// shader paints, [`ResourceError::Shader`] when the source fails
    /// validation, [`ResourceError::Lost`] when the render thread is gone.
    pub fn shader(&self, source: ShaderSource) -> Result<Shader, ResourceError> {
        let Some(backend) = self.shaders else {
            return Err(ResourceError::Unsupported("shader paint"));
        };
        let key = ShaderKey {
            hash: content_hash(source.source.as_bytes()),
            animated: source.animated,
        };
        if let Some(shader) = self.shaders_cache.borrow().get(&key) {
            return Ok(shader.clone());
        }
        let shader = backend.register_shader(source)?;
        self.shaders_cache.borrow_mut().insert(key, shader.clone());
        Ok(shader)
    }
}

impl core::fmt::Debug for SceneResources<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SceneResources")
            .field("shaders", &self.shaders.is_some())
            .finish_non_exhaustive()
    }
}
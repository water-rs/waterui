//! The engine-owned resources scene content records against.
//!
//! Cherenkov hands out fonts, images and shaders as handles minted by the
//! [`Engine`] that will draw them; a display list only names the ids. Content
//! that draws text, images or shader paints therefore records through a
//! [`Scene`], which pairs a live [`Recorder`] with the [`SceneResources`] of
//! the engine the backend renders with.

use alloc::rc::Rc;
use core::fmt;

use cherenkov::{
    Backend, Engine, Font, FontSource, Image, ImageData, Recorder, ResourceError, Rgba8, Shader,
    ShaderSource,
};
#[cfg(feature = "cpu")]
use cherenkov_cpu::Raster;

/// Mints engine-owned resource handles for content that records against it.
///
/// Implemented by engines whose backend defines its scene resource policy.
/// GPU backends register shaders through Cherenkov's capability contract;
/// CPU backends report unsupported GPU resources explicitly.
pub trait SceneResources: 'static {
    /// Registers a font from raw data.
    ///
    /// # Errors
    /// [`ResourceError`] when the data is not a font the engine can shape.
    fn font(&self, source: FontSource) -> Result<Font, ResourceError>;

    /// Uploads straight-alpha `Rgba8` texels.
    ///
    /// # Errors
    /// [`ResourceError`] when the upload is rejected.
    fn image(&self, image: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError>;

    /// Compiles a shader paint fragment.
    ///
    /// # Errors
    /// [`ResourceError`] when the WGSL does not compile.
    fn shader(&self, source: ShaderSource) -> Result<Shader, ResourceError>;
}

/// Resource operations available to backend-independent scene content.
pub trait SceneBackend: Backend {
    /// Uploads `image` to the engine.
    ///
    /// # Errors
    /// [`ResourceError`] when the backend refuses the upload.
    fn upload(
        engine: &Engine<Self>,
        image: ImageData<Rgba8>,
    ) -> Result<Image<Rgba8>, ResourceError>;

    /// Registers a shader paint with this backend.
    ///
    /// # Errors
    /// Returns the backend's shader compilation or capability error.
    fn shader(engine: &Engine<Self>, source: ShaderSource) -> Result<Shader, ResourceError>;
}

#[cfg(feature = "gpu")]
impl SceneBackend for cherenkov_gpu::Gpu {
    fn upload(
        engine: &Engine<Self>,
        image: ImageData<Rgba8>,
    ) -> Result<Image<Rgba8>, ResourceError> {
        Engine::<Self>::image(engine, image)
    }

    fn shader(engine: &Engine<Self>, source: ShaderSource) -> Result<Shader, ResourceError> {
        Engine::<Self>::shader(engine, source)
    }
}

#[cfg(feature = "cpu")]
impl SceneBackend for Raster {
    fn upload(
        _engine: &Engine<Self>,
        _image: ImageData<Rgba8>,
    ) -> Result<Image<Rgba8>, ResourceError> {
        Err(ResourceError::Unsupported(
            "the CPU raster backend takes no image uploads",
        ))
    }

    fn shader(_engine: &Engine<Self>, _source: ShaderSource) -> Result<Shader, ResourceError> {
        Err(ResourceError::Unsupported(
            "the CPU raster backend runs no shader paints",
        ))
    }
}

impl<B> SceneResources for Engine<B>
where
    B: SceneBackend,
{
    fn font(&self, source: FontSource) -> Result<Font, ResourceError> {
        Self::font(self, source)
    }

    fn image(&self, image: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError> {
        B::upload(self, image)
    }

    fn shader(&self, source: ShaderSource) -> Result<Shader, ResourceError> {
        B::shader(self, source)
    }
}

/// One recording of a scene: the recorder to draw into, the resources of the
/// engine that will draw it, and the size the content is laid out at.
pub struct Scene<'a> {
    recorder: &'a mut Recorder,
    resources: &'a Rc<dyn SceneResources>,
    width: f32,
    height: f32,
}

impl fmt::Debug for Scene<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scene")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl<'a> Scene<'a> {
    /// Pairs a recorder with the resources of the engine that renders it.
    pub fn new(
        recorder: &'a mut Recorder,
        resources: &'a Rc<dyn SceneResources>,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            recorder,
            resources,
            width,
            height,
        }
    }

    /// The recorder the content draws into.
    pub const fn recorder(&mut self) -> &mut Recorder {
        self.recorder
    }

    /// The engine's resources, shared so content may keep them.
    #[must_use]
    pub fn resources(&self) -> &Rc<dyn SceneResources> {
        self.resources
    }

    /// The width the content is laid out at, in points.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }

    /// The height the content is laid out at, in points.
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.height
    }
}

/// Resources for tests that record no fonts, images or shaders.
#[cfg(test)]
pub mod testing {
    use alloc::rc::Rc;

    use cherenkov::{
        Font, FontSource, Image, ImageData, ResourceError, Rgba8, Shader, ShaderSource,
    };

    use super::SceneResources;

    /// Resources that reject every request: for content that draws only
    /// shapes and paints.
    #[derive(Debug)]
    pub struct NoResources;

    impl SceneResources for NoResources {
        fn font(&self, _source: FontSource) -> Result<Font, ResourceError> {
            Err(ResourceError::Unsupported("test scene has no engine"))
        }

        fn image(&self, _image: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError> {
            Err(ResourceError::Unsupported("test scene has no engine"))
        }

        fn shader(&self, _source: ShaderSource) -> Result<Shader, ResourceError> {
            Err(ResourceError::Unsupported("test scene has no engine"))
        }
    }

    /// [`NoResources`] behind the shared handle a [`super::Scene`] takes.
    #[must_use]
    pub fn none() -> Rc<dyn SceneResources> {
        Rc::new(NoResources)
    }
}

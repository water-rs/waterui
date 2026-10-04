//! The resource-source vocabulary [`SceneResources`] registers: fonts,
//! images and shader paints.
//!
//! Nothing here names a render target's resource type — a `FontSource` is
//! the *description* of a font, and the target's [`SceneBackend`] answers
//! which descriptions it accepts. A description the target cannot honour is
//! rejected at registration with a [`ResourceError`] naming the problem,
//! never silently dropped.
//!
//! [`SceneResources`]: crate::resources::SceneResources
//! [`SceneBackend`]: crate::resources::SceneBackend

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::sync::Arc;
use std::path::Path;

/// The source of a font to register.
///
/// A font is either its file's bytes — with the face's index in the
/// collection — or a reference into the target's platform font stack, which
/// resolves a system family without bytes. `SceneBackend::register_font`
/// answers which the target accepts: a target without a platform font stack
/// rejects the reference with [`ResourceError::Font`].
#[derive(Clone)]
pub enum FontSource {
    /// The bytes of a font file, plus the face's index in a collection.
    Bytes {
        /// The font file's bytes.
        data: Arc<[u8]>,
        /// The face's index in the file's collection.
        index: u32,
    },
    /// A system font by family name, resolved through the target's platform
    /// font stack.
    System {
        /// The platform family name, e.g. `Roboto` or `SF Pro`.
        family: String,
    },
}

impl std::fmt::Debug for FontSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bytes { data, index } => f
                .debug_struct("FontSource")
                .field("len", &data.len())
                .field("index", index)
                .finish(),
            Self::System { family } => f
                .debug_struct("FontSource")
                .field("system", family)
                .finish(),
        }
    }
}

impl FontSource {
    /// A font already in memory.
    pub fn bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self::Bytes {
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

    /// A system font by platform family name.
    pub fn system(family: impl Into<String>) -> Self {
        Self::System {
            family: family.into(),
        }
    }

    /// Selects a face index inside the file's collection.
    ///
    /// # Panics
    /// When the source is a `System` reference, which has no collection.
    #[must_use]
    pub fn with_index(self, index: u32) -> Self {
        match self {
            Self::Bytes { data, .. } => Self::Bytes { data, index },
            Self::System { .. } => panic!("a system-font reference has no collection index"),
        }
    }
}

/// A shading language a [`ShaderSource`] may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShaderLanguage {
    /// WGSL, the Cherenkov GPU backend's language.
    Wgsl,
    /// AGSL — Android Graphics Shading Language.
    Agsl,
}

impl std::fmt::Display for ShaderLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wgsl => f.write_str("WGSL"),
            Self::Agsl => f.write_str("AGSL"),
        }
    }
}

/// The source of a shader paint to register: the fragment's text and the
/// language it is written in.
///
/// A target draws the languages it compiles; a source in any other language
/// is rejected at registration with [`ResourceError::Shader`] naming the
/// language it did get — never dropped silently.
#[derive(Clone)]
pub struct ShaderSource {
    /// The fragment's source text in `language`.
    pub source: Cow<'static, str>,
    /// The language `source` is written in.
    pub language: ShaderLanguage,
    /// Whether the fragment samples `uniforms.time`, so the engine
    /// re-renders it every frame.
    pub animated: bool,
}

impl std::fmt::Debug for ShaderSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShaderSource")
            .field("len", &self.source.len())
            .field("language", &self.language)
            .field("animated", &self.animated)
            .finish()
    }
}

impl ShaderSource {
    /// A WGSL fragment.
    pub fn wgsl(source: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: source.into(),
            language: ShaderLanguage::Wgsl,
            animated: false,
        }
    }

    /// An AGSL fragment.
    pub fn agsl(source: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: source.into(),
            language: ShaderLanguage::Agsl,
            animated: false,
        }
    }

    /// Marks the fragment as sampling `uniforms.time`.
    #[must_use]
    pub fn animated(self) -> Self {
        Self {
            animated: true,
            ..self
        }
    }
}

mod sealed {
    pub trait Sealed {}
}

/// An image storage format. The `F` parameter of [`ImageData`] — typed
/// because which formats a backend accepts is a static fact.
pub trait Format: sealed::Sealed + 'static {
    /// The untyped tag carried across the channel.
    const FORMAT: ImageFormat;
    /// The encoded byte length of a `width` × `height` image.
    fn bytes(width: u32, height: u32) -> usize;
}

/// Uncompressed premultiplied RGBA8.
#[derive(Clone, Copy, Debug)]
pub struct Rgba8;
impl sealed::Sealed for Rgba8 {}
impl Format for Rgba8 {
    const FORMAT: ImageFormat = ImageFormat::Rgba8;
    fn bytes(width: u32, height: u32) -> usize {
        (width as usize) * (height as usize) * 4
    }
}

/// Uncompressed `Rgba16Float` (8 bytes per texel).
#[derive(Clone, Copy, Debug)]
pub struct Rgba16F;
impl sealed::Sealed for Rgba16F {}
impl Format for Rgba16F {
    const FORMAT: ImageFormat = ImageFormat::Rgba16F;
    fn bytes(width: u32, height: u32) -> usize {
        (width as usize) * (height as usize) * 8
    }
}

/// The untyped tag of a [`Format`], carried across the channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// Uncompressed RGBA8.
    Rgba8,
    /// Uncompressed `Rgba16Float`.
    Rgba16F,
}

/// The colour space an image's texels are encoded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageColorSpace {
    /// sRGB primaries and transfer function.
    Srgb,
    /// Display P3 primaries with the sRGB transfer function.
    DisplayP3,
    /// Linear sRGB (extended values allowed).
    LinearSrgb,
    /// Linear Display P3 — the working space; decoding is the identity.
    LinearP3,
}

/// The data of an image to register, typed by its storage [`Format`].
///
/// The byte length is validated at construction, so a well-formed
/// `ImageData` always matches its dimensions.
pub struct ImageData<F: Format> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The texel data in `F`'s encoding.
    pub data: Arc<[u8]>,
    /// The encoded colour space.
    pub color_space: ImageColorSpace,
    /// Whether the texels carry premultiplied alpha.
    pub premultiplied: bool,
    /// The format tag.
    format: core::marker::PhantomData<F>,
}

impl<F: Format> std::fmt::Debug for ImageData<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageData")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("len", &self.data.len())
            .field("color_space", &self.color_space)
            .field("premultiplied", &self.premultiplied)
            .field("format", &F::FORMAT)
            .finish()
    }
}

impl<F: Format> ImageData<F> {
    /// A `width` × `height` image in `F`'s encoding.
    ///
    /// # Errors
    /// [`ResourceError::Image`] for a zero dimension or a byte-length
    /// mismatch.
    pub fn new(width: u32, height: u32, data: impl Into<Arc<[u8]>>) -> Result<Self, ResourceError> {
        let data = data.into();
        if width == 0 || height == 0 {
            return Err(ResourceError::Image("zero-size image".into()));
        }
        let want = F::bytes(width, height);
        if data.len() != want {
            return Err(ResourceError::Image(alloc::format!(
                "{width}x{height} {:?} needs {want} bytes, got {}",
                F::FORMAT,
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            data,
            color_space: ImageColorSpace::Srgb,
            premultiplied: false,
            format: core::marker::PhantomData,
        })
    }

    /// Sets the encoded colour space (default [`ImageColorSpace::Srgb`]).
    #[must_use]
    pub fn color_space(self, color_space: ImageColorSpace) -> Self {
        Self {
            color_space,
            ..self
        }
    }

    /// Marks the texels as premultiplied rather than straight alpha.
    #[must_use]
    pub fn premultiplied(self) -> Self {
        Self {
            premultiplied: true,
            ..self
        }
    }
}

/// Resource registration failure.
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// The font data could not be parsed, or the source is a kind the
    /// target does not register — a system-font reference on a target
    /// without a platform font stack.
    #[error("font: {0}")]
    Font(String),
    /// The image data is malformed or unsupported by the backend.
    #[error("image: {0}")]
    Image(String),
    /// The shader source is not in a language the target draws, or failed
    /// its validation.
    #[error("shader: {0}")]
    Shader(String),
    /// The resource needs a feature this backend does not implement; the
    /// string is the feature name.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// Reading the resource failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The render thread is gone.
    #[error("the render thread is gone")]
    Lost,
}

#[cfg(any(feature = "cherenkov", test))]
impl From<cherenkov::ResourceError> for ResourceError {
    fn from(error: cherenkov::ResourceError) -> Self {
        match error {
            cherenkov::ResourceError::Font(error) => Self::Font(error),
            cherenkov::ResourceError::Image(error) => Self::Image(error),
            cherenkov::ResourceError::Shader(error) => Self::Shader(error),
            cherenkov::ResourceError::Unsupported(what) => Self::Unsupported(what),
            cherenkov::ResourceError::Io(error) => Self::Io(error),
            cherenkov::ResourceError::Lost => Self::Lost,
        }
    }
}

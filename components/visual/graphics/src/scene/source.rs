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

// The image-data vocabulary — `ImageData`, its `Format` markers, the
// `ImageFormat` tag and `ImageColorSpace` — and the `ResourceError` a
// registration failure reports are defined once in `cherenkov-record`,
// shared with the Cherenkov engine, and re-exported here at their public
// paths.
pub use cherenkov_record::{
    Format, ImageColorSpace, ImageData, ImageFormat, ResourceError, Rgba8, Rgba16F,
};

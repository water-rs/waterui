//! Plain resource identifiers: the values recorded commands and updates
//! name for fonts, images, shaders and backdrop shaders. A render target
//! assigns them; the recording layer only carries them.

use crate::glyph::FontId;
use crate::paint::{ImageId, ShaderId};

/// A resource registered with a render target that installed content can
/// draw.
///
/// A target frees a released resource only once no installed content draws
/// it. A backdrop shader is never named by a command: content samples
/// fonts, images and shader paints only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceId {
    /// A font.
    Font(FontId),
    /// An image.
    Image(ImageId),
    /// A user shader.
    Shader(ShaderId),
    /// A backdrop effect shader.
    BackdropShader(BackdropShaderId),
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Font(id) => write!(f, "font {}", id.raw()),
            Self::Image(id) => write!(f, "image {}", id.raw()),
            Self::Shader(id) => write!(f, "shader {}", id.raw()),
            Self::BackdropShader(id) => write!(f, "backdrop shader {}", id.raw()),
        }
    }
}

/// Identifier of a backdrop effect shader, allocated by the target that
/// registers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BackdropShaderId(u64);

impl BackdropShaderId {
    /// Creates an identifier from a target-assigned raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

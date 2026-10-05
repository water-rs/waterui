//! The error resource registration reports back to the recording side.

use crate::resource::ImageLimits;

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
    /// The image exceeds the backend's [`ImageLimits`]. The check runs
    /// where the registration or replacement was made, before anything
    /// is queued: the content that made it sees this error, and a
    /// rejected replacement keeps the image's previous pixels.
    #[error("image {width}x{height} exceeds the image limits {limits}")]
    TooLarge {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// The backend's limits.
        limits: ImageLimits,
    },
    /// The shader source is not in a language the target draws, or failed
    /// its validation or pipeline creation.
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

/// Checks an image size against a backend's limits: `Ok` when it is
/// admitted, [`ResourceError::TooLarge`] naming the limits when it is
/// not. Runs wherever a registration or replacement is made.
///
/// # Errors
///
/// [`ResourceError::TooLarge`] with the requested size and the limits
/// checked against, when `limits` does not admit `width` × `height`.
pub fn admit(limits: ImageLimits, width: u32, height: u32) -> Result<(), ResourceError> {
    if limits.admits(width, height) {
        Ok(())
    } else {
        Err(ResourceError::TooLarge {
            width,
            height,
            limits,
        })
    }
}

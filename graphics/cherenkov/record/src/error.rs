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
    /// The image fits the backend's [`ImageLimits`] alone, but not beside
    /// the images already registered: together they would hold more than
    /// [`ImageLimits::max_bytes`]. Like [`TooLarge`](Self::TooLarge), the
    /// check runs where the registration or replacement was made, before
    /// anything is queued, and a rejected replacement keeps the image's
    /// previous pixels. An image's bytes return to the budget when its
    /// last handle drops.
    #[error(
        "image {width}x{height} does not fit the image budget: {available} of its {} bytes are left",
        limits.max_bytes
    )]
    OverBudget {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// The bytes left for this image: the budget less every other
        /// registered image, including the replaced image's previous
        /// pixels' share.
        available: u64,
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

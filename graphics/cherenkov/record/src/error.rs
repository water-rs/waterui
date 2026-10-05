//! The error resource registration reports back to the recording side.

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

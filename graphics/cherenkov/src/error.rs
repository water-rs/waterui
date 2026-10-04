//! Error types for the engine's front end.
//!
//! Each operation family has its own error enum. Backend-specific
//! "unsupported feature" enums stay in the backend crates and convert into
//! [`SurfaceError::UnsupportedTarget`], [`SurfaceError::UnsupportedFormat`]
//! or [`RenderError::Unsupported`]/[`ResourceError::Unsupported`], which
//! carry the feature name string the benchmark harness maps back to a scene
//! feature.

use std::sync::Arc;

use crate::frame::OffscreenFormat;
use crate::resource::ResourceId;

/// Engine initialization or engine-wide failure.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The backend failed to initialize: device or worker-pool creation.
    #[error("backend: {0}")]
    Backend(String),
    /// The render thread failed to spawn or died during initialization.
    #[error("render thread: {0}")]
    Thread(String),
}

/// Surface creation or surface-level failure.
#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    /// The engine failed while creating the surface.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The requested size exceeds the device limit.
    #[error("surface {width}x{height} exceeds the maximum texture size {max}")]
    TooLarge {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// Device maximum.
        max: u32,
    },
    /// The target kind is not drawable by this backend.
    #[error("unsupported surface target: {0}")]
    UnsupportedTarget(String),
    /// The requested [`OffscreenFormat`] is not renderable by this backend.
    #[error("unsupported offscreen format: {0:?}")]
    UnsupportedFormat(OffscreenFormat),
    /// A zero-size surface cannot hold a target.
    #[error("surface size must be non-zero")]
    ZeroSize,
    /// The render thread is gone.
    #[error("the render thread is gone")]
    Lost,
}

/// Resource registration failure.
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// The font data could not be parsed.
    #[error("font: {0}")]
    Font(String),
    /// The image data is malformed or unsupported by the backend.
    #[error("image: {0}")]
    Image(String),
    /// The shader source failed validation or pipeline creation.
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

/// Rendering or readback failure.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The GPU made no progress for a whole wait window — or, on wasm32,
    /// did not complete within the configured deadline.
    #[error("the GPU did not finish {what} within {timeout:?}")]
    Timeout {
        /// The operation waiting for completion.
        what: &'static str,
        /// The elapsed deadline.
        timeout: std::time::Duration,
    },
    /// The atlas needs to grow or clear before the frame can be committed.
    #[error("glyph atlas full")]
    AtlasFull,
    /// The frame's live coverage exceeds the maximum atlas capacity.
    #[error("glyph atlas exhausted")]
    AtlasExhausted,
    /// The frame needs a feature this backend does not implement; the
    /// string is the feature name the benchmark harness maps back to a
    /// scene feature.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// The device was lost.
    #[error("the device was lost")]
    DeviceLost,
    /// The render thread failed or stopped.
    #[error("render thread stopped")]
    Thread,
    /// Every surface of the engine is [`Hidden`](crate::Visibility::Hidden):
    /// there is nothing a frame may draw, and a host renders only while a
    /// surface is visible.
    #[error("every surface is hidden")]
    Hidden,
    /// The surface's pixels cannot be read back (window surfaces).
    #[error("the surface is not readable")]
    NotReadable,
    /// Pixel readback failed.
    #[error("readback: {0}")]
    Readback(String),
    /// A render pass failed.
    #[error("render: {0}")]
    Render(String),
    /// A glyph run references a font that is not registered.
    #[error("font: {0}")]
    Font(String),
    /// A shader paint references a shader that is not registered.
    #[error("shader: {0}")]
    Shader(String),
    /// A draw references an image that is not registered.
    #[error("image: {0}")]
    Image(String),
    /// A projective layer's sampled pose (its projection composed with
    /// its tilt, depth and affine components) is not a valid transform.
    #[error("layer {layer:?}: projective pose: {error}")]
    ProjectivePose {
        /// The projective layer.
        layer: crate::LayerId,
        /// Why the composed pose is invalid.
        error: crate::ProjectiveError,
    },
    /// A visible projective layer cannot be realized: its local image
    /// exceeds a resource limit, or a finite projection or sampling bound
    /// cannot be established in `f64`.
    #[error("layer {layer:?}: projective image unsupported: {reason}")]
    ProjectiveUnsupported {
        /// The projective layer.
        layer: crate::LayerId,
        /// What limit was exceeded, with the required size.
        reason: String,
    },
    /// A draw names a resource the backend rejected after its handle was
    /// returned: its registration, or an image's latest replacement.
    /// Every render that draws the resource fails this way until the
    /// resource is replaced successfully, or its last handle has dropped
    /// and no installed content draws it any more.
    #[error("{resource} was rejected by the backend: {reason}")]
    Rejected {
        /// The rejected resource.
        resource: ResourceId,
        /// The backend's reason.
        reason: Arc<ResourceError>,
    },
}

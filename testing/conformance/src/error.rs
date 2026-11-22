//! Error types for `waterui-conformance`.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Errors returned by conformance image I/O, perceptual diffing, gallery
/// emission, and offscreen capture runs.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConformanceError {
    /// Reading or writing an artifact file failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path that could not be read or written.
        path: PathBuf,
        /// Underlying OS I/O error.
        #[source]
        source: io::Error,
    },

    /// Decoding or encoding a PNG image failed.
    #[error("PNG image error at {path}: {source}")]
    Image {
        /// Path associated with the image operation.
        path: PathBuf,
        /// Underlying decoder or encoder error.
        #[source]
        source: image::ImageError,
    },

    /// An RGBA8 pixel buffer's length did not match `width * height * 4`.
    #[error(
        "invalid RGBA8 buffer length: expected {expected} bytes for {width}x{height}, got {actual}"
    )]
    InvalidImageBuffer {
        /// Declared width in pixels.
        width: u32,
        /// Declared height in pixels.
        height: u32,
        /// Expected byte length (`width * height * 4`).
        expected: usize,
        /// Actual byte length supplied.
        actual: usize,
    },

    /// Two images compared with FLIP had different dimensions.
    #[error(
        "image dimensions differ: reference is {ref_width}x{ref_height}, candidate is {test_width}x{test_height}"
    )]
    DimensionMismatch {
        /// Reference width in pixels.
        ref_width: u32,
        /// Reference height in pixels.
        ref_height: u32,
        /// Candidate width in pixels.
        test_width: u32,
        /// Candidate height in pixels.
        test_height: u32,
    },

    /// FLIP cannot compare empty (zero-width or zero-height) images.
    #[error("cannot run perceptual diff on empty image ({width}x{height})")]
    EmptyImage {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
    },

    /// Invalid FLIP configuration parameter (for example non-positive PPD or zero block size).
    #[error("invalid FLIP configuration: {0}")]
    InvalidConfig(&'static str),

    /// Serializing `report.json` failed.
    #[error("failed to serialize conformance report: {0}")]
    SerializeReport(#[from] serde_json::Error),

    /// Rendering the static HTML gallery template failed.
    #[error("failed to render HTML gallery template: {0}")]
    RenderTemplate(#[from] askama::Error),

    /// An offscreen `ViewRenderer` failed to capture a view.
    #[cfg(feature = "capture")]
    #[error("backend `{backend}` failed to render `{case}`: {source}")]
    BackendRender {
        /// Name of the backend that failed.
        backend: String,
        /// Case name that was being rendered.
        case: String,
        /// Underlying `ViewRenderer` error.
        #[source]
        source: waterui_capture::RenderError,
    },

    /// One or more stages failed conformance checks.
    #[error("conformance check failed: {failed_stages} of {total_stages} stage(s) failed")]
    ConformanceFailed {
        /// Number of stages that failed.
        failed_stages: usize,
        /// Total number of stages evaluated.
        total_stages: usize,
    },
}

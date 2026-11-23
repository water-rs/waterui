//! Error types for visual diffing, RGBA8 PNG image I/O, and gallery reporting.

use std::io;
use std::path::PathBuf;

use image::ImageError;
use png::EncodingError;
use thiserror::Error;

/// Errors returned by RGBA8/PNG image I/O, FLIP perceptual diffing, and HTML
/// gallery report generation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VisualDiffError {
    /// Reading or writing an image or gallery artifact file failed.
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
        source: ImageError,
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

    /// Encoding a PNG image stream failed.
    #[error("PNG encoding error: {0}")]
    EncodePng(#[from] EncodingError),

    /// Serializing `report.json` failed.
    #[error("failed to serialize gallery report: {0}")]
    SerializeReport(#[from] serde_json::Error),

    /// Rendering the static HTML gallery template failed.
    #[cfg(feature = "gallery")]
    #[error("failed to render HTML gallery template: {0}")]
    RenderTemplate(#[from] askama::Error),
}

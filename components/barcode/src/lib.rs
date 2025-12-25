//! GPU-accelerated barcode and QR code rendering.
//!
//! `filtrate-barcode` provides a [`Source`] implementation for generating
//! barcodes and QR codes that integrate with the filtrate pipeline.
//!
//! # Features
//!
//! - **Lazy generation**: QR code matrix computed on first render
//! - **GPU rendering**: Compute shader generates pixel-perfect output
//! - **Pipeline integration**: Works with `filtrate` filter chains
//!
//! # Example
//!
//! ```ignore
//! use filtrate_barcode::Barcode;
//! use filtrate::{Pipeline, Blur};
//!
//! // Create a QR code source
//! let barcode = Barcode::qr("https://waterui.dev");
//!
//! // Chain with filters and render
//! barcode
//!     .blur(5.0)
//!     .render(device, queue, &output);
//! ```

#![allow(clippy::multiple_crate_versions)]

mod qr;
mod renderer;
mod view;

pub use qr::BarcodeSource;
pub use renderer::BarcodeRenderer;
pub use view::Barcode;

// Re-export core types
pub use filtrate_core::{Pipeline, RenderContext, Source, SourceExt};

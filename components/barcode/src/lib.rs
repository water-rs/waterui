//! GPU-accelerated QR code rendering.
//!
//! This crate provides QR code generation and GPU rendering using compute shaders.
//! The QR matrix is generated using `fast_qr` and rendered directly on the GPU,
//! avoiding CPU-side pixel generation for optimal performance.
//!
//! # Architecture
//!
//! 1. **Matrix Generation**: `fast_qr` generates the QR matrix (which modules are dark/light)
//! 2. **GPU Upload**: Matrix data is packed into bits and uploaded to a storage buffer
//! 3. **Compute Shader**: Renders the QR code at any resolution directly on GPU
//! 4. **Blit**: Result is blitted to the frame
//!
//! # Example
//!
//! ```ignore
//! use waterui_barcode::Barcode;
//!
//! // Create a QR code view
//! Barcode::qr("https://waterui.dev")
//! ```

#![allow(clippy::multiple_crate_versions)]

mod qr;
mod renderer;
mod view;

pub use qr::{BarcodeSource, QrMatrix};
pub use renderer::BarcodeRenderer;
pub use view::Barcode;

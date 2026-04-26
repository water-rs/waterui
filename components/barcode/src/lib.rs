//! GPU-accelerated barcode rendering.
//!
//! This crate provides barcode generation and GPU rendering.
//! Encoded module data is packed into a bit buffer and rasterized directly
//! on GPU each frame.
//!
//! # Architecture
//!
//! 1. **Matrix Generation**: encoders generate module matrix data
//! 2. **GPU Upload**: Matrix data is packed into bits and uploaded to a storage buffer
//! 3. **Fragment Shader**: Renders barcodes at any resolution directly on GPU
//!
//! # Example
//!
//! ```ignore
//! use waterui_barcode::Barcode;
//!
//! // Create a QR code view
//! Barcode::qr("https://waterui.dev")
//!
//! // Create a Code128 barcode view
//! Barcode::code128("HELLO-WATERUI")
//! ```

mod effect;
mod qr;
mod renderer;
mod view;

pub use effect::BarcodeMaskEffect;
pub use qr::{BarcodeMatrix, BarcodeSource, BarcodeSymbology, QrMatrix};
pub use renderer::BarcodeRenderer;
pub use view::{Barcode, BarcodeFill, BarcodeGpuFill};

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
// Proving `Send` across `wgpu`'s generic type graph is deeper than rustc's
// default recursion limit of 128 on the workspace's nightly toolchain, which
// reports `overflow evaluating the requirement ...: Send` — a hard error under
// `-D warnings`. The bound genuinely holds; the solver just needs room to say
// so. Harmless on stable, where the limit is never reached.
#![recursion_limit = "256"]

mod effect;
mod qr;
mod renderer;
mod shaders;
mod view;

pub use effect::BarcodeMaskEffect;
pub use qr::{BarcodeError, BarcodeMatrix, BarcodeSource, BarcodeSymbology};
pub use renderer::BarcodeRenderer;
pub use view::{Barcode, BarcodeFill, BarcodeGpuFill, code128, qr_code};

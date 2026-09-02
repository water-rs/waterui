//! Real-time audio visualization components for `WaterUI`.
//!
//! This crate provides GPU-accelerated audio visualization using microphone input.
//!
//! # Views
//!
//! - [`Waveform`] - Time-domain oscilloscope display
//!
//! # Example
//!
//! ```ignore
//! use waterui_visualizer::{AudioCapture, Waveform};
//!
//! let capture = AudioCapture::new();
//! Waveform::new(capture)
//!     .sensitivity(1.5)
//!     .glow(0.8)
//! ```
// Proving `Send` across `wgpu`'s generic type graph is deeper than rustc's
// default recursion limit of 128 on the workspace's nightly toolchain, which
// reports `overflow evaluating the requirement ...: Send` — a hard error under
// `-D warnings`. The bound genuinely holds; the solver just needs room to say
// so. Harmless on stable, where the limit is never reached.
#![recursion_limit = "256"]

mod audio;
mod theme;
mod waveform;

use shaderloom::CompiledShader;

const WAVEFORM_SHADER: CompiledShader = include!(concat!(env!("OUT_DIR"), "/waveform.rs"));

pub use audio::AudioCapture;
pub use theme::WaveformTheme;
pub use waveform::{Waveform, waveform};

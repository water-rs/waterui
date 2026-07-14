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

mod audio;
mod theme;
mod waveform;

pub use audio::AudioCapture;
pub use theme::WaveformTheme;
pub use waveform::{Waveform, waveform};

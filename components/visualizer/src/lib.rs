//! Real-time audio visualization components for WaterUI.
//!
//! This crate provides GPU-accelerated audio visualization using microphone input.
//!
//! # Views
//!
//! - [`Waveform`] - Time-domain oscilloscope display
//! - [`Spectrum`] - Frequency spectrum bars (FFT)
//! - [`Spectrogram`] - Frequency heatmap over time
//! - [`PhaseScope`] - Stereo correlation (Lissajous)
//!
//! # Example
//!
//! ```ignore
//! use waterui_visualizer::Waveform;
//!
//! Waveform::new()
//!     .sensitivity(1.5)
//!     .glow(true)
//! ```

#![allow(clippy::multiple_crate_versions)]

mod audio;
mod theme;
mod waveform;
// mod spectrum;
// mod spectrogram;
// mod phase;

pub use theme::{WaveformTheme, SpectrumTheme};
pub use waveform::Waveform;
// pub use spectrum::Spectrum;
// pub use spectrogram::Spectrogram;
// pub use phase::PhaseScope;

pub use wgpu;

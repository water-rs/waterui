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
//! use waterui_visualizer::{AudioCapture, Waveform};
//!
//! let capture = AudioCapture::new();
//! Waveform::new(capture)
//!     .sensitivity(1.5)
//!     .glow(true)
//! ```

mod audio;
mod theme;
mod waveform;
// mod spectrum;
// mod spectrogram;
// mod phase;

pub use audio::AudioCapture;
pub use theme::{SpectrumTheme, WaveformTheme};
pub use waveform::Waveform;
// pub use spectrum::Spectrum;
// pub use spectrogram::Spectrogram;
// pub use phase::PhaseScope;

pub use wgpu;

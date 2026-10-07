//! Backend-neutral preview run contract.
//!
//! The CLI drives a generated preview binary — whatever backend realizes it —
//! with a single JSON run configuration, passed as a file path through
//! [`PREVIEW_RUN_CONFIG_ENV`], and every backend encodes its captures through
//! the same PNG writer.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Environment variable carrying the path of the JSON-encoded
/// [`PreviewRunConfig`] for the generated preview binary.
pub const PREVIEW_RUN_CONFIG_ENV: &str = "WATERUI_PREVIEW_RUN_CONFIG";

/// One offscreen preview invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewRunConfig {
    /// Viewport width in logical units.
    pub width: f32,
    /// Viewport height in logical units.
    pub height: f32,
    /// What the run produces.
    pub mode: PreviewRunMode,
}

/// What a preview run produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreviewRunMode {
    /// A single PNG capture after the view tree has mounted.
    Image {
        /// Destination PNG path.
        output: PathBuf,
    },
    /// A timeline capture: frames at `captures_ms` with `events` replayed at
    /// their timestamps.
    Scenario {
        /// Directory receiving `frame-XXXXms.png` captures.
        output_dir: PathBuf,
        /// Capture timestamps in milliseconds from scenario start.
        captures_ms: Vec<u64>,
        /// Input events sorted by timestamp.
        events: Vec<ScenarioEvent>,
    },
    /// Semantic accessibility-tree assertions (no render target).
    Semantic,
}

/// One input event in a preview scenario timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScenarioEvent {
    /// Event timestamp in milliseconds from scenario start.
    pub at_ms: u64,
    /// Event kind.
    pub kind: ScenarioEventKind,
    /// Pointer x coordinate in logical units.
    pub x: f32,
    /// Pointer y coordinate in logical units.
    pub y: f32,
    /// Pointer button for down/up events.
    pub button: ScenarioPointerButton,
    /// Scroll delta along the x axis for scroll events.
    pub dx: f32,
    /// Scroll delta along the y axis for scroll events.
    pub dy: f32,
    /// Whether scroll delta values are line units instead of logical units.
    pub is_line_delta: bool,
}

/// Event kind in a preview scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScenarioEventKind {
    /// Move the pointer without pressing.
    PointerMove,
    /// Press a pointer button.
    PointerDown,
    /// Release a pointer button.
    PointerUp,
    /// Cancel the active pointer.
    PointerCancel,
    /// Dispatch a wheel or trackpad scroll event.
    Scroll,
}

/// Pointer button identifier for scenario events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScenarioPointerButton {
    /// The primary button.
    #[default]
    Primary,
    /// The secondary button.
    Secondary,
    /// The middle button.
    Middle,
}

impl PreviewRunConfig {
    /// Reads the JSON configuration file [`PREVIEW_RUN_CONFIG_ENV`] names.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the variable is unset, the file cannot be
    /// read, or its contents are not a run configuration.
    pub fn load_from_env() -> Result<Self, RunConfigError> {
        let path = std::env::var_os(PREVIEW_RUN_CONFIG_ENV)
            .map(PathBuf::from)
            .ok_or(RunConfigError::MissingEnvVar)?;
        let text = std::fs::read_to_string(&path).map_err(|source| RunConfigError::Read {
            path: path.clone(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| RunConfigError::Json { path, source })
    }
}

/// Why [`PreviewRunConfig::load_from_env`] failed.
#[derive(Debug, thiserror::Error)]
pub enum RunConfigError {
    /// The environment variable naming the configuration file is unset.
    #[error("{PREVIEW_RUN_CONFIG_ENV} is not set")]
    MissingEnvVar,
    /// The configuration file could not be read.
    #[error("cannot read preview run configuration {}: {source}", .path.display())]
    Read {
        /// The path the read attempted.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The file is not a JSON [`PreviewRunConfig`].
    #[error("cannot parse preview run configuration {}: {source}", .path.display())]
    Json {
        /// The path the parse attempted.
        path: PathBuf,
        /// The underlying JSON error.
        source: serde_json::Error,
    },
}

/// Why a capture could not be encoded or written — [`encode_png`],
/// [`write_png`].
#[derive(Debug, thiserror::Error)]
pub enum PngError {
    /// The capture has no pixels — a zero-sized render is an error, not an
    /// empty file.
    #[error("the capture is empty ({width}x{height}, {len} bytes)")]
    Empty {
        /// Capture width in pixels.
        width: u32,
        /// Capture height in pixels.
        height: u32,
        /// Buffer length in bytes.
        len: usize,
    },
    /// The buffer is not `width * height * 4` bytes.
    #[error("invalid RGBA buffer: {len} bytes for {width}x{height}")]
    InvalidBuffer {
        /// Capture width in pixels.
        width: u32,
        /// Capture height in pixels.
        height: u32,
        /// Buffer length in bytes.
        len: usize,
    },
    /// The PNG encoder failed.
    #[error("PNG encoding failed: {0}")]
    Encode(#[source] image::ImageError),
    /// The output directory could not be created.
    #[error("cannot create capture directory {}: {source}", .path.display())]
    CreateDir {
        /// The directory the create attempted.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The PNG file write failed.
    #[error("cannot write capture to {}: {source}", .path.display())]
    Write {
        /// The path the write attempted.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// The alpha convention a capture's RGBA8 bytes carry — [`write_png`]
/// needs it to flatten correctly.
///
/// The convention comes from the surface that produced the bytes, not
/// from a comment: Apple's `CGBitmapContext` readout is
/// [`Alpha::Premultiplied`], while a Hydrolysis `HeadlessSnapshot`
/// reports its surface's `OutputAlpha` — `Straight` on the offscreen
/// path today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alpha {
    /// Colour channels carry colour premultiplied by alpha.
    Premultiplied,
    /// Colour channels carry unassociated colour; alpha lives only in the
    /// fourth byte.
    Straight,
}

/// Encodes a capture's RGBA8 pixels as PNG bytes.
///
/// The bytes are written as they arrive — this is a raw encode, so a
/// capture meant to sit on white flattens first; [`write_png`] does both
/// under the caller's declared [`Alpha`].
///
/// # Errors
///
/// Returns [`PngError`] when the capture is empty, the buffer size does not
/// match `width * height * 4`, or encoding fails.
pub fn encode_png(width: u32, height: u32, rgba_data: Vec<u8>) -> Result<Vec<u8>, PngError> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba};

    if rgba_data.is_empty() || width == 0 || height == 0 {
        return Err(PngError::Empty {
            width,
            height,
            len: rgba_data.len(),
        });
    }

    let len = rgba_data.len();
    let Some(img): Option<ImageBuffer<Rgba<u8>, _>> =
        ImageBuffer::from_raw(width, height, rgba_data)
    else {
        return Err(PngError::InvalidBuffer { width, height, len });
    };

    let mut png_bytes = Vec::new();
    // Preview favors encode speed over smallest file size.
    let encoder =
        PngEncoder::new_with_quality(&mut png_bytes, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(img.as_raw(), width, height, ExtendedColorType::Rgba8)
        .map_err(PngError::Encode)?;
    Ok(png_bytes)
}

/// Composites `alpha`-convention pixels onto opaque white, so captures
/// land on the same background a white window would have given them.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the rounded quotient is at most (255*255 + 127)/255 = 255"
)]
fn flatten_alpha_over_white(rgba_data: &mut [u8], alpha: Alpha) {
    for pixel in rgba_data.as_chunks_mut::<4>().0 {
        let a = u32::from(pixel[3]);
        let inv = 255 - a;
        for channel in &mut pixel[..3] {
            let c = u32::from(*channel);
            *channel = match alpha {
                // Premultiplied source-over white is exact — c + (255 − a)
                // — and a premultiplied channel never exceeds its alpha, so
                // the sum stays inside u8.
                Alpha::Premultiplied => {
                    u8::try_from(c + inv).expect("a premultiplied channel never exceeds its alpha")
                }
                // (C·A + 255·(255−A) + 127)/255 — the products fit u32, the
                // quotient u8.
                Alpha::Straight => ((c * a + 255 * inv + 127) / 255) as u8,
            };
        }
        pixel[3] = 255;
    }
}

/// Flattens `rgba_data` over white under its declared `alpha` convention,
/// encodes it as PNG and writes it to `output`, creating the
/// destination's parent directory when needed.
///
/// # Errors
///
/// Returns [`PngError`] when the capture is empty, the buffer is the wrong
/// size, encoding fails, or the file cannot be written.
pub fn write_png(
    output: &Path,
    width: u32,
    height: u32,
    mut rgba_data: Vec<u8>,
    alpha: Alpha,
) -> Result<(), PngError> {
    flatten_alpha_over_white(&mut rgba_data, alpha);
    let png = encode_png(width, height, rgba_data)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PngError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(output, &png).map_err(|source| PngError::Write {
        path: output.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::{Alpha, flatten_alpha_over_white};

    /// A translucent straight-alpha pixel whose colour channel exceeds its
    /// alpha — the anti-aliased light-ink case the premultiplied formula
    /// would panic on. Flattened over white its red lands at
    /// (200·128 + 255·127 + 127)/255 ≈ 227 and its zero channels at 127.
    #[test]
    fn straight_alpha_flattens_channels_past_alpha() {
        let mut pixels = vec![200_u8, 0, 0, 128];
        flatten_alpha_over_white(&mut pixels, Alpha::Straight);
        assert_eq!(
            pixels,
            [227, 127, 127, 255],
            "a straight channel above its alpha flattens without a panic"
        );
    }

    /// The same pixel under the premultiplied convention is a contract
    /// violation — the invariant documents itself by panicking.
    #[test]
    #[should_panic(expected = "a premultiplied channel never exceeds its alpha")]
    fn premultiplied_rejects_a_channel_above_alpha() {
        flatten_alpha_over_white(&mut [200, 0, 0, 128], Alpha::Premultiplied);
    }
}

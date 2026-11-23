//! Backend-neutral preview run contract.
//!
//! The CLI drives a generated preview binary — whatever backend realizes it —
//! with a single JSON run configuration, passed as a file path through
//! [`PREVIEW_RUN_CONFIG_ENV`], and every backend encodes its captures through
//! the same PNG writer.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use half::f16;
use png::chunk::cICP as CICP_CHUNK;
use png::{BitDepth, ColorType, Compression, Encoder as PngEncoder, EncodingError, Filter};
use serde::{Deserialize, Serialize};

/// Environment variable carrying the path of the JSON-encoded
/// [`PreviewRunConfig`] for the generated preview binary.
pub const PREVIEW_RUN_CONFIG_ENV: &str = "WATERUI_PREVIEW_RUN_CONFIG";

/// Default frame rate for `water preview --video` and `water preview test --video`.
pub const DEFAULT_VIDEO_FPS: u32 = 60;

/// PNG3 `cICP` payload for Display P3 SDR captures: Display P3 primaries (12),
/// sRGB transfer (13), RGB identity matrix (0), full range (1).
pub const CICP_DISPLAY_P3_SDR: [u8; 4] = [12, 13, 0, 1];

/// PNG3 `cICP` payload for Display P3 PQ HDR captures: Display P3 primaries
/// (12), SMPTE ST 2084 PQ transfer (16), RGB identity matrix (0), full range
/// (1).
pub const CICP_DISPLAY_P3_PQ_HDR: [u8; 4] = [12, 16, 0, 1];

/// ITU-R BT.2408 / Apple EDR SDR reference white luminance in nits (`1.0` in
/// extended linear Display P3).
pub const SDR_REFERENCE_WHITE_NITS: f32 = 203.0;

/// SMPTE ST 2084 PQ peak luminance in nits.
const PQ_PEAK_NITS: f32 = 10_000.0;

/// Magic header for version-1 raw RGBA8 Display P3 preview frame streams.
const RAW_FRAME_STREAM_MAGIC_V1: &[u8; 8] = b"WUIRF001";

/// Magic header for version-2 typed raw preview frame streams (carrying a
/// trailing [`RawPixelFormat`] byte after dimensions).
const RAW_FRAME_STREAM_MAGIC_V2: &[u8; 8] = b"WUIRF002";

/// How long a capture waits for owed content when the caller names no bound.
///
/// Owed content is what a view still loads after mount — a decoding image,
/// a `Suspense` still loading. The bound is long enough for a local decode
/// or a fetch on a slow link, short enough that a load that never lands
/// fails the run instead of hanging it.
pub const DEFAULT_CONTENT_DEADLINE: Duration = Duration::from_secs(10);

/// Default display scale factor shared across preview backends.
pub const DEFAULT_PREVIEW_SCALE: f32 = 2.0;

const fn default_scale() -> f32 {
    DEFAULT_PREVIEW_SCALE
}

/// Appearance override for an offscreen preview invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[expect(
    clippy::exhaustive_enums,
    reason = "every preview host realizes light and dark explicitly"
)]
pub enum PreviewColorScheme {
    /// Light appearance.
    Light,
    /// Dark appearance.
    Dark,
}

/// One offscreen preview invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PreviewRunConfig {
    /// Viewport width in logical units.
    width: f32,
    /// Viewport height in logical units.
    height: f32,
    /// What the run produces.
    mode: PreviewRunMode,
    /// How long, in milliseconds, an image or scenario capture waits for
    /// content its views owe after mount before the run fails naming them.
    content_deadline_ms: u64,
    /// Optional color scheme override (`None` follows the host default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color_scheme: Option<PreviewColorScheme>,
    /// Display scale factor applied to the captured image.
    #[serde(default = "default_scale")]
    scale: f32,
    /// Whether this preview run captures extended-dynamic-range (`--hdr`)
    /// Display P3 output (16-bit PQ PNGs and `Rgba16Float` linear P3 video
    /// streams) instead of 8-bit Display P3 SDR.
    #[serde(default, skip_serializing_if = "is_false")]
    hdr: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires a shared reference predicate"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

impl PreviewRunConfig {
    /// Viewport width in logical units.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }

    /// Viewport height in logical units.
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.height
    }

    /// What the run produces.
    #[must_use]
    pub const fn mode(&self) -> &PreviewRunMode {
        &self.mode
    }

    /// Takes what the run produces.
    #[must_use]
    pub fn into_mode(self) -> PreviewRunMode {
        self.mode
    }

    /// How long, in milliseconds, an image or scenario capture waits for
    /// content its views owe after mount before the run fails naming them.
    #[must_use]
    pub const fn content_deadline_ms(&self) -> u64 {
        self.content_deadline_ms
    }

    /// Optional color scheme override for the preview render.
    #[must_use]
    pub const fn color_scheme(&self) -> Option<PreviewColorScheme> {
        self.color_scheme
    }

    /// Display scale factor applied to the preview render.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// Whether extended-dynamic-range HDR capture is enabled (`--hdr`).
    #[must_use]
    pub const fn hdr(&self) -> bool {
        self.hdr
    }
}

/// Fixed-FPS raw RGBA8 frame capture configuration for video recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VideoCaptureConfig {
    /// Destination file path for the framed raw RGBA8 capture stream.
    output: PathBuf,
    /// Fixed sampling frame rate in frames per second (strictly positive).
    fps: u32,
    /// Total scenario timeline duration in milliseconds (`None` for semantic
    /// test runs, whose duration is determined by the test automation).
    #[serde(default)]
    duration_ms: Option<u64>,
}

impl VideoCaptureConfig {
    /// Creates a video capture configuration writing raw RGBA8 frames to
    /// `output` at `fps` frames per second.
    #[must_use]
    pub const fn new(output: PathBuf, fps: u32) -> Self {
        Self {
            output,
            fps,
            duration_ms: None,
        }
    }

    /// Sets the explicit timeline duration in milliseconds for scenario runs.
    #[must_use]
    pub const fn with_duration_ms(mut self, duration_ms: Option<u64>) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    /// Destination file path for the framed raw RGBA8 capture stream.
    #[must_use]
    pub fn output(&self) -> &Path {
        &self.output
    }

    /// Fixed sampling frame rate in frames per second.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }

    /// Explicit scenario timeline duration in milliseconds, if set.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }
}

/// What a preview run produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every preview host and the CLI realize each run mode variant by variant; a new mode must fail their build instead of falling into a wildcard arm"
)]
pub enum PreviewRunMode {
    /// A single PNG capture after the view tree has mounted.
    Image {
        /// Destination PNG path.
        output: PathBuf,
    },
    /// A timeline capture: PNG frames at `captures_ms` (when `output_dir` is
    /// set) and/or fixed-FPS raw RGBA8 frames into `video`, with `events`
    /// replayed at their timestamps.
    Scenario {
        /// Directory receiving `frame-XXXXms.png` captures, when PNG output is requested.
        #[serde(default)]
        output_dir: Option<PathBuf>,
        /// Capture timestamps in milliseconds from scenario start.
        #[serde(default)]
        captures_ms: Vec<u64>,
        /// Input events sorted by timestamp.
        #[serde(default)]
        events: Vec<ScenarioEvent>,
        /// Optional fixed-FPS raw RGBA8 frame stream for video recording.
        #[serde(default)]
        video: Option<VideoCaptureConfig>,
    },
    /// Semantic accessibility-tree assertions, optionally recording fixed-FPS
    /// raw RGBA8 frames when `video` is set.
    Semantic {
        /// Optional fixed-FPS raw RGBA8 frame stream recorded across the semantic test timeline.
        #[serde(default)]
        video: Option<VideoCaptureConfig>,
    },
}

/// One input event in a preview scenario timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ScenarioEvent {
    /// Event timestamp in milliseconds from scenario start.
    at_ms: u64,
    /// Event kind.
    kind: ScenarioEventKind,
    /// Pointer x coordinate in logical units.
    x: f32,
    /// Pointer y coordinate in logical units.
    y: f32,
    /// Pointer button for down/up events.
    button: ScenarioPointerButton,
    /// Scroll delta along the x axis for scroll events.
    dx: f32,
    /// Scroll delta along the y axis for scroll events.
    dy: f32,
    /// Whether scroll delta values are line units instead of logical units.
    is_line_delta: bool,
}

impl ScenarioEvent {
    /// Event timestamp in milliseconds from scenario start.
    #[must_use]
    pub const fn at_ms(&self) -> u64 {
        self.at_ms
    }

    /// Event kind.
    #[must_use]
    pub const fn kind(&self) -> ScenarioEventKind {
        self.kind
    }

    /// Pointer x coordinate in logical units.
    #[must_use]
    pub const fn x(&self) -> f32 {
        self.x
    }

    /// Pointer y coordinate in logical units.
    #[must_use]
    pub const fn y(&self) -> f32 {
        self.y
    }

    /// Pointer button for down/up events.
    #[must_use]
    pub const fn button(&self) -> ScenarioPointerButton {
        self.button
    }

    /// Scroll delta along the x axis for scroll events.
    #[must_use]
    pub const fn dx(&self) -> f32 {
        self.dx
    }

    /// Scroll delta along the y axis for scroll events.
    #[must_use]
    pub const fn dy(&self) -> f32 {
        self.dy
    }

    /// Whether scroll delta values are line units instead of logical units.
    #[must_use]
    pub const fn is_line_delta(&self) -> bool {
        self.is_line_delta
    }
}

impl ScenarioEvent {
    /// A `kind` event at `x`, `y` and `at_ms` from scenario start, with the
    /// primary button and no scroll delta.
    #[must_use]
    pub const fn new(at_ms: u64, kind: ScenarioEventKind, x: f32, y: f32) -> Self {
        Self {
            at_ms,
            kind,
            x,
            y,
            button: ScenarioPointerButton::Primary,
            dx: 0.0,
            dy: 0.0,
            is_line_delta: false,
        }
    }

    /// Presses or releases `button`.
    #[must_use]
    pub const fn with_button(mut self, button: ScenarioPointerButton) -> Self {
        self.button = button;
        self
    }

    /// Scrolls by `dx`, `dy`, in lines when `is_line_delta` and in logical
    /// units otherwise.
    #[must_use]
    pub const fn with_scroll_delta(mut self, dx: f32, dy: f32, is_line_delta: bool) -> Self {
        self.dx = dx;
        self.dy = dy;
        self.is_line_delta = is_line_delta;
        self
    }
}

/// Event kind in a preview scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
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
#[non_exhaustive]
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
    /// A configuration for a `width`×`height` viewport producing `mode`,
    /// whose captures wait up to `content_deadline` for owed content.
    ///
    /// # Panics
    ///
    /// Panics if `content_deadline` does not fit `u64` milliseconds.
    #[must_use]
    pub fn new(width: f32, height: f32, mode: PreviewRunMode, content_deadline: Duration) -> Self {
        Self {
            width,
            height,
            mode,
            content_deadline_ms: u64::try_from(content_deadline.as_millis())
                .expect("a preview content deadline fits u64 milliseconds"),
            color_scheme: None,
            scale: DEFAULT_PREVIEW_SCALE,
            hdr: false,
        }
    }

    /// Sets the appearance override for this preview run.
    #[must_use]
    pub const fn with_color_scheme(mut self, color_scheme: PreviewColorScheme) -> Self {
        self.color_scheme = Some(color_scheme);
        self
    }

    /// Sets or clears the appearance override for this preview run.
    #[must_use]
    pub const fn with_optional_color_scheme(
        mut self,
        color_scheme: Option<PreviewColorScheme>,
    ) -> Self {
        self.color_scheme = color_scheme;
        self
    }

    /// Sets the display scale factor for this preview run.
    #[must_use]
    pub const fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Enables or disables extended-dynamic-range HDR capture (`--hdr`).
    #[must_use]
    pub const fn with_hdr(mut self, hdr: bool) -> Self {
        self.hdr = hdr;
        self
    }

    /// How long a capture waits for owed content.
    #[must_use]
    pub const fn content_deadline(&self) -> Duration {
        Duration::from_millis(self.content_deadline_ms)
    }

    /// Reads the JSON configuration file [`PREVIEW_RUN_CONFIG_ENV`] names.
    ///
    /// A relative `output`/`output_dir` resolves against the configuration
    /// file's own directory, so a host that cannot name the payload's
    /// absolute root — a device-side staging area under private files —
    /// writes its outputs run-relative.
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
        let mut config: Self =
            serde_json::from_str(&text).map_err(|source| RunConfigError::Json {
                path: path.clone(),
                source,
            })?;
        if let Some(base) = path.parent() {
            match &mut config.mode {
                PreviewRunMode::Image { output } if !output.is_absolute() => {
                    *output = base.join(&*output);
                }
                PreviewRunMode::Scenario {
                    output_dir, video, ..
                } => {
                    if let Some(dir) = output_dir
                        && !dir.is_absolute()
                    {
                        *dir = base.join(&*dir);
                    }
                    if let Some(video) = video
                        && !video.output.is_absolute()
                    {
                        video.output = base.join(&video.output);
                    }
                }
                PreviewRunMode::Semantic { video } => {
                    if let Some(video) = video
                        && !video.output.is_absolute()
                    {
                        video.output = base.join(&video.output);
                    }
                }
                PreviewRunMode::Image { .. } => {}
            }
        }
        Ok(config)
    }
}

/// Why [`PreviewRunConfig::load_from_env`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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
#[non_exhaustive]
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
    Encode(#[source] EncodingError),
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
#[non_exhaustive]
pub enum Alpha {
    /// Colour channels carry colour premultiplied by alpha.
    Premultiplied,
    /// Colour channels carry unassociated colour; alpha lives only in the
    /// fourth byte.
    Straight,
}

/// Encodes a capture's Display P3 RGBA8 pixels as PNG bytes tagged with the
/// Display P3 SDR `cICP` chunk (`[12, 13, 0, 1]`).
///
/// The bytes are written as they arrive — this is a raw encode, so a
/// capture meant to sit on white flattens first; [`write_png`] does both
/// under the caller's declared [`Alpha`].
///
/// # Errors
///
/// Returns [`PngError`] when the capture is empty, the buffer size does not
/// match `width * height * 4`, or encoding fails.
#[expect(
    clippy::needless_pass_by_value,
    reason = "preserves the public encode_png signature for callers holding owned capture buffers"
)]
pub fn encode_png(width: u32, height: u32, rgba_data: Vec<u8>) -> Result<Vec<u8>, PngError> {
    if rgba_data.is_empty() || width == 0 || height == 0 {
        return Err(PngError::Empty {
            width,
            height,
            len: rgba_data.len(),
        });
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if expected_len != Some(rgba_data.len()) {
        return Err(PngError::InvalidBuffer {
            width,
            height,
            len: rgba_data.len(),
        });
    }

    encode_cicp_png(
        width,
        height,
        BitDepth::Eight,
        CICP_DISPLAY_P3_SDR,
        &rgba_data,
    )
}

/// Converts a single linear extended-dynamic-range channel value (`1.0` =
/// [`SDR_REFERENCE_WHITE_NITS`] = 203 nits) into a normalized `[0.0, 1.0]`
/// SMPTE ST 2084 PQ code value (peaking at 10,000 nits).
#[must_use]
pub fn linear_edr_to_pq(linear_edr: f32) -> f32 {
    // SMPTE ST 2084-2014 EOTF^-1 constants.
    const M1: f64 = 2610.0 / 16384.0;
    const M2: f64 = (2523.0 / 4096.0) * 128.0;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = (2413.0 / 4096.0) * 32.0;
    const C3: f64 = (2392.0 / 4096.0) * 32.0;

    if !linear_edr.is_finite() || linear_edr <= 0.0 {
        return 0.0;
    }
    let nits =
        (f64::from(linear_edr) * f64::from(SDR_REFERENCE_WHITE_NITS)).min(f64::from(PQ_PEAK_NITS));
    let y = nits / f64::from(PQ_PEAK_NITS);
    let ym1 = y.powf(M1);
    let pq = (C2.mul_add(ym1, C1) / C3.mul_add(ym1, 1.0)).powf(M2);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "PQ code is normalized in [0.0, 1.0]"
    )]
    let out = pq.clamp(0.0, 1.0) as f32;
    out
}

/// Encodes flattened `ExtendedLinearDisplayP3` RGBA `f32` pixels (`1.0` = 203
/// nits SDR reference white, `> 1.0` = HDR highlights) as a 16-bit Display P3
/// PQ PNG tagged with `cICP` (`[12, 16, 0, 1]`).
///
/// # Errors
///
/// Returns [`PngError`] when the capture is empty, the buffer length does not
/// equal `width * height * 4`, or PNG encoding fails.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "normalized [0.0, 1.0] values scaled by 65535.0 fit u16"
)]
pub fn encode_hdr_png(
    width: u32,
    height: u32,
    rgba_linear_p3: &[f32],
) -> Result<Vec<u8>, PngError> {
    if rgba_linear_p3.is_empty() || width == 0 || height == 0 {
        return Err(PngError::Empty {
            width,
            height,
            len: size_of_val(rgba_linear_p3),
        });
    }

    let expected_floats = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if expected_floats != Some(rgba_linear_p3.len()) {
        return Err(PngError::InvalidBuffer {
            width,
            height,
            len: size_of_val(rgba_linear_p3),
        });
    }

    let mut rgba16_be = Vec::with_capacity(rgba_linear_p3.len() * 2);
    for pixel in rgba_linear_p3.as_chunks::<4>().0 {
        let r = (linear_edr_to_pq(pixel[0]) * 65535.0).round() as u16;
        let g = (linear_edr_to_pq(pixel[1]) * 65535.0).round() as u16;
        let b = (linear_edr_to_pq(pixel[2]) * 65535.0).round() as u16;
        let a = (pixel[3].clamp(0.0, 1.0) * 65535.0).round() as u16;
        rgba16_be.extend_from_slice(&r.to_be_bytes());
        rgba16_be.extend_from_slice(&g.to_be_bytes());
        rgba16_be.extend_from_slice(&b.to_be_bytes());
        rgba16_be.extend_from_slice(&a.to_be_bytes());
    }

    encode_cicp_png(
        width,
        height,
        BitDepth::Sixteen,
        CICP_DISPLAY_P3_PQ_HDR,
        &rgba16_be,
    )
}

fn encode_cicp_png(
    width: u32,
    height: u32,
    depth: BitDepth,
    cicp: [u8; 4],
    pixel_bytes: &[u8],
) -> Result<Vec<u8>, PngError> {
    let mut png_bytes = Vec::new();
    let mut encoder = PngEncoder::new(&mut png_bytes, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(depth);
    // Preview favors encode speed over smallest file size.
    encoder.set_compression(Compression::Fast);
    encoder.set_filter(Filter::NoFilter);
    let mut writer = encoder.write_header().map_err(PngError::Encode)?;
    // PNG3 requires `cICP` before `PLTE` and `IDAT`.
    writer
        .write_chunk(CICP_CHUNK, &cicp)
        .map_err(PngError::Encode)?;
    writer
        .write_image_data(pixel_bytes)
        .map_err(PngError::Encode)?;
    writer.finish().map_err(PngError::Encode)?;
    Ok(png_bytes)
}

/// Composites `alpha`-convention linear Display P3 pixels onto opaque SDR
/// reference white `(1.0, 1.0, 1.0)` without clamping HDR highlights above
/// `1.0`.
fn flatten_linear_over_white(rgba_linear: &mut [f32], alpha: Alpha) {
    for pixel in rgba_linear.as_chunks_mut::<4>().0 {
        let a = pixel[3].clamp(0.0, 1.0);
        let inv = 1.0 - a;
        for channel in &mut pixel[..3] {
            let c = if channel.is_finite() { *channel } else { 0.0 };
            *channel = match alpha {
                Alpha::Premultiplied => c + inv,
                Alpha::Straight => c.mul_add(a, inv),
            };
        }
        pixel[3] = 1.0;
    }
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

/// Writes a Display P3 SDR PNG (`cICP [12, 13, 0, 1]`) to `output`.
///
/// Flattens `rgba_data` over white under `alpha` and creates the parent
/// directory when needed.
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
    write_png_bytes(output, &png)
}

/// Flattens `ExtendedLinearDisplayP3` RGBA `f32` pixels over SDR white
/// `(1.0, 1.0, 1.0)` under `alpha`, encodes a 16-bit Display P3 PQ HDR PNG
/// (`cICP [12, 16, 0, 1]`), and writes it to `output`.
///
/// # Errors
///
/// Returns [`PngError`] when the capture is empty, the buffer is the wrong
/// size, encoding fails, or the file cannot be written.
pub fn write_hdr_png(
    output: &Path,
    width: u32,
    height: u32,
    mut rgba_linear_p3: Vec<f32>,
    alpha: Alpha,
) -> Result<(), PngError> {
    flatten_linear_over_white(&mut rgba_linear_p3, alpha);
    let png = encode_hdr_png(width, height, &rgba_linear_p3)?;
    write_png_bytes(output, &png)
}

/// Writes a 16-bit Display P3 PQ HDR PNG (`cICP [12, 16, 0, 1]`) from
/// little-endian IEEE-754 `f16` `ExtendedLinearDisplayP3` RGBA bytes.
///
/// Flattens over SDR white `(1.0, 1.0, 1.0)` under `alpha` and creates the
/// parent directory when needed.
///
/// # Errors
///
/// Returns [`PngError`] when the capture is empty, the buffer length does not
/// match `width * height * 8`, encoding fails, or the file cannot be written.
pub fn write_hdr_png_f16(
    output: &Path,
    width: u32,
    height: u32,
    rgba16f_bytes: &[u8],
    alpha: Alpha,
) -> Result<(), PngError> {
    let rgba_f32 = decode_rgba16f_le(width, height, rgba16f_bytes)?;
    write_hdr_png(output, width, height, rgba_f32, alpha)
}

fn decode_rgba16f_le(width: u32, height: u32, rgba16f_bytes: &[u8]) -> Result<Vec<f32>, PngError> {
    if rgba16f_bytes.is_empty() || width == 0 || height == 0 {
        return Err(PngError::Empty {
            width,
            height,
            len: rgba16f_bytes.len(),
        });
    }
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(8));
    if expected_len != Some(rgba16f_bytes.len()) {
        return Err(PngError::InvalidBuffer {
            width,
            height,
            len: rgba16f_bytes.len(),
        });
    }
    Ok(rgba16f_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&bits| f16::from_le_bytes(bits).to_f32())
        .collect())
}

fn write_png_bytes(output: &Path, png: &[u8]) -> Result<(), PngError> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| PngError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(output, png).map_err(|source| PngError::Write {
        path: output.to_path_buf(),
        source,
    })
}

/// One scheduled capture point on a scenario timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScenarioSample {
    /// Exact offset from scenario start.
    offset: Duration,
    /// When `Some(ms)`, write `frame-{ms:04}ms.png` to the scenario output directory.
    png_capture_ms: Option<u64>,
    /// Whether this capture appends a frame to the video raw frame stream.
    record_video: bool,
}

impl ScenarioSample {
    /// Exact offset from scenario start.
    #[must_use]
    pub const fn offset(&self) -> Duration {
        self.offset
    }

    /// Timestamp in milliseconds for the PNG frame, when this sample writes one.
    #[must_use]
    pub const fn png_capture_ms(&self) -> Option<u64> {
        self.png_capture_ms
    }

    /// Whether this sample writes a frame into the video stream.
    #[must_use]
    pub const fn record_video(&self) -> bool {
        self.record_video
    }
}

/// Exact timeline offset for `frame_index` sampled at `fps` frames per second.
///
/// # Panics
///
/// Panics if `fps` is zero.
#[must_use]
pub fn frame_offset(frame_index: u64, fps: u32) -> Duration {
    assert!(fps > 0, "video frame rate must be greater than zero");
    let nanos = (u128::from(frame_index) * 1_000_000_000) / u128::from(fps);
    Duration::from_nanos(u64::try_from(nanos).expect("scenario timeline fits u64 nanoseconds"))
}

/// Plans the ordered capture samples for a scenario run, coalescing matching
/// PNG capture timestamps and fixed-FPS video sample instants into a single
/// timeline.
///
/// # Errors
///
/// Returns [`RawFrameError::InvalidFps`] when `video` specifies `fps == 0`,
/// or [`RawFrameError::EmptyTimeline`] when neither PNG captures nor video
/// recording are configured.
pub fn plan_scenario_timeline(
    captures_ms: &[u64],
    video: Option<&VideoCaptureConfig>,
    events: &[ScenarioEvent],
) -> Result<Vec<ScenarioSample>, RawFrameError> {
    let mut merged: BTreeMap<Duration, (Option<u64>, bool)> = BTreeMap::new();

    for &capture_ms in captures_ms {
        let offset = Duration::from_millis(capture_ms);
        merged.entry(offset).or_insert((None, false)).0 = Some(capture_ms);
    }

    if let Some(video) = video {
        if video.fps == 0 {
            return Err(RawFrameError::InvalidFps);
        }
        let inferred_ms = captures_ms
            .iter()
            .copied()
            .chain(events.iter().map(ScenarioEvent::at_ms))
            .max()
            .unwrap_or(0);
        let limit_nanos = u128::from(video.duration_ms.unwrap_or(inferred_ms)) * 1_000_000;
        let mut frame_index = 0_u64;
        loop {
            let offset_ns = (u128::from(frame_index) * 1_000_000_000) / u128::from(video.fps);
            if offset_ns > limit_nanos {
                break;
            }
            let Ok(nanos) = u64::try_from(offset_ns) else {
                break;
            };
            let offset = Duration::from_nanos(nanos);
            merged.entry(offset).or_insert((None, false)).1 = true;
            frame_index += 1;
        }
    }

    if merged.is_empty() {
        return Err(RawFrameError::EmptyTimeline);
    }

    Ok(merged
        .into_iter()
        .map(|(offset, (png_capture_ms, record_video))| ScenarioSample {
            offset,
            png_capture_ms,
            record_video,
        })
        .collect())
}

/// Pixel format carried by a `WUIRF` raw preview frame stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawPixelFormat {
    /// Display P3 8-bit opaque RGBA (4 bytes per pixel).
    Rgba8DisplayP3,
    /// `ExtendedLinearDisplayP3` IEEE-754 half-float (`f16` little-endian)
    /// opaque RGBA (8 bytes per pixel; `1.0` = 203 nits SDR reference white).
    Rgba16FloatLinearDisplayP3,
}

impl RawPixelFormat {
    /// Bytes stored per pixel in the frame payload.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8DisplayP3 => 4,
            Self::Rgba16FloatLinearDisplayP3 => 8,
        }
    }

    /// Whether this format carries extended-dynamic-range linear HDR pixels.
    #[must_use]
    pub const fn is_hdr(self) -> bool {
        matches!(self, Self::Rgba16FloatLinearDisplayP3)
    }

    const fn wire_code(self) -> u8 {
        match self {
            Self::Rgba8DisplayP3 => 0,
            Self::Rgba16FloatLinearDisplayP3 => 1,
        }
    }

    const fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Rgba8DisplayP3),
            1 => Some(Self::Rgba16FloatLinearDisplayP3),
            _ => None,
        }
    }
}

/// Why reading or writing a raw preview frame stream failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RawFrameError {
    /// Video frame rate must be strictly positive.
    #[error("video frame rate must be greater than zero")]
    InvalidFps,
    /// Scenario specified neither PNG captures nor video recording.
    #[error("scenario timeline has no PNG captures and no video recording configured")]
    EmptyTimeline,
    /// The capture frame has zero width, zero height, or no pixels.
    #[error("video frame is empty ({width}x{height}, {len} bytes)")]
    EmptyFrame {
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
        /// Buffer length in bytes.
        len: usize,
    },
    /// Frame buffer length does not match the expected pixel format byte size.
    #[error("invalid raw frame buffer: {len} bytes for {width}x{height}")]
    InvalidBuffer {
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
        /// Buffer length in bytes.
        len: usize,
    },
    /// Attempted to write a frame whose pixel format does not match the stream's format.
    #[error("raw frame stream expects {expected:?} frames, got {actual:?}")]
    FormatMismatch {
        /// Format the writer was opened with.
        expected: RawPixelFormat,
        /// Format the caller attempted to write.
        actual: RawPixelFormat,
    },
    /// A subsequent frame changed dimensions mid-stream.
    #[error(
        "video frame dimensions changed mid-stream from {expected_width}x{expected_height} to {actual_width}x{actual_height}"
    )]
    DimensionMismatch {
        /// Width recorded by the first frame.
        expected_width: u32,
        /// Height recorded by the first frame.
        expected_height: u32,
        /// Width of the mismatched frame.
        actual_width: u32,
        /// Height of the mismatched frame.
        actual_height: u32,
    },
    /// Stream finished with zero frames written.
    #[error("video capture produced no frames")]
    NoFrames,
    /// Input file is not a valid `WUIRF001` or `WUIRF002` raw frame stream.
    #[error("invalid raw frame stream header in {}", .path.display())]
    InvalidHeader {
        /// File path whose header failed validation.
        path: PathBuf,
    },
    /// Output directory creation failed.
    #[error("cannot create video capture directory {}: {source}", .path.display())]
    CreateDir {
        /// Directory that could not be created.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// Underlying file I/O failed.
    #[error("raw frame stream I/O failed for {}: {source}", .path.display())]
    Io {
        /// File path being read or written.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
}

/// Streams flattened opaque Display P3 frames to disk for host-side encoding.
///
/// Uses `WUIRF001` for 8-bit Display P3 SDR streams and `WUIRF002` for typed
/// SDR/HDR (`Rgba16Float` linear Display P3) streams.
#[derive(Debug)]
pub struct RawFrameStreamWriter {
    path: PathBuf,
    writer: BufWriter<File>,
    fps: u32,
    pixel_format: RawPixelFormat,
    dimensions: Option<(u32, u32)>,
    frame_count: u64,
}

impl RawFrameStreamWriter {
    /// Creates a new 8-bit Display P3 (`WUIRF001`) raw frame stream at `path`
    /// for captures at `fps`.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError::InvalidFps`] when `fps == 0`, or an I/O error
    /// if the parent directory or file cannot be created.
    pub fn create(path: impl AsRef<Path>, fps: u32) -> Result<Self, RawFrameError> {
        Self::create_with_format(path, fps, RawPixelFormat::Rgba8DisplayP3)
    }

    /// Creates a new HDR `Rgba16Float` `ExtendedLinearDisplayP3` (`WUIRF002`)
    /// raw frame stream at `path` for captures at `fps`.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError::InvalidFps`] when `fps == 0`, or an I/O error
    /// if the parent directory or file cannot be created.
    pub fn create_hdr(path: impl AsRef<Path>, fps: u32) -> Result<Self, RawFrameError> {
        Self::create_with_format(path, fps, RawPixelFormat::Rgba16FloatLinearDisplayP3)
    }

    /// Creates a new raw frame stream at `path` for `pixel_format` captures at
    /// `fps`.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError::InvalidFps`] when `fps == 0`, or an I/O error
    /// if the parent directory or file cannot be created.
    pub fn create_with_format(
        path: impl AsRef<Path>,
        fps: u32,
        pixel_format: RawPixelFormat,
    ) -> Result<Self, RawFrameError> {
        if fps == 0 {
            return Err(RawFrameError::InvalidFps);
        }
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| RawFrameError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let file = File::create(&path).map_err(|source| RawFrameError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            fps,
            pixel_format,
            dimensions: None,
            frame_count: 0,
        })
    }

    /// Pixel format written by this stream.
    #[must_use]
    pub const fn pixel_format(&self) -> RawPixelFormat {
        self.pixel_format
    }

    /// Flattens 8-bit Display P3 `rgba_data` over white under `alpha` and
    /// appends the frame at `timestamp` to the stream.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError`] if the writer format is not
    /// [`RawPixelFormat::Rgba8DisplayP3`], the frame is empty or malformed,
    /// dimensions change mid-stream, or writing fails.
    pub fn write_frame(
        &mut self,
        width: u32,
        height: u32,
        mut rgba_data: Vec<u8>,
        alpha: Alpha,
        timestamp: Duration,
    ) -> Result<(), RawFrameError> {
        if self.pixel_format != RawPixelFormat::Rgba8DisplayP3 {
            return Err(RawFrameError::FormatMismatch {
                expected: self.pixel_format,
                actual: RawPixelFormat::Rgba8DisplayP3,
            });
        }
        self.validate_frame_bytes(width, height, rgba_data.len())?;
        flatten_alpha_over_white(&mut rgba_data, alpha);
        self.append_encoded_frame(width, height, &rgba_data, timestamp)
    }

    /// Flattens `ExtendedLinearDisplayP3` RGBA `f32` pixels over SDR white
    /// under `alpha`, encodes them as IEEE-754 `f16` little-endian RGBA, and
    /// appends the frame at `timestamp` to an HDR stream.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError`] if the writer format is not
    /// [`RawPixelFormat::Rgba16FloatLinearDisplayP3`], the buffer is empty or
    /// malformed, dimensions change mid-stream, or writing fails.
    pub fn write_hdr_frame_f32(
        &mut self,
        width: u32,
        height: u32,
        rgba_linear_p3: &[f32],
        alpha: Alpha,
        timestamp: Duration,
    ) -> Result<(), RawFrameError> {
        if self.pixel_format != RawPixelFormat::Rgba16FloatLinearDisplayP3 {
            return Err(RawFrameError::FormatMismatch {
                expected: self.pixel_format,
                actual: RawPixelFormat::Rgba16FloatLinearDisplayP3,
            });
        }
        let byte_len = rgba_linear_p3.len().saturating_mul(2);
        self.validate_frame_bytes(width, height, byte_len)?;
        let mut linear = rgba_linear_p3.to_vec();
        flatten_linear_over_white(&mut linear, alpha);
        let mut f16_bytes = Vec::with_capacity(byte_len);
        for value in linear {
            f16_bytes.extend_from_slice(&f16::from_f32(value).to_le_bytes());
        }
        self.append_encoded_frame(width, height, &f16_bytes, timestamp)
    }

    /// Flattens `ExtendedLinearDisplayP3` IEEE-754 `f16` little-endian RGBA
    /// bytes (`width * height * 8` bytes) over SDR white under `alpha` and
    /// appends the frame at `timestamp` to an HDR stream.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError`] if the writer format is not
    /// [`RawPixelFormat::Rgba16FloatLinearDisplayP3`], the buffer is empty or
    /// malformed, dimensions change mid-stream, or writing fails.
    pub fn write_hdr_frame_f16(
        &mut self,
        width: u32,
        height: u32,
        rgba16f_bytes: &[u8],
        alpha: Alpha,
        timestamp: Duration,
    ) -> Result<(), RawFrameError> {
        if self.pixel_format != RawPixelFormat::Rgba16FloatLinearDisplayP3 {
            return Err(RawFrameError::FormatMismatch {
                expected: self.pixel_format,
                actual: RawPixelFormat::Rgba16FloatLinearDisplayP3,
            });
        }
        self.validate_frame_bytes(width, height, rgba16f_bytes.len())?;
        let mut linear: Vec<f32> = rgba16f_bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&bits| f16::from_le_bytes(bits).to_f32())
            .collect();
        flatten_linear_over_white(&mut linear, alpha);
        let mut encoded = Vec::with_capacity(rgba16f_bytes.len());
        for value in linear {
            encoded.extend_from_slice(&f16::from_f32(value).to_le_bytes());
        }
        self.append_encoded_frame(width, height, &encoded, timestamp)
    }

    fn validate_frame_bytes(
        &self,
        width: u32,
        height: u32,
        byte_len: usize,
    ) -> Result<(), RawFrameError> {
        if width == 0 || height == 0 || byte_len == 0 {
            return Err(RawFrameError::EmptyFrame {
                width,
                height,
                len: byte_len,
            });
        }
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(self.pixel_format.bytes_per_pixel()));
        if expected_len != Some(byte_len) {
            return Err(RawFrameError::InvalidBuffer {
                width,
                height,
                len: byte_len,
            });
        }
        Ok(())
    }

    fn append_encoded_frame(
        &mut self,
        width: u32,
        height: u32,
        payload: &[u8],
        timestamp: Duration,
    ) -> Result<(), RawFrameError> {
        if let Some((expected_width, expected_height)) = self.dimensions {
            if expected_width != width || expected_height != height {
                return Err(RawFrameError::DimensionMismatch {
                    expected_width,
                    expected_height,
                    actual_width: width,
                    actual_height: height,
                });
            }
        } else {
            match self.pixel_format {
                RawPixelFormat::Rgba8DisplayP3 => {
                    self.write_all(RAW_FRAME_STREAM_MAGIC_V1)?;
                    self.write_all(&self.fps.to_le_bytes())?;
                    self.write_all(&width.to_le_bytes())?;
                    self.write_all(&height.to_le_bytes())?;
                }
                RawPixelFormat::Rgba16FloatLinearDisplayP3 => {
                    self.write_all(RAW_FRAME_STREAM_MAGIC_V2)?;
                    self.write_all(&self.fps.to_le_bytes())?;
                    self.write_all(&width.to_le_bytes())?;
                    self.write_all(&height.to_le_bytes())?;
                    self.write_all(&[self.pixel_format.wire_code()])?;
                }
            }
            self.dimensions = Some((width, height));
        }

        let timestamp_ns = u64::try_from(timestamp.as_nanos()).unwrap_or(u64::MAX);
        self.write_all(&timestamp_ns.to_le_bytes())?;
        self.write_all(payload)?;
        self.frame_count += 1;
        Ok(())
    }

    /// Flushes the stream and returns the number of written frames.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError::NoFrames`] when no frames were written, or an
    /// I/O error on flush failure.
    pub fn finish(mut self) -> Result<u64, RawFrameError> {
        if self.frame_count == 0 {
            return Err(RawFrameError::NoFrames);
        }
        self.writer.flush().map_err(|source| RawFrameError::Io {
            path: self.path,
            source,
        })?;
        Ok(self.frame_count)
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), RawFrameError> {
        self.writer
            .write_all(bytes)
            .map_err(|source| RawFrameError::Io {
                path: self.path.clone(),
                source,
            })
    }
}

/// Reads flattened opaque frames produced by [`RawFrameStreamWriter`]
/// (`WUIRF001` 8-bit Display P3 or `WUIRF002` typed SDR/HDR).
#[derive(Debug)]
pub struct RawFrameStreamReader {
    path: PathBuf,
    reader: BufReader<File>,
    fps: u32,
    width: u32,
    height: u32,
    pixel_format: RawPixelFormat,
    frame_bytes: usize,
}

impl RawFrameStreamReader {
    /// Opens a `WUIRF001` or `WUIRF002` raw frame stream and validates its
    /// header.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError`] if the file cannot be opened or has an invalid
    /// header, unknown pixel format, zero fps, or zero dimensions.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RawFrameError> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).map_err(|source| RawFrameError::Io {
            path: path.clone(),
            source,
        })?;
        let mut reader = BufReader::new(file);
        let mut header = [0_u8; 20];
        reader.read_exact(&mut header).map_err(|source| {
            if source.kind() == ErrorKind::UnexpectedEof {
                RawFrameError::InvalidHeader { path: path.clone() }
            } else {
                RawFrameError::Io {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let pixel_format = match &header[0..8] {
            magic if magic == RAW_FRAME_STREAM_MAGIC_V1 => RawPixelFormat::Rgba8DisplayP3,
            magic if magic == RAW_FRAME_STREAM_MAGIC_V2 => {
                let mut format_byte = [0_u8; 1];
                reader.read_exact(&mut format_byte).map_err(|source| {
                    if source.kind() == ErrorKind::UnexpectedEof {
                        RawFrameError::InvalidHeader { path: path.clone() }
                    } else {
                        RawFrameError::Io {
                            path: path.clone(),
                            source,
                        }
                    }
                })?;
                let Some(format) = RawPixelFormat::from_wire_code(format_byte[0]) else {
                    return Err(RawFrameError::InvalidHeader { path });
                };
                format
            }
            _ => return Err(RawFrameError::InvalidHeader { path }),
        };
        let fps = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let width = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        let height = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
        if fps == 0 {
            return Err(RawFrameError::InvalidFps);
        }
        let Some(frame_bytes) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(pixel_format.bytes_per_pixel()))
            .filter(|&len| len > 0)
        else {
            return Err(RawFrameError::InvalidHeader { path });
        };
        Ok(Self {
            path,
            reader,
            fps,
            width,
            height,
            pixel_format,
            frame_bytes,
        })
    }

    /// Stream frame rate in frames per second.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }

    /// Frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Pixel format carried by every frame in this stream.
    #[must_use]
    pub const fn pixel_format(&self) -> RawPixelFormat {
        self.pixel_format
    }

    /// Bytes per pixel for this stream's pixel format.
    #[must_use]
    pub const fn bytes_per_pixel(&self) -> usize {
        self.pixel_format.bytes_per_pixel()
    }

    /// Reads the next frame into `rgba_out` (reusing its allocation) and
    /// returns its timestamp, or `Ok(None)` at clean EOF.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError::Io`] on truncated frames or read failures.
    pub fn next_frame(
        &mut self,
        rgba_out: &mut Vec<u8>,
    ) -> Result<Option<Duration>, RawFrameError> {
        let mut ts_bytes = [0_u8; 8];
        let mut read = 0;
        while read < ts_bytes.len() {
            match self.reader.read(&mut ts_bytes[read..]) {
                Ok(0) if read == 0 => return Ok(None),
                Ok(0) => {
                    return Err(RawFrameError::Io {
                        path: self.path.clone(),
                        source: io::Error::new(
                            ErrorKind::UnexpectedEof,
                            "truncated raw frame timestamp",
                        ),
                    });
                }
                Ok(n) => read += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(source) => {
                    return Err(RawFrameError::Io {
                        path: self.path.clone(),
                        source,
                    });
                }
            }
        }
        let timestamp_ns = u64::from_le_bytes(ts_bytes);
        rgba_out.resize(self.frame_bytes, 0);
        self.reader
            .read_exact(rgba_out)
            .map_err(|source| RawFrameError::Io {
                path: self.path.clone(),
                source,
            })?;
        Ok(Some(Duration::from_nanos(timestamp_ns)))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{BufReader, Cursor};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use std::{env, process};

    use half::f16;
    use png::{BitDepth, Decoder as PngDecoder};

    use super::{
        Alpha, PreviewRunConfig, PreviewRunMode, RawFrameError, RawFrameStreamReader,
        RawFrameStreamWriter, RawPixelFormat, ScenarioEvent, ScenarioEventKind, ScenarioSample,
        VideoCaptureConfig, encode_hdr_png, encode_png, flatten_alpha_over_white, frame_offset,
        linear_edr_to_pq, plan_scenario_timeline, write_hdr_png_f16,
    };

    fn scratch_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or_default(|duration| duration.as_nanos());
        env::temp_dir().join(format!(
            "waterui-preview-protocol-{name}-{}-{nanos}.wuirf",
            process::id()
        ))
    }

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

    #[test]
    fn run_config_defaults_and_round_trips_color_scheme_and_scale() {
        use core::time::Duration;
        use std::path::PathBuf;

        use super::{DEFAULT_PREVIEW_SCALE, PreviewColorScheme, PreviewRunConfig, PreviewRunMode};

        let legacy_json = r#"{"width":375.0,"height":667.0,"mode":{"Image":{"output":"out.png"}},"content_deadline_ms":10000}"#;
        let legacy: PreviewRunConfig =
            serde_json::from_str(legacy_json).expect("legacy run config parses");
        assert_eq!(legacy.color_scheme(), None);
        assert!((legacy.scale() - DEFAULT_PREVIEW_SCALE).abs() < f32::EPSILON);

        let config = PreviewRunConfig::new(
            220.0,
            80.0,
            PreviewRunMode::Image {
                output: PathBuf::from("dark.png"),
            },
            Duration::from_secs(5),
        )
        .with_color_scheme(PreviewColorScheme::Dark)
        .with_scale(3.0);
        let serialized = serde_json::to_string(&config).expect("run config serializes");
        let parsed: PreviewRunConfig =
            serde_json::from_str(&serialized).expect("run config deserializes");
        assert_eq!(parsed.color_scheme(), Some(PreviewColorScheme::Dark));
        assert!((parsed.scale() - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn plan_scenario_timeline_coalesces_png_and_video_samples() {
        let video = VideoCaptureConfig::new("frames.wuirf".into(), 20).with_duration_ms(Some(100));
        let samples =
            plan_scenario_timeline(&[0, 50, 75], Some(&video), &[]).expect("valid merged timeline");
        // 20 fps over 100ms produces 0, 50, 100ms; PNG adds 75ms and coalesces at 0 & 50ms.
        let summary: Vec<(Duration, Option<u64>, bool)> = samples
            .iter()
            .map(|sample| {
                (
                    sample.offset(),
                    sample.png_capture_ms(),
                    sample.record_video(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (Duration::from_millis(0), Some(0), true),
                (Duration::from_millis(50), Some(50), true),
                (Duration::from_millis(75), Some(75), false),
                (Duration::from_millis(100), None, true),
            ]
        );
    }

    #[test]
    fn plan_scenario_timeline_infers_duration_from_events() {
        let video = VideoCaptureConfig::new("frames.wuirf".into(), 10);
        let events = [ScenarioEvent::new(
            200,
            ScenarioEventKind::PointerUp,
            10.0,
            20.0,
        )];
        let samples = plan_scenario_timeline(&[], Some(&video), &events)
            .expect("video timeline inferred from events");
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].offset(), frame_offset(0, 10));
        assert_eq!(samples[1].offset(), frame_offset(1, 10));
        assert_eq!(samples[2].offset(), frame_offset(2, 10));
        assert!(samples.iter().all(ScenarioSample::record_video));
    }

    #[test]
    fn plan_scenario_timeline_rejects_zero_fps_and_empty_timeline() {
        let bad_video = VideoCaptureConfig::new("frames.wuirf".into(), 0);
        assert!(matches!(
            plan_scenario_timeline(&[], Some(&bad_video), &[]),
            Err(RawFrameError::InvalidFps)
        ));
        assert!(matches!(
            plan_scenario_timeline(&[], None, &[]),
            Err(RawFrameError::EmptyTimeline)
        ));
    }

    #[test]
    fn raw_frame_stream_roundtrips_and_validates_frames() {
        let path = scratch_path("roundtrip");
        let mut writer = RawFrameStreamWriter::create(&path, 60).expect("create stream");
        writer
            .write_frame(
                2,
                1,
                vec![200, 0, 0, 128, 10, 20, 30, 255],
                Alpha::Straight,
                Duration::from_millis(0),
            )
            .expect("frame 0");
        writer
            .write_frame(
                2,
                1,
                vec![0, 100, 50, 255, 255, 255, 255, 255],
                Alpha::Premultiplied,
                Duration::from_millis(16),
            )
            .expect("frame 1");
        assert!(matches!(
            writer.write_frame(1, 1, vec![0; 4], Alpha::Straight, Duration::from_millis(33)),
            Err(RawFrameError::DimensionMismatch { .. })
        ));
        assert_eq!(writer.finish().expect("finish"), 2);

        let mut reader = RawFrameStreamReader::open(&path).expect("open stream");
        assert_eq!((reader.fps(), reader.width(), reader.height()), (60, 2, 1));
        assert_eq!(reader.pixel_format(), RawPixelFormat::Rgba8DisplayP3);
        let mut buf = Vec::new();
        assert_eq!(
            reader.next_frame(&mut buf).expect("read 0"),
            Some(Duration::from_millis(0))
        );
        assert_eq!(buf, vec![227, 127, 127, 255, 10, 20, 30, 255]);
        assert_eq!(
            reader.next_frame(&mut buf).expect("read 1"),
            Some(Duration::from_millis(16))
        );
        assert_eq!(buf, vec![0, 100, 50, 255, 255, 255, 255, 255]);
        assert_eq!(reader.next_frame(&mut buf).expect("eof"), None);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn encode_png_tags_display_p3_sdr_cicp() {
        let png_bytes =
            encode_png(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).expect("encode sdr png");
        let decoder = PngDecoder::new(Cursor::new(png_bytes));
        let reader = decoder.read_info().expect("decode sdr png");
        let info = reader.info();
        assert_eq!(info.bit_depth, BitDepth::Eight);
        let cicp = info
            .coding_independent_code_points
            .expect("SDR preview PNG must carry Display P3 cICP chunk");
        assert_eq!(cicp.color_primaries, 12);
        assert_eq!(cicp.transfer_function, 13);
        assert_eq!(cicp.matrix_coefficients, 0);
        assert!(cicp.is_video_full_range_image);
    }

    #[test]
    fn encode_hdr_png_writes_16bit_display_p3_pq_preserving_edr_headroom() {
        // Pixel 0: SDR white (1.0 = 203 nits), Pixel 1: 4x EDR highlight (812 nits).
        let png_bytes =
            encode_hdr_png(2, 1, &[1.0, 1.0, 1.0, 1.0, 4.0, 2.0, 0.5, 1.0]).expect("encode hdr");
        let decoder = PngDecoder::new(Cursor::new(png_bytes));
        let mut reader = decoder.read_info().expect("decode hdr png");
        let info = reader.info();
        assert_eq!(info.bit_depth, BitDepth::Sixteen);
        let cicp = info
            .coding_independent_code_points
            .expect("HDR preview PNG must carry Display P3 PQ cICP chunk");
        assert_eq!(cicp.color_primaries, 12);
        assert_eq!(cicp.transfer_function, 16);
        assert_eq!(cicp.matrix_coefficients, 0);
        assert!(cicp.is_video_full_range_image);

        let mut pixels = vec![0_u8; reader.output_buffer_size().expect("buffer size")];
        let frame = reader.next_frame(&mut pixels).expect("read hdr pixels");
        let bytes = &pixels[..frame.buffer_size()];
        let sdr_r = u16::from_be_bytes([bytes[0], bytes[1]]);
        let hdr_r = u16::from_be_bytes([bytes[8], bytes[9]]);
        let hdr_g = u16::from_be_bytes([bytes[10], bytes[11]]);
        let hdr_b = u16::from_be_bytes([bytes[12], bytes[13]]);
        assert!(
            hdr_r > sdr_r,
            "4.0 EDR highlight ({hdr_r}) must encode strictly brighter than 1.0 SDR white ({sdr_r})"
        );
        assert!(hdr_g > sdr_r);
        assert!(hdr_b < sdr_r);
        assert!((linear_edr_to_pq(1.0) - 0.5807).abs() < 0.002);
    }

    #[test]
    fn write_hdr_png_f16_flattens_and_tags_cicp() {
        let path = scratch_path("hdr-png-f16").with_extension("png");
        let mut f16_bytes = Vec::new();
        for v in [3.0_f32, 1.5, 0.5, 1.0] {
            f16_bytes.extend_from_slice(&f16::from_f32(v).to_le_bytes());
        }
        write_hdr_png_f16(&path, 1, 1, &f16_bytes, Alpha::Premultiplied)
            .expect("write hdr png from f16");
        let file = File::open(&path).expect("open hdr png");
        let reader = PngDecoder::new(BufReader::new(file))
            .read_info()
            .expect("decode hdr png");
        let cicp = reader
            .info()
            .coding_independent_code_points
            .expect("cICP present");
        assert_eq!((cicp.color_primaries, cicp.transfer_function), (12, 16));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn hdr_raw_frame_stream_v2_roundtrips_linear_p3_f16_and_rejects_mismatch() {
        let path = scratch_path("hdr-v2");
        let mut writer = RawFrameStreamWriter::create_hdr(&path, 60).expect("create hdr stream");
        assert_eq!(
            writer.pixel_format(),
            RawPixelFormat::Rgba16FloatLinearDisplayP3
        );
        assert!(matches!(
            writer.write_frame(1, 1, vec![0; 4], Alpha::Straight, Duration::ZERO),
            Err(RawFrameError::FormatMismatch { .. })
        ));
        writer
            .write_hdr_frame_f32(
                2,
                1,
                &[3.5, 1.0, 0.25, 1.0, 2.0, 0.0, 0.0, 0.5],
                Alpha::Straight,
                Duration::from_millis(0),
            )
            .expect("write hdr f32 frame");
        let mut f16_frame = Vec::new();
        for v in [4.0_f32, 2.0, 1.0, 1.0, 0.5, 0.5, 0.5, 1.0] {
            f16_frame.extend_from_slice(&f16::from_f32(v).to_le_bytes());
        }
        writer
            .write_hdr_frame_f16(
                2,
                1,
                &f16_frame,
                Alpha::Premultiplied,
                Duration::from_millis(16),
            )
            .expect("write hdr f16 frame");
        assert_eq!(writer.finish().expect("finish hdr stream"), 2);

        let mut reader = RawFrameStreamReader::open(&path).expect("open hdr v2 stream");
        assert_eq!(
            reader.pixel_format(),
            RawPixelFormat::Rgba16FloatLinearDisplayP3
        );
        assert_eq!(reader.bytes_per_pixel(), 8);
        assert_eq!((reader.fps(), reader.width(), reader.height()), (60, 2, 1));

        let mut buf = Vec::new();
        assert_eq!(
            reader.next_frame(&mut buf).expect("read hdr 0"),
            Some(Duration::from_millis(0))
        );
        assert_eq!(buf.len(), 2 * 8);
        let r0 = f16::from_le_bytes([buf[0], buf[1]]).to_f32();
        let r1 = f16::from_le_bytes([buf[8], buf[9]]).to_f32();
        assert!((r0 - 3.5).abs() < 0.01, "HDR 3.5 preserved: {r0}");
        // Straight (2.0 * 0.5 + 1.0 * 0.5) = 1.5 over SDR white.
        assert!((r1 - 1.5).abs() < 0.01, "flattened HDR pixel: {r1}");

        assert_eq!(
            reader.next_frame(&mut buf).expect("read hdr 1"),
            Some(Duration::from_millis(16))
        );
        let r0_f2 = f16::from_le_bytes([buf[0], buf[1]]).to_f32();
        assert!((r0_f2 - 4.0).abs() < 0.01);
        assert_eq!(reader.next_frame(&mut buf).expect("eof"), None);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn preview_run_config_json_defaults_and_roundtrips_hdr() {
        let legacy_json = r#"{"width":375.0,"height":667.0,"mode":{"Image":{"output":"out.png"}},"content_deadline_ms":10000}"#;
        let legacy: PreviewRunConfig =
            serde_json::from_str(legacy_json).expect("legacy json without hdr parses");
        assert!(!legacy.hdr());

        let hdr_config = PreviewRunConfig::new(
            375.0,
            667.0,
            PreviewRunMode::Image {
                output: PathBuf::from("hdr.png"),
            },
            Duration::from_secs(10),
        )
        .with_hdr(true);
        let json = serde_json::to_string(&hdr_config).expect("serialize hdr config");
        let parsed: PreviewRunConfig = serde_json::from_str(&json).expect("deserialize hdr config");
        assert!(parsed.hdr());
    }

    #[test]
    fn preview_run_config_json_roundtrips_video_modes() {
        let scenario = PreviewRunConfig::new(
            375.0,
            667.0,
            PreviewRunMode::Scenario {
                output_dir: None,
                captures_ms: Vec::new(),
                events: Vec::new(),
                video: Some(
                    VideoCaptureConfig::new("frames.wuirf".into(), 60).with_duration_ms(Some(500)),
                ),
            },
            Duration::from_secs(10),
        );
        let json = serde_json::to_string(&scenario).expect("serialize scenario");
        let parsed: PreviewRunConfig = serde_json::from_str(&json).expect("deserialize scenario");
        let PreviewRunMode::Scenario { video, .. } = parsed.mode() else {
            panic!("expected scenario mode");
        };
        let video = video.as_ref().expect("video config");
        assert_eq!(video.fps(), 60);
        assert_eq!(video.duration_ms(), Some(500));

        let semantic = PreviewRunConfig::new(
            375.0,
            667.0,
            PreviewRunMode::Semantic {
                video: Some(VideoCaptureConfig::new("test.wuirf".into(), 30)),
            },
            Duration::from_secs(10),
        );
        let json = serde_json::to_string(&semantic).expect("serialize semantic");
        let parsed: PreviewRunConfig = serde_json::from_str(&json).expect("deserialize semantic");
        let PreviewRunMode::Semantic { video } = parsed.mode() else {
            panic!("expected semantic mode");
        };
        assert_eq!(video.as_ref().map(VideoCaptureConfig::fps), Some(30));
    }
}

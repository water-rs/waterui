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

use serde::{Deserialize, Serialize};

/// Environment variable carrying the path of the JSON-encoded
/// [`PreviewRunConfig`] for the generated preview binary.
pub const PREVIEW_RUN_CONFIG_ENV: &str = "WATERUI_PREVIEW_RUN_CONFIG";

/// Default frame rate for `water preview --video` and `water preview test --video`.
pub const DEFAULT_VIDEO_FPS: u32 = 60;

/// Magic header for raw RGBA8 preview frame streams (`WUIRF001`).
const RAW_FRAME_STREAM_MAGIC: &[u8; 8] = b"WUIRF001";

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
#[non_exhaustive]
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

/// Why reading or writing a raw RGBA8 preview frame stream failed.
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
    /// Frame buffer length does not equal `width * height * 4`.
    #[error("invalid RGBA8 frame buffer: {len} bytes for {width}x{height}")]
    InvalidBuffer {
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
        /// Buffer length in bytes.
        len: usize,
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
    /// Input file is not a valid `WUIRF001` raw frame stream.
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

/// Streams flattened opaque RGBA8 frames to disk in a compact binary framing
/// (`WUIRF001`) so the host CLI can hardware-encode them without per-frame PNG
/// compression overhead.
#[derive(Debug)]
pub struct RawFrameStreamWriter {
    path: PathBuf,
    writer: BufWriter<File>,
    fps: u32,
    dimensions: Option<(u32, u32)>,
    frame_count: u64,
}

impl RawFrameStreamWriter {
    /// Creates a new raw frame stream at `path` for captures at `fps`.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError::InvalidFps`] when `fps == 0`, or an I/O error
    /// if the parent directory or file cannot be created.
    pub fn create(path: impl AsRef<Path>, fps: u32) -> Result<Self, RawFrameError> {
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
            dimensions: None,
            frame_count: 0,
        })
    }

    /// Flattens `rgba_data` over white under `alpha` and appends the frame at
    /// `timestamp` to the stream.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError`] if the frame is empty, malformed, changes
    /// dimensions mid-stream, or cannot be written.
    pub fn write_frame(
        &mut self,
        width: u32,
        height: u32,
        mut rgba_data: Vec<u8>,
        alpha: Alpha,
        timestamp: Duration,
    ) -> Result<(), RawFrameError> {
        if width == 0 || height == 0 || rgba_data.is_empty() {
            return Err(RawFrameError::EmptyFrame {
                width,
                height,
                len: rgba_data.len(),
            });
        }
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if expected_len != Some(rgba_data.len()) {
            return Err(RawFrameError::InvalidBuffer {
                width,
                height,
                len: rgba_data.len(),
            });
        }

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
            self.write_all(RAW_FRAME_STREAM_MAGIC)?;
            self.write_all(&self.fps.to_le_bytes())?;
            self.write_all(&width.to_le_bytes())?;
            self.write_all(&height.to_le_bytes())?;
            self.dimensions = Some((width, height));
        }

        flatten_alpha_over_white(&mut rgba_data, alpha);
        let timestamp_ns = u64::try_from(timestamp.as_nanos()).unwrap_or(u64::MAX);
        self.write_all(&timestamp_ns.to_le_bytes())?;
        self.write_all(&rgba_data)?;
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

/// Reads flattened opaque RGBA8 frames produced by [`RawFrameStreamWriter`].
#[derive(Debug)]
pub struct RawFrameStreamReader {
    path: PathBuf,
    reader: BufReader<File>,
    fps: u32,
    width: u32,
    height: u32,
    frame_bytes: usize,
}

impl RawFrameStreamReader {
    /// Opens a `WUIRF001` raw frame stream and validates its header.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameError`] if the file cannot be opened or has an invalid
    /// header, zero fps, or zero dimensions.
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
        if &header[0..8] != RAW_FRAME_STREAM_MAGIC {
            return Err(RawFrameError::InvalidHeader { path });
        }
        let fps = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let width = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        let height = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
        if fps == 0 {
            return Err(RawFrameError::InvalidFps);
        }
        let Some(frame_bytes) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
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
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use std::{env, fs, process};

    use super::{
        Alpha, PreviewRunConfig, PreviewRunMode, RawFrameError, RawFrameStreamReader,
        RawFrameStreamWriter, ScenarioEvent, ScenarioEventKind, VideoCaptureConfig,
        flatten_alpha_over_white, frame_offset, plan_scenario_timeline,
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
        assert!(samples.iter().all(super::ScenarioSample::record_video));
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

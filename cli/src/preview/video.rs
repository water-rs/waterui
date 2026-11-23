//! Hardware-accelerated MP4 video encoder for `water preview --video` and
//! `water preview test --video`.
//!
//! Frame sampling is decoupled from encoding: preview runtimes stream flattened
//! opaque RGBA8 frames into a `WUIRF001` file ([`RawFrameStreamReader`]), and
//! this module converts each frame to NV12 (`yuv`), hardware-encodes it with
//! `waterkit-codec` (`VideoToolbox` on macOS, Media Foundation on Windows), and
//! muxes the compressed stream into an MP4 container via
//! `waterkit-video-container`. No external tools or software encoder fallbacks
//! are used; hosts without hardware encoding fail fast with a clear diagnostic.

use std::fs;
use std::path::Path;

use eyre::{Context as _, Result, bail, eyre};
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "android"))]
use waterkit_codec::annex_b_to_length_prefixed;
use waterkit_codec::{CodecType, Encoder, EncoderProfile};
use waterkit_video_container::{MuxerCodecType, VideoWriter};
use waterkit_video_core::{
    ColorPrimaries, ColorRange, MatrixCoefficients, TransferFunction, VideoColorInfo,
};
use waterui_preview_protocol::run::RawFrameStreamReader;
use yuv::{
    YuvBiPlanarImageMut, YuvChromaSubsampling, YuvConversionMode, YuvRange, YuvStandardMatrix,
    rgba_to_yuv_nv12,
};

/// Default recording frame rate (60 fps).
pub use waterui_preview_protocol::run::DEFAULT_VIDEO_FPS;

/// Hardware codec selected for encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HardwareCodec {
    H264,
    H265,
}

impl HardwareCodec {
    const fn is_hevc(self) -> bool {
        matches!(self, Self::H265)
    }

    const fn muxer_codec(self) -> MuxerCodecType {
        match self {
            Self::H264 => MuxerCodecType::H264,
            Self::H265 => MuxerCodecType::H265,
        }
    }
}

/// Standard BT.709 SDR limited-range color description for H.264/H.265 MP4 output.
const fn bt709_sdr_limited() -> VideoColorInfo {
    VideoColorInfo {
        matrix: MatrixCoefficients::Bt709,
        primaries: ColorPrimaries::Bt709,
        transfer: TransferFunction::Sdr,
        range: ColorRange::Limited,
        content_light_level: None,
        dolby_vision: false,
    }
}

/// Streaming hardware RGBA8-to-MP4 video encoder.
///
/// Reuses its NV12 conversion and optional odd-dimension padding buffers across
/// frames so long recordings allocate once.
#[derive(Debug)]
pub struct HardwareVideoEncoder {
    encoder: Encoder,
    codec: HardwareCodec,
    writer: VideoWriter,
    input_width: u32,
    input_height: u32,
    encoded_width: u32,
    encoded_height: u32,
    padded_rgba: Vec<u8>,
    nv12: Vec<u8>,
    has_codec_config: bool,
}

impl HardwareVideoEncoder {
    /// Opens a hardware H.264 (or H.265) encoder and MP4 container writer at
    /// `output_path` for `width`×`height` RGBA8 frames at `fps`.
    ///
    /// Odd dimensions are rounded up by 1 pixel (replicating the edge pixel)
    /// to satisfy 4:2:0 NV12 alignment.
    ///
    /// # Errors
    ///
    /// Fails fast if `width`, `height`, or `fps` is zero, if no hardware video
    /// encoder is available on this host, or if `output_path` cannot be created.
    pub fn new(output_path: &Path, width: u32, height: u32, fps: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("Video frame dimensions must be positive, got {width}x{height}.");
        }
        if fps == 0 {
            bail!("Video frame rate (`--fps`) must be greater than zero.");
        }

        let encoded_width = even_dimension(width)?;
        let encoded_height = even_dimension(height)?;
        let color = bt709_sdr_limited();

        let (encoder, codec) = open_hardware_encoder(encoded_width, encoded_height, color)?;

        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).wrap_err_with(|| {
                format!(
                    "Failed to create video output directory {}",
                    parent.display()
                )
            })?;
        }
        let writer = VideoWriter::new(
            output_path,
            encoded_width,
            encoded_height,
            fps,
            codec.muxer_codec(),
            color,
        )
        .map_err(|error| {
            eyre!(
                "Failed to create MP4 writer {}: {error}",
                output_path.display()
            )
        })?;

        let y_len = (encoded_width as usize)
            .checked_mul(encoded_height as usize)
            .ok_or_else(|| {
                eyre!("Video frame dimensions {encoded_width}x{encoded_height} overflow usize")
            })?;
        let nv12_len = y_len
            .checked_add(y_len / 2)
            .ok_or_else(|| eyre!("NV12 frame size overflows usize"))?;

        Ok(Self {
            encoder,
            codec,
            writer,
            input_width: width,
            input_height: height,
            encoded_width,
            encoded_height,
            padded_rgba: Vec::new(),
            nv12: Vec::with_capacity(nv12_len),
            has_codec_config: false,
        })
    }

    /// Converts one opaque RGBA8 frame to NV12, hardware-encodes it, and
    /// appends any emitted access units to the MP4 container.
    ///
    /// # Errors
    ///
    /// Returns an error if `rgba` does not match `width * height * 4`, if YUV
    /// conversion fails, or if hardware encoding or container muxing fails.
    pub fn push_rgba(&mut self, rgba: &[u8]) -> Result<()> {
        let expected_len = (self.input_width as usize)
            .checked_mul(self.input_height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if expected_len != Some(rgba.len()) {
            bail!(
                "Invalid RGBA8 frame buffer: got {} bytes for {}x{}",
                rgba.len(),
                self.input_width,
                self.input_height
            );
        }

        let source_rgba: &[u8] =
            if self.input_width == self.encoded_width && self.input_height == self.encoded_height {
                rgba
            } else {
                pad_rgba_to_even(
                    rgba,
                    self.input_width,
                    self.input_height,
                    self.encoded_width,
                    self.encoded_height,
                    &mut self.padded_rgba,
                );
                &self.padded_rgba
            };

        let mut nv12_image = YuvBiPlanarImageMut::alloc(
            self.encoded_width,
            self.encoded_height,
            YuvChromaSubsampling::Yuv420,
        );
        rgba_to_yuv_nv12(
            &mut nv12_image,
            source_rgba,
            self.encoded_width * 4,
            YuvRange::Limited,
            YuvStandardMatrix::Bt709,
            YuvConversionMode::Balanced,
        )
        .map_err(|error| eyre!("RGBA8-to-NV12 conversion failed: {error}"))?;

        self.nv12.clear();
        self.nv12.extend_from_slice(nv12_image.y_plane.borrow());
        self.nv12.extend_from_slice(nv12_image.uv_plane.borrow());

        let mut packet = Vec::new();
        for item in self.encoder.encode_nv12(&self.nv12) {
            let bytes = item.map_err(|error| eyre!("Hardware video encoding failed: {error}"))?;
            packet.extend_from_slice(&bytes);
        }
        if !packet.is_empty() {
            self.emit_packet(packet)?;
        }
        Ok(())
    }

    /// Drains any buffered hardware-encoder frames and finalizes the MP4 file,
    /// returning the number of encoded video frames written.
    ///
    /// # Errors
    ///
    /// Returns an error if flushing fails, if zero frames were encoded, if the
    /// encoder produced no codec configuration (`avcC`/`hvcC`), or if writing
    /// the MP4 `moov` box fails.
    pub fn finish(mut self) -> Result<u64> {
        let trailing = self
            .encoder
            .flush()
            .map_err(|error| eyre!("Hardware video encoder flush failed: {error}"))?;
        for packet in trailing {
            if !packet.is_empty() {
                self.emit_packet(packet)?;
            }
        }
        if !self.has_codec_config
            && let Some(config) = self.encoder.codec_config()
        {
            self.writer.set_codec_config(config);
            self.has_codec_config = true;
        }
        let frames = self.writer.frame_count();
        if frames == 0 {
            bail!("Hardware video encoder produced zero frames.");
        }
        if !self.has_codec_config {
            bail!("Hardware video encoder produced no codec configuration (avcC/hvcC).");
        }
        self.writer
            .finish()
            .map_err(|error| eyre!("Failed to finalize MP4 container: {error}"))?;
        Ok(frames)
    }

    fn emit_packet(&mut self, packet: Vec<u8>) -> Result<()> {
        if !self.has_codec_config
            && let Some(config) = self.encoder.codec_config()
        {
            self.writer.set_codec_config(config);
            self.has_codec_config = true;
        }
        let (sample, is_keyframe) = normalize_sample(packet, self.codec);
        self.writer
            .write_sample(&sample, is_keyframe)
            .map_err(|error| eyre!("Failed to write MP4 sample: {error}"))
    }
}

/// Encodes a `WUIRF001` raw RGBA8 frame stream into a hardware-encoded MP4 file,
/// returning the number of frames written.
///
/// # Errors
///
/// Returns an error if the raw frame stream cannot be opened or read, if no
/// hardware video encoder is available on this host, or if encoding/muxing fails.
pub fn encode_raw_frame_stream_to_mp4(raw_stream_path: &Path, output_mp4: &Path) -> Result<u64> {
    let mut reader = RawFrameStreamReader::open(raw_stream_path).wrap_err_with(|| {
        format!(
            "Failed to open preview raw frame stream {}",
            raw_stream_path.display()
        )
    })?;
    let mut encoder =
        HardwareVideoEncoder::new(output_mp4, reader.width(), reader.height(), reader.fps())?;
    let mut frame_buf = Vec::new();
    while reader.next_frame(&mut frame_buf)?.is_some() {
        encoder.push_rgba(&frame_buf)?;
    }
    encoder.finish()
}

fn open_hardware_encoder(
    width: u32,
    height: u32,
    color: VideoColorInfo,
) -> Result<(Encoder, HardwareCodec)> {
    let mut attempts = Vec::new();
    for (codec_type, choice, label) in [
        (CodecType::H264, HardwareCodec::H264, "H.264"),
        (CodecType::H265, HardwareCodec::H265, "H.265"),
    ] {
        match Encoder::new(codec_type, width, height, EncoderProfile::Realtime, color) {
            Ok(encoder) => return Ok((encoder, choice)),
            Err(error) => attempts.push(format!("{label}: {error}")),
        }
    }
    bail!(
        "Hardware video encoding is unavailable on this machine ({}). \
         `water preview --video` requires a platform hardware encoder via `waterkit-codec`.",
        attempts.join("; ")
    )
}

fn even_dimension(value: u32) -> Result<u32> {
    if value.is_multiple_of(2) {
        Ok(value)
    } else {
        value
            .checked_add(1)
            .ok_or_else(|| eyre!("Video dimension {value} overflows u32 when aligned to even"))
    }
}

/// Copies `src` (`src_w`×`src_h`) into `dst` (`dst_w`×`dst_h`, where `dst_w >= src_w`
/// and `dst_h >= src_h`), replicating the rightmost pixel and bottommost row for
/// odd-dimension padding.
fn pad_rgba_to_even(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32, dst: &mut Vec<u8>) {
    let src_row_bytes = (src_w as usize) * 4;
    let dst_row_bytes = (dst_w as usize) * 4;
    dst.resize(dst_row_bytes * (dst_h as usize), 0);
    for row in 0..(src_h as usize) {
        let src_row = &src[row * src_row_bytes..(row + 1) * src_row_bytes];
        let dst_row = &mut dst[row * dst_row_bytes..(row + 1) * dst_row_bytes];
        dst_row[..src_row_bytes].copy_from_slice(src_row);
        if dst_w > src_w {
            let last_pixel = &src_row[src_row_bytes - 4..src_row_bytes];
            dst_row[src_row_bytes..dst_row_bytes].copy_from_slice(last_pixel);
        }
    }
    if dst_h > src_h {
        let last_row_start = ((src_h as usize) - 1) * dst_row_bytes;
        let pad_row_start = (src_h as usize) * dst_row_bytes;
        dst.copy_within(
            last_row_start..last_row_start + dst_row_bytes,
            pad_row_start,
        );
    }
}

/// Converts Annex-B packets (Media Foundation / `MediaCodec`) to 4-byte
/// length-prefixed ISO BMFF framing and reports whether the access unit holds
/// an IDR/IRAP keyframe. Already length-prefixed packets (`VideoToolbox`) pass
/// through unchanged with their NALU headers inspected for keyframes.
fn normalize_sample(packet: Vec<u8>, codec: HardwareCodec) -> (Vec<u8>, bool) {
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "android"))]
    {
        let (converted, keyframe) = annex_b_to_length_prefixed(&packet, codec.is_hevc());
        if let Some(length_prefixed) = converted {
            return (length_prefixed, keyframe);
        }
    }
    let keyframe = length_prefixed_is_keyframe(&packet, codec.is_hevc());
    (packet, keyframe)
}

/// Inspects 4-byte big-endian length-prefixed NAL units for an IDR/IRAP slice.
const fn length_prefixed_is_keyframe(data: &[u8], is_hevc: bool) -> bool {
    let mut offset = 0usize;
    while offset + 4 <= data.len() {
        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if len == 0 || offset + len > data.len() {
            return false;
        }
        let header = data[offset];
        let is_idr = if is_hevc {
            matches!((header >> 1) & 0x3F, 16..=21)
        } else {
            (header & 0x1F) == 5
        };
        if is_idr {
            return true;
        }
        offset += len;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use waterui_preview_protocol::run::{Alpha, RawFrameStreamWriter};

    use super::*;

    #[test]
    fn length_prefixed_keyframe_detection_covers_h264_and_h265() {
        // H.264: non-IDR slice (type 1) vs IDR slice (type 5).
        let h264_non_idr = [0, 0, 0, 2, 0x41, 0x00];
        let h264_idr = [0, 0, 0, 2, 0x65, 0x88];
        assert!(!length_prefixed_is_keyframe(&h264_non_idr, false));
        assert!(length_prefixed_is_keyframe(&h264_idr, false));

        // H.265: trail slice (type 1 = 0x02) vs IDR_W_RADL (type 19 = 0x26).
        let h265_trail = [0, 0, 0, 3, 1 << 1, 0x01, 0x00];
        let h265_idr = [0, 0, 0, 3, 19 << 1, 0x01, 0x00];
        assert!(!length_prefixed_is_keyframe(&h265_trail, true));
        assert!(length_prefixed_is_keyframe(&h265_idr, true));
    }

    #[test]
    fn pad_rgba_to_even_replicates_right_and_bottom_edges() {
        // 1x1 red pixel padded to 2x2 becomes 4 identical red pixels.
        let src = [200_u8, 10, 20, 255];
        let mut dst = Vec::new();
        pad_rgba_to_even(&src, 1, 1, 2, 2, &mut dst);
        assert_eq!(dst.len(), 2 * 2 * 4);
        for &chunk in dst.as_chunks::<4>().0 {
            assert_eq!(chunk, [200, 10, 20, 255]);
        }
    }

    #[test]
    fn rejects_zero_fps_and_zero_dimensions() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("out.mp4");
        assert!(HardwareVideoEncoder::new(&path, 0, 64, 60).is_err());
        assert!(HardwareVideoEncoder::new(&path, 64, 0, 60).is_err());
        assert!(HardwareVideoEncoder::new(&path, 64, 64, 0).is_err());
    }

    /// Generates an animated `width`x`height` RGBA8 frame with a horizontal bar
    /// moving across the frame so inter-frame compression exercises real motion.
    fn synthetic_animated_rgba(width: u32, height: u32, frame_index: u32) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
        let bar_x = (frame_index * 16) % width.max(1);
        for y in 0..height {
            for x in 0..width {
                let in_bar = x >= bar_x && x < (bar_x + 24).min(width);
                let r = if in_bar {
                    240
                } else {
                    u8::try_from((x * 255) / width.max(1)).expect("normalized channel fits u8")
                };
                let g = if in_bar {
                    60
                } else {
                    u8::try_from((y * 255) / height.max(1)).expect("normalized channel fits u8")
                };
                let b = ((frame_index * 17) & 0xFF) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        rgba
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn hardware_encodes_raw_stream_to_playable_mp4_on_macos() {
        use waterkit_video_container::VideoReader;

        let dir = tempfile::tempdir().expect("temp dir");
        let raw_path = dir.path().join("frames.wuirf");
        let mp4_path = dir.path().join("preview.mp4");
        let width = 160;
        let height = 120;
        let fps = 30;
        let frame_count = 15_u32;

        let mut stream = RawFrameStreamWriter::create(&raw_path, fps).expect("create raw stream");
        for i in 0..frame_count {
            let rgba = synthetic_animated_rgba(width, height, i);
            let ts = Duration::from_nanos(u64::from(i) * 1_000_000_000 / u64::from(fps));
            stream
                .write_frame(width, height, rgba, Alpha::Straight, ts)
                .expect("write raw frame");
        }
        assert_eq!(
            stream.finish().expect("finish raw stream"),
            u64::from(frame_count)
        );

        let encoded_frames =
            encode_raw_frame_stream_to_mp4(&raw_path, &mp4_path).expect("hardware encode mp4");
        assert_eq!(encoded_frames, u64::from(frame_count));

        let metadata = fs::metadata(&mp4_path).expect("mp4 file exists");
        assert!(metadata.len() > 0, "mp4 file must be non-empty");

        let mut reader = VideoReader::open(&mp4_path).expect("mp4 container opens cleanly");
        assert_eq!(reader.dimensions(), (width, height));
        assert!(
            reader.codec_config().is_some_and(|cfg| !cfg.is_empty()),
            "mp4 track must carry avcC/hvcC codec configuration"
        );
        let mut read_samples = 0_u32;
        let mut saw_keyframe = false;
        while let Some((data, _pts, is_keyframe)) = reader.read_sample().expect("read mp4 sample") {
            assert!(!data.is_empty(), "encoded sample must be non-empty");
            saw_keyframe |= is_keyframe;
            read_samples += 1;
        }
        assert_eq!(read_samples, frame_count);
        assert!(saw_keyframe, "mp4 track must contain at least one keyframe");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn hardware_encodes_odd_dimensions_padded_to_even_on_macos() {
        use waterkit_video_container::VideoReader;

        let dir = tempfile::tempdir().expect("temp dir");
        let mp4_path = dir.path().join("odd.mp4");
        let mut encoder = HardwareVideoEncoder::new(&mp4_path, 121, 91, 30)
            .expect("open hardware encoder for odd dimensions");
        for i in 0..5_u32 {
            let rgba = synthetic_animated_rgba(121, 91, i);
            encoder.push_rgba(&rgba).expect("encode odd frame");
        }
        assert_eq!(encoder.finish().expect("finish odd mp4"), 5);

        let reader = VideoReader::open(&mp4_path).expect("open odd mp4");
        assert_eq!(reader.dimensions(), (122, 92));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fails_fast_when_hardware_encoding_is_unavailable_on_linux() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mp4_path = dir.path().join("out.mp4");
        let error = HardwareVideoEncoder::new(&mp4_path, 160, 120, 30).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Hardware video encoding is unavailable"),
            "unexpected error: {error}"
        );
    }
}

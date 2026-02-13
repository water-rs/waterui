//! A Photo component that displays an image from a URL.
//!
//! The Photo component fetches an image from a URL, decodes it, and displays
//! it using the GPU-accelerated [`Image`] view.
//!
//! # Example
//!
//! ```ignore
//! use waterui_media::Photo;
//! use waterui_media::url::Url;
//!
//! let url = Url::parse("https://example.com/image.jpg").unwrap();
//! let photo = Photo::new(url);
//! ```

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::Url;
use crate::image::Image;
use executor_core::spawn_local;
use futures::StreamExt;
use image::GenericImageView;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::{Environment, View};

/// A photo component that displays an image from a URL.
///
/// Photo fetches the image asynchronously and displays it using the
/// GPU-accelerated [`Image`] view once loaded.
///
/// # Example
///
/// ```ignore
/// use waterui_media::Photo;
///
/// Photo::new("https://example.com/image.jpg")
/// ```
pub struct Photo {
    /// The URL of the image to display.
    url: Url,
    /// Event handler for photo loading events.
    on_event: Option<Box<dyn Fn(Event) + 'static>>,
}

impl core::fmt::Debug for Photo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Photo")
            .field("url", &self.url)
            .field("on_event", &self.on_event.is_some())
            .finish()
    }
}

/// Events emitted by the Photo component.
#[derive(Debug, Clone)]
pub enum Event {
    /// The image has finished loading.
    Loaded,
    /// The image has failed to load.
    Error(String),
}

impl Photo {
    /// Creates a new `Photo` component with the specified image source URL.
    ///
    /// # Arguments
    ///
    /// * `source` - The URL of the image to display.
    #[must_use]
    pub fn new(source: impl Into<Url>) -> Self {
        Self {
            url: source.into(),
            on_event: None,
        }
    }

    /// Sets the event handler for the photo.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use waterui_media::{Photo, photo::Event};
    ///
    /// let photo = Photo::new(url)
    ///     .on_event(|event| {
    ///         match event {
    ///             Event::Loaded => println!("Image loaded!"),
    ///             Event::Error(msg) => println!("Error: {}", msg),
    ///         }
    ///     });
    /// ```
    #[must_use]
    pub fn on_event(mut self, handler: impl Fn(Event) + 'static) -> Self {
        self.on_event = Some(Box::new(handler));
        self
    }
}

impl View for Photo {
    fn body(self, _env: &Environment) -> impl View {
        let (handler, view) = Dynamic::new();
        // Set initial empty view (loading state)
        handler.set(());

        spawn_load_task(self.url, self.on_event, handler);

        view
    }
}

fn spawn_load_task(
    url: Url,
    on_event: Option<Box<dyn Fn(Event) + 'static>>,
    handler: DynamicHandler,
) {
    spawn_local(async move {
        let mut last_fingerprint: Option<u64> = None;
        match fetch_and_decode_streaming(url, |decoded| {
            publish_decoded_frame(&handler, &mut last_fingerprint, decoded);
        })
        .await
        {
            Ok(()) => {
                if let Some(on_event) = on_event {
                    on_event(Event::Loaded);
                }
            }
            Err(e) => {
                tracing::error!("[Photo] Failed to load: {}", e);
                if let Some(on_event) = on_event {
                    on_event(Event::Error(e));
                }
            }
        }
    })
    .detach();
}

/// Fetch and decode an image from a URL with streaming/progressive updates.
///
/// The callback is invoked for every published frame, including the final full-quality frame.
async fn fetch_and_decode_streaming(
    url: Url,
    mut on_decoded_frame: impl FnMut(DecodedRgba),
) -> Result<(), String> {
    // Fetch the image data using redirect-following client
    use zenwave::{Client, Method, redirect::FollowRedirect};
    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, url.as_str())
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(alloc::format!("HTTP error: {}", response.status()));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    let mut body = response.into_body();
    let mut data = Vec::new();
    let mut progressive_state = ProgressiveDecodeState::new();

    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if chunk.is_empty() {
            continue;
        }

        data.extend_from_slice(&chunk);
        let total_len = data.len();

        if progressive_state.should_attempt(total_len, content_type.as_deref(), &data) {
            let snapshot = data.clone();
            let progressive_decoded =
                blocking::unblock(move || decode_progressive_frame(&snapshot)).await;
            progressive_state.record_attempt(total_len);
            if let Some(decoded) = progressive_decoded {
                on_decoded_frame(decoded);
            }
        }
    }

    if data.is_empty() {
        return Err(String::from("image response body was empty"));
    }

    // Always emit the final full decode (platform-aware routing).
    let final_decoded = blocking::unblock(move || decode_to_rgba8(&data)).await?;
    on_decoded_frame(final_decoded);
    Ok(())
}

#[derive(Debug, Clone)]
struct DecodedRgba {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    pixel_format: waterkit_codec::DecodedPixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EncodedCodec {
    Av1,
    Hevc,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct ProgressiveDecodeState {
    attempts: usize,
    next_attempt_at: usize,
}

impl ProgressiveDecodeState {
    const FIRST_ATTEMPT_BYTES: usize = 24 * 1024;
    const ATTEMPT_STEP_BYTES: usize = 96 * 1024;
    const MAX_ATTEMPTS: usize = 10;
    const MAX_BUFFER_BYTES: usize = 8 * 1024 * 1024;

    const fn new() -> Self {
        Self {
            attempts: 0,
            next_attempt_at: Self::FIRST_ATTEMPT_BYTES,
        }
    }

    fn should_attempt(&self, total_len: usize, content_type: Option<&str>, data: &[u8]) -> bool {
        if self.attempts >= Self::MAX_ATTEMPTS
            || total_len < self.next_attempt_at
            || total_len > Self::MAX_BUFFER_BYTES
        {
            return false;
        }
        is_progressive_candidate(content_type, data)
    }

    fn record_attempt(&mut self, total_len: usize) {
        self.attempts += 1;
        self.next_attempt_at = total_len.saturating_add(Self::ATTEMPT_STEP_BYTES);
    }
}

fn publish_decoded_frame(
    handler: &DynamicHandler,
    last_fingerprint: &mut Option<u64>,
    decoded: DecodedRgba,
) {
    let fingerprint = frame_fingerprint(&decoded);
    if *last_fingerprint == Some(fingerprint) {
        return;
    }
    *last_fingerprint = Some(fingerprint);

    let image = match decoded.pixel_format {
        waterkit_codec::DecodedPixelFormat::Rgba8UnormSrgb => {
            Image::new(decoded.pixels, decoded.width, decoded.height)
        }
        waterkit_codec::DecodedPixelFormat::Rgba16Float => {
            Image::new_rgba16f(decoded.pixels, decoded.width, decoded.height)
        }
    };
    handler.set(image);
}

fn frame_fingerprint(decoded: &DecodedRgba) -> u64 {
    let len = decoded.pixels.len();
    if len == 0 {
        return 0;
    }
    let first = decoded.pixels[0] as u64;
    let mid = decoded.pixels[len / 2] as u64;
    let last = decoded.pixels[len - 1] as u64;
    ((decoded.width as u64) << 32)
        ^ (decoded.height as u64)
        ^ ((len as u64) << 8)
        ^ first
        ^ (mid << 16)
        ^ (last << 24)
}

fn decode_progressive_frame(data: &[u8]) -> Option<DecodedRgba> {
    match detect_codec(data) {
        EncodedCodec::Hevc | EncodedCodec::Av1 => None,
        EncodedCodec::Other => decode_with_software_fallback(data).ok(),
    }
}

fn is_progressive_candidate(content_type: Option<&str>, data: &[u8]) -> bool {
    if matches!(detect_codec(data), EncodedCodec::Hevc | EncodedCodec::Av1) {
        return false;
    }

    if let Ok(format) = image::guess_format(data) {
        return matches!(
            format,
            image::ImageFormat::Jpeg
                | image::ImageFormat::Png
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
                | image::ImageFormat::Bmp
                | image::ImageFormat::Ico
                | image::ImageFormat::Tiff
        );
    }

    if let Some(content_type) = content_type {
        let lower = content_type.to_ascii_lowercase();
        if lower.contains("avif") || lower.contains("heic") || lower.contains("heif") {
            return false;
        }
    }
    true
}

fn decode_to_rgba8(data: &[u8]) -> Result<DecodedRgba, String> {
    match detect_codec(data) {
        EncodedCodec::Hevc => decode_hevc_platform_only(data),
        EncodedCodec::Av1 => decode_av1_prefer_platform(data),
        EncodedCodec::Other => decode_other_prefer_platform(data),
    }
}

fn decode_hevc_platform_only(data: &[u8]) -> Result<DecodedRgba, String> {
    decode_with_platform(data).map_err(|e| {
        alloc::format!(
            "H265/HEVC image decode requires platform codec support; {e}. \
             No software HEVC fallback is provided"
        )
    })
}

fn decode_av1_prefer_platform(data: &[u8]) -> Result<DecodedRgba, String> {
    match decode_with_platform(data) {
        Ok(decoded) => Ok(decoded),
        Err(platform_err) => decode_with_software_fallback(data).map_err(|software_err| {
            alloc::format!(
                "AV1 decode failed: platform path error: {platform_err}; software path error: \
                 {software_err}"
            )
        }),
    }
}

fn decode_other_prefer_platform(data: &[u8]) -> Result<DecodedRgba, String> {
    match decode_with_platform(data) {
        Ok(decoded) => Ok(decoded),
        Err(_) => decode_with_software_fallback(data),
    }
}

fn decode_with_software_fallback(data: &[u8]) -> Result<DecodedRgba, String> {
    let img = match image::load_from_memory(data) {
        Ok(img) => img,
        Err(primary_err) => {
            let Some(patched) = patch_heif_brand_to_avif(data) else {
                return Err(alloc::format!("Image decode failed: {}", primary_err));
            };
            image::load_from_memory(&patched).map_err(|fallback_err| {
                alloc::format!(
                    "Image decode failed: {primary_err}; HEIF fallback failed: \
                     {fallback_err}. HEIF software decode only supports AV1 payloads"
                )
            })?
        }
    };
    let (width, height) = img.dimensions();
    let pixels = img.into_rgba8().into_raw();
    Ok(DecodedRgba {
        pixels,
        width,
        height,
        pixel_format: waterkit_codec::DecodedPixelFormat::Rgba8UnormSrgb,
    })
}

fn decode_with_platform(data: &[u8]) -> Result<DecodedRgba, String> {
    let decoded = waterkit_codec::decode_image(data).map_err(|e| e.to_string())?;
    Ok(DecodedRgba {
        pixels: decoded.pixels,
        width: decoded.width,
        height: decoded.height,
        pixel_format: decoded.pixel_format,
    })
}

fn detect_codec(data: &[u8]) -> EncodedCodec {
    let Some(ftyp) = parse_ftyp(data) else {
        return EncodedCodec::Other;
    };

    if is_avif_brand(&ftyp.major) {
        return EncodedCodec::Av1;
    }
    if is_hevc_brand(&ftyp.major) {
        return EncodedCodec::Hevc;
    }

    if ftyp.compat.iter().any(is_avif_brand) {
        return EncodedCodec::Av1;
    }
    if ftyp.compat.iter().any(is_hevc_brand) {
        return EncodedCodec::Hevc;
    }

    if is_generic_heif_brand(&ftyp.major) || ftyp.compat.iter().any(is_generic_heif_brand) {
        if contains_fourcc(data, b"av01") {
            return EncodedCodec::Av1;
        }
        if contains_fourcc(data, b"hvc1") || contains_fourcc(data, b"hev1") {
            return EncodedCodec::Hevc;
        }
    }
    EncodedCodec::Other
}

#[derive(Debug, Clone)]
struct Ftyp {
    major: [u8; 4],
    compat: Vec<[u8; 4]>,
}

fn parse_ftyp(data: &[u8]) -> Option<Ftyp> {
    if data.len() < 16 {
        return None;
    }
    let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if box_size < 16 || box_size > data.len() || &data[4..8] != b"ftyp" {
        return None;
    }
    let major = [data[8], data[9], data[10], data[11]];
    let compat_bytes = &data[16..box_size];
    if compat_bytes.len() % 4 != 0 {
        return None;
    }
    let mut compat = Vec::with_capacity(compat_bytes.len() / 4);
    for chunk in compat_bytes.chunks_exact(4) {
        compat.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Some(Ftyp { major, compat })
}

fn contains_fourcc(haystack: &[u8], needle: &[u8; 4]) -> bool {
    haystack.windows(4).any(|w| w == needle)
}

fn is_avif_brand(brand: &[u8; 4]) -> bool {
    matches!(brand, b"avif" | b"avis")
}

fn is_hevc_brand(brand: &[u8; 4]) -> bool {
    matches!(brand, b"heic" | b"heix" | b"hevc" | b"hevx")
}

fn is_generic_heif_brand(brand: &[u8; 4]) -> bool {
    matches!(brand, b"mif1" | b"msf1" | b"heif")
}

fn patch_heif_brand_to_avif(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 16 {
        return None;
    }
    let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if box_size < 16 || box_size > data.len() || &data[4..8] != b"ftyp" {
        return None;
    }

    let major = [data[8], data[9], data[10], data[11]];
    let is_heif = is_heif_brand(&major)
        || data[16..box_size]
            .chunks_exact(4)
            .any(|brand| is_heif_brand(&[brand[0], brand[1], brand[2], brand[3]]));
    if !is_heif {
        return None;
    }

    let mut patched = data.to_vec();
    patched[8..12].copy_from_slice(b"avif");

    let mut has_avif_compat = false;
    let mut first_heif_compat_offset: Option<usize> = None;
    for offset in (16..box_size).step_by(4) {
        if offset + 4 > box_size {
            break;
        }
        let brand = &patched[offset..offset + 4];
        if brand == b"avif" || brand == b"avis" {
            has_avif_compat = true;
            break;
        }
        if first_heif_compat_offset.is_none()
            && is_heif_brand(&[brand[0], brand[1], brand[2], brand[3]])
        {
            first_heif_compat_offset = Some(offset);
        }
    }
    if !has_avif_compat {
        if let Some(offset) = first_heif_compat_offset {
            patched[offset..offset + 4].copy_from_slice(b"avif");
        } else if box_size >= 20 {
            patched[16..20].copy_from_slice(b"avif");
        }
    }

    Some(patched)
}

fn is_heif_brand(brand: &[u8; 4]) -> bool {
    is_generic_heif_brand(brand) || is_hevc_brand(brand)
}

/// Convenience constructor for building a `Photo` component inline.
#[must_use]
pub fn photo(source: impl Into<Url>) -> Photo {
    Photo::new(source)
}

#[cfg(test)]
mod tests {
    use super::{
        EncodedCodec, ProgressiveDecodeState, decode_progressive_frame, detect_codec,
        fetch_and_decode_streaming, is_progressive_candidate, patch_heif_brand_to_avif,
    };
    use crate::Url;

    #[test]
    fn patches_heif_major_and_compatible_brand() {
        let heif = [
            0, 0, 0, 24, b'f', b't', b'y', b'p', b'h', b'e', b'i', b'c', 0, 0, 0, 0, b'm', b'i',
            b'f', b'1', b'h', b'e', b'i', b'c',
        ];
        let patched = patch_heif_brand_to_avif(&heif).expect("should patch heif");
        assert_eq!(&patched[8..12], b"avif");
        assert_eq!(&patched[16..20], b"avif");
    }

    #[test]
    fn ignores_non_heif_ftyp() {
        let png_like = [
            0, 0, 0, 24, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 0, 0, b'i', b's',
            b'o', b'm', b'm', b'p', b'4', b'2',
        ];
        assert!(patch_heif_brand_to_avif(&png_like).is_none());
    }

    #[test]
    fn detects_avif_codec() {
        let avif = [
            0, 0, 0, 20, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', 0, 0, 0, 0, b'm', b'i',
            b'f', b'1',
        ];
        assert_eq!(detect_codec(&avif), EncodedCodec::Av1);
    }

    #[test]
    fn detects_hevc_codec() {
        let heic = [
            0, 0, 0, 20, b'f', b't', b'y', b'p', b'h', b'e', b'i', b'c', 0, 0, 0, 0, b'm', b'i',
            b'f', b'1',
        ];
        assert_eq!(detect_codec(&heic), EncodedCodec::Hevc);
    }

    #[test]
    fn detects_generic_heif_with_av1_payload() {
        let mut heif = vec![
            0, 0, 0, 20, b'f', b't', b'y', b'p', b'm', b'i', b'f', b'1', 0, 0, 0, 0, b'm', b's',
            b'f', b'1',
        ];
        heif.extend_from_slice(b"xxxxav01yyyy");
        assert_eq!(detect_codec(&heif), EncodedCodec::Av1);
    }

    #[test]
    fn detects_generic_heif_with_hevc_payload() {
        let mut heif = vec![
            0, 0, 0, 20, b'f', b't', b'y', b'p', b'm', b'i', b'f', b'1', 0, 0, 0, 0, b'm', b's',
            b'f', b'1',
        ];
        heif.extend_from_slice(b"xxxxhvc1yyyy");
        assert_eq!(detect_codec(&heif), EncodedCodec::Hevc);
    }

    #[test]
    fn progressive_candidates_reject_heif_family() {
        let heic = [
            0, 0, 0, 20, b'f', b't', b'y', b'p', b'h', b'e', b'i', b'c', 0, 0, 0, 0, b'm', b'i',
            b'f', b'1',
        ];
        assert!(!is_progressive_candidate(Some("image/heic"), &heic));
    }

    #[test]
    fn progressive_candidates_allow_jpeg() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
        assert!(is_progressive_candidate(Some("image/jpeg"), &jpeg));
    }

    #[test]
    fn progressive_state_respects_thresholds() {
        let mut state = ProgressiveDecodeState::new();
        let jpeg = [0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
        assert!(!state.should_attempt(8 * 1024, Some("image/jpeg"), &jpeg));
        assert!(state.should_attempt(32 * 1024, Some("image/jpeg"), &jpeg));
        state.record_attempt(32 * 1024);
        assert!(!state.should_attempt(40 * 1024, Some("image/jpeg"), &jpeg));
    }

    #[test]
    fn progressive_decode_skips_avif_like_data() {
        let avif = [
            0, 0, 0, 20, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', 0, 0, 0, 0, b'm', b'i',
            b'f', b'1',
        ];
        assert!(decode_progressive_frame(&avif).is_none());
    }

    #[test]
    #[ignore = "requires network access to real image URLs"]
    fn streaming_decode_real_images_smoke() {
        futures::executor::block_on(async {
            #[allow(unused_mut)]
            let mut cases = vec![
                (
                    "jpeg",
                    "https://raw.githubusercontent.com/libjpeg-turbo/libjpeg-turbo/main/testimages/testorig.jpg",
                ),
                (
                    "tiff",
                    "https://raw.githubusercontent.com/python-pillow/Pillow/main/Tests/images/hopper.tif",
                ),
            ];
            #[cfg(not(target_vendor = "apple"))]
            {
                cases.push((
                    "avif",
                    "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.8bpc.yuv420.avif",
                ));
            }
            #[cfg(target_os = "android")]
            {
                cases.push((
                    "heic",
                    "https://raw.githubusercontent.com/strukturag/libheif/master/examples/example.heic",
                ));
            }

            let mut validated_cases = 0usize;
            for (name, raw_url) in cases {
                let url: Url = raw_url.parse().expect("url should parse");
                let mut frames = 0usize;
                let result = fetch_and_decode_streaming(url, |_| {
                    frames += 1;
                })
                .await;
                let Err(e) = result else {
                    assert!(frames >= 1, "{name} should publish at least one frame");
                    validated_cases += 1;
                    continue;
                };
                eprintln!("[streaming_decode_real_images_smoke] skip {name}: {e}");
            }

            if validated_cases == 0 {
                eprintln!(
                    "[streaming_decode_real_images_smoke] no cases validated; network may be unavailable in this environment"
                );
                return;
            }
        });
    }

    #[cfg(any(target_vendor = "apple", target_os = "android"))]
    #[test]
    #[ignore = "requires network access and platform HDR AVIF decode support"]
    fn hdr_avif_decode_real_image_smoke() {
        futures::executor::block_on(async {
            use zenwave::{Client, Method, redirect::FollowRedirect};

            let mut client = FollowRedirect::new(zenwave::client());
            let response = match client
                .method(
                    Method::GET,
                    "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile2.10bpc.yuv420.avif",
                )
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    eprintln!("[hdr_avif_decode_real_image_smoke] skip due to network error: {e}");
                    return;
                }
            };
            assert!(
                response.status().is_success(),
                "status={}",
                response.status()
            );
            let bytes = response
                .into_body()
                .into_bytes()
                .await
                .expect("body should load");

            let decoded =
                waterkit_codec::decode_image(&bytes).expect("platform decode should work");
            assert_eq!(
                decoded.pixel_format,
                waterkit_codec::DecodedPixelFormat::Rgba16Float,
                "10-bit AVIF should decode to RGBA16F on HDR-capable platform decoder path"
            );
            assert!(
                decoded.hdr,
                "10-bit AVIF should be marked as HDR from platform decoder"
            );
        });
    }
}

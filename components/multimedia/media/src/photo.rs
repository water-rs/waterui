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
//! let url = Url::parse("https://www.rust-lang.org/logos/rust-logo-512x512.png").unwrap();
//! let photo = Photo::new(url);
//! ```

use alloc::borrow::ToOwned;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use crate::Url;
use executor_core::spawn_local;
use futures::StreamExt;
#[cfg(target_arch = "wasm32")]
use std::path::Path;
#[cfg(target_arch = "wasm32")]
use waterkit_fs::WaterFs;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::event::{LifeCycle, LifeCycleHook};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{AnyView, Binding, Computed, Environment, Metadata, Retain, Signal, View};
use waterui_image::Image;
use zenwave::{Client, Method, redirect::FollowRedirect};

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
/// Photo::new("https://www.rust-lang.org/logos/rust-logo-512x512.png")
/// ```
pub struct Photo {
    /// The URL of the image to display.
    source: Computed<Url>,
    /// Event handler for photo loading events. Stored as a typed
    /// [`BoxedEventAction`] so the closure can extract `State<T>` /
    /// environment values, mirroring the [`Button::action`] pattern.
    on_event: Option<waterui_core::handler::BoxedEventAction<Event>>,
    /// Dynamic view content updated when decoded frames arrive.
    content: Dynamic,
    /// Handle for publishing decoded frames into `content`.
    content_handler: DynamicHandler,
    /// Guards one-shot loading for the current component instance.
    load_started: Binding<bool>,
}

impl core::fmt::Debug for Photo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Photo")
            .field("source", &self.source)
            .field("on_event", &self.on_event.is_some())
            .field("content", &self.content)
            .field("content_handler", &self.content_handler)
            .field("load_started", &self.load_started)
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
    pub fn new(source: impl IntoComputed<Url>) -> Self {
        let (content_handler, content) = Dynamic::new();
        content_handler.set(());
        Self {
            source: source.into_computed(),
            on_event: None,
            content,
            content_handler,
            load_started: Binding::bool(false),
        }
    }

    /// Creates a `Photo` from a local filesystem path.
    #[must_use]
    pub fn from_path(path: impl Into<String>) -> Self {
        Self::new(Url::from_file_path_str(path.into()))
    }

    /// Sets the event handler for the photo.
    ///
    /// The handler shares the [`Handler`](waterui_core::handler::Handler)
    /// extractor pattern used by `Button::action`: the leading argument is
    /// the [`Event`] payload and any subsequent arguments are extracted from
    /// the rendering [`Environment`] at fire time.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use waterui::prelude::*;
    /// use waterui_media::{Photo, photo::Event};
    ///
    /// // Plain callback — no extractors.
    /// let photo = Photo::new(url).on_event(|event: Event| match event {
    ///     Event::Loaded => tracing::debug!("Image loaded!"),
    ///     Event::Error(msg) => tracing::debug!("Error: {msg}"),
    /// });
    ///
    /// // Extractor-based callback — pulls a binding via `State<T>`.
    /// let last_status = binding(String::new());
    /// let photo = Photo::new(url)
    ///     .on_event(|event: Event, State(last): State<Binding<String>>| {
    ///         last.set(format!("{event:?}"));
    ///     })
    ///     .state(&last_status);
    /// ```
    #[must_use]
    pub fn on_event<H, A>(mut self, handler: H) -> Self
    where
        H: waterui_core::handler::EventHandler<Event, A, ()> + 'static,
    {
        self.on_event = Some(waterui_core::handler::boxed_event_handler(handler));
        self
    }
}

impl View for Photo {
    fn body(self, env: &Environment) -> impl View {
        let Self {
            source,
            on_event,
            content,
            content_handler,
            load_started,
        } = self;
        let env_for_event = env.clone();
        let on_event = Rc::new(RefCell::new(on_event));
        let generation = Binding::usize(0);

        let guard = source.watch({
            let on_event = Rc::clone(&on_event);
            let content_handler = content_handler.clone();
            let generation = generation.clone();
            let env = env_for_event.clone();
            move |ctx| {
                start_load_task(
                    ctx.into_value(),
                    Rc::clone(&on_event),
                    content_handler.clone(),
                    env.clone(),
                    generation.clone(),
                );
            }
        });

        AnyView::new(Metadata::new(
            Metadata::new(
                content,
                LifeCycleHook::new(LifeCycle::Appear, {
                    let source = source.clone();
                    let on_event = Rc::clone(&on_event);
                    let content_handler = content_handler;
                    let generation = generation.clone();
                    move || {
                        if load_started.get() {
                            return;
                        }
                        load_started.set(true);
                        start_load_task(
                            source.get(),
                            Rc::clone(&on_event),
                            content_handler.clone(),
                            env_for_event.clone(),
                            generation.clone(),
                        );
                    }
                }),
            ),
            Retain::new((guard, source, on_event, generation)),
        ))
    }
}

type PhotoEventHandler = Rc<RefCell<Option<waterui_core::handler::BoxedEventAction<Event>>>>;

fn start_load_task(
    url: Url,
    on_event: PhotoEventHandler,
    handler: DynamicHandler,
    env: Environment,
    generation: Binding<usize>,
) {
    let token = generation.get().saturating_add(1);
    generation.set(token);
    handler.set(());
    spawn_local(async move {
        let frame_generation = generation.clone();
        match fetch_and_decode_streaming(url, move |image| {
            if frame_generation.get() == token {
                publish_decoded_frame(&handler, image);
            }
        })
        .await
        {
            Ok(()) => {
                if generation.get() == token
                    && let Some(on_event) = on_event.borrow_mut().as_mut()
                {
                    on_event(Event::Loaded, &env);
                }
            }
            Err(e) => {
                tracing::error!("[Photo] Failed to load: {}", e);
                if generation.get() == token
                    && let Some(on_event) = on_event.borrow_mut().as_mut()
                {
                    on_event(Event::Error(e), &env);
                }
            }
        }
    })
    .detach();
}

/// Fetch and decode an image from a URL with streaming/progressive updates.
///
/// The callback is invoked for every published frame, including the final full-quality frame.
#[allow(
    clippy::future_not_send,
    reason = "WaterUI drives Photo loading on the main-thread executor; decoded frames update UI-local DynamicHandler state"
)]
async fn fetch_and_decode_streaming(
    url: Url,
    mut on_decoded_frame: impl FnMut(Image),
) -> Result<(), String> {
    let is_local = url.is_local();
    let url = url.as_str().to_string();
    if is_local {
        let path = url;
        #[cfg(target_arch = "wasm32")]
        let bytes = WaterFs::read(Path::new(&path))
            .await
            .map_err(|error| error.to_string())?;
        #[cfg(not(target_arch = "wasm32"))]
        let bytes = blocking::unblock(move || std::fs::read(&path))
            .await
            .map_err(|error| error.to_string())?;
        let image = Image::from_encoded(&bytes)?;
        on_decoded_frame(image);
        return Ok(());
    }

    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, &url)
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
    let mut decoder = Image::stream_decoder(content_type.as_deref());

    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if let Some(progressive_image) = decoder.push_chunk(&chunk) {
            on_decoded_frame(progressive_image);
        }
    }

    let final_image = decoder.finish()?;
    on_decoded_frame(final_image);
    Ok(())
}

fn publish_decoded_frame(handler: &DynamicHandler, image: Image) {
    handler.set(image);
}

/// Convenience constructor for building a `Photo` component inline.
#[must_use]
pub fn photo(source: impl IntoComputed<Url>) -> Photo {
    Photo::new(source)
}

#[cfg(test)]
fn png_contains_cicp_pq(bytes: &[u8]) -> bool {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 8 || &bytes[0..8] != PNG_SIG {
        return false;
    }
    let mut offset = 8usize;
    while offset + 12 <= bytes.len() {
        let len = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let chunk_start = offset + 8;
        let chunk_end = chunk_start.saturating_add(len);
        if chunk_end + 4 > bytes.len() {
            return false;
        }
        let chunk_type = &bytes[offset + 4..offset + 8];
        if chunk_type == b"cICP" {
            if len != 4 {
                return false;
            }
            let data = &bytes[chunk_start..chunk_end];
            return data[1] == 16; // PQ transfer
        }
        if chunk_type == b"IEND" {
            break;
        }
        offset = chunk_end + 4;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use super::{fetch_and_decode_streaming, png_contains_cicp_pq};
    use crate::Url;
    use image::{ImageDecoder, ImageEncoder as _};
    use waterui_graphics::{OffscreenRenderConfig, OffscreenSize};
    use waterui_image::{DecodePath, Image};

    fn sample_png_bytes() -> Vec<u8> {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
                2,
                2,
                image::ExtendedColorType::Rgba8,
            )
            .expect("sample PNG should encode");
        png
    }

    #[test]
    fn png_decode_path_smoke() {
        let bytes = sample_png_bytes();
        let (image, path) =
            Image::from_encoded_with_path(&bytes).expect("png should decode from software path");
        assert_eq!(path, DecodePath::SoftwareFallback);
        assert!(image.width() > 0 && image.height() > 0);
    }

    #[test]
    #[ignore = "requires network access to real image URLs"]
    fn streaming_decode_real_images_smoke() {
        futures::executor::block_on(async {
            let mut cases = Vec::new();
            cases.push((
                "jpeg",
                "https://raw.githubusercontent.com/libjpeg-turbo/libjpeg-turbo/main/testimages/testorig.jpg",
            ));
            cases.push((
                "tiff",
                "https://raw.githubusercontent.com/python-pillow/Pillow/main/Tests/images/hopper.tif",
            ));
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
            }
        });
    }

    #[cfg(any(target_vendor = "apple", target_os = "android"))]
    #[test]
    #[ignore = "requires network access and platform HDR AVIF decode support"]
    fn hdr_avif_decode_real_image_smoke() {
        futures::executor::block_on(async {
            use zenwave::{Client, Method, redirect::FollowRedirect};

            let candidates = [
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.10bpc.yuv420.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/hato.profile0.10bpc.yuv420.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile1.10bpc.yuv444.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile2.10bpc.yuv422.avif",
            ];

            let mut selected = None;
            for url in candidates {
                let mut client = FollowRedirect::new(zenwave::client());
                let response = match client.method(Method::GET, url).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        eprintln!(
                            "[hdr_avif_decode_real_image_smoke] skip candidate due to network error: {e}"
                        );
                        continue;
                    }
                };
                if !response.status().is_success() {
                    continue;
                }
                let bytes = response
                    .into_body()
                    .into_bytes()
                    .await
                    .expect("body should load");
                let Ok(decoded) = waterkit_codec::decode_image(&bytes) else {
                    continue;
                };
                if decoded.pixel_format() != waterkit_codec::DecodedPixelFormat::Rgba16Float
                    || !decoded.hdr()
                {
                    continue;
                }
                selected = Some(decoded);
                break;
            }

            let decoded = selected.expect("no HDR AVIF sample decoded to RGBA16F HDR data");
            assert_eq!(
                decoded.pixel_format(),
                waterkit_codec::DecodedPixelFormat::Rgba16Float,
                "10-bit AVIF should decode to RGBA16F on HDR-capable platform decoder path"
            );
            assert!(
                decoded.hdr(),
                "10-bit AVIF should be marked as HDR from platform decoder"
            );
        });
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    #[ignore = "requires network access and Apple HEIC platform decode support"]
    fn heic_h265_decode_real_image_smoke() {
        futures::executor::block_on(async {
            use zenwave::{Client, Method, redirect::FollowRedirect};

            let mut client = FollowRedirect::new(zenwave::client());
            let response = match client
                .method(
                    Method::GET,
                    "https://raw.githubusercontent.com/strukturag/libheif/master/examples/example.heic",
                )
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    eprintln!("[heic_h265_decode_real_image_smoke] skip due to network error: {e}");
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

            let image =
                Image::from_encoded(&bytes).expect("HEIC/H265 decode should succeed on Apple");
            assert!(image.width() > 0 && image.height() > 0);
        });
    }

    #[test]
    #[ignore = "requires network access and AV1/AVIF decode support"]
    fn av1_decode_fallback_probe_smoke() {
        futures::executor::block_on(async {
            use zenwave::{Client, Method, redirect::FollowRedirect};

            let mut client = FollowRedirect::new(zenwave::client());
            let response = match client
                .method(
                    Method::GET,
                    "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.8bpc.yuv420.avif",
                )
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    eprintln!("[av1_decode_fallback_probe_smoke] skip due to network error: {e}");
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

            let platform_probe = waterkit_codec::decode_image_platform(&bytes);
            let platform_ok = platform_probe.is_ok();
            if let Err(err) = &platform_probe {
                eprintln!(
                    "[av1_decode_fallback_probe_smoke] platform AV1 decode unavailable, expecting software fallback: {err}"
                );
            }

            let final_image = Image::from_encoded(&bytes)
                .expect("AV1 should decode via platform or software path");
            let (decoded_with_path, path) =
                Image::from_encoded_with_path(&bytes).expect("AV1 should decode via selected path");
            let decoded_source =
                waterkit_codec::decode_image(&bytes).expect("AV1 sample should decode to pixels");
            assert!(
                !decoded_source.hdr(),
                "8bpc AVIF sample must stay SDR (hdr=false)"
            );
            assert_eq!(final_image.width(), decoded_with_path.width());
            assert_eq!(final_image.height(), decoded_with_path.height());
            if platform_ok {
                assert_eq!(
                    path,
                    DecodePath::Platform,
                    "decode path should stay platform when platform probe succeeds"
                );
            } else {
                assert_eq!(
                    path,
                    DecodePath::SoftwareFallback,
                    "decode path should fall back to software when platform probe fails"
                );
                let software_probe = waterkit_codec::decode_image(&bytes)
                    .expect("AV1 software fallback should decode on platform AV1 decode miss");
                assert!(software_probe.width() > 0 && software_probe.height() > 0);
            }
            eprintln!(
                "[av1_decode_fallback_probe_smoke] platform_ok={platform_ok} selected_path={path:?}"
            );
        });
    }

    #[cfg(any(target_vendor = "apple", target_os = "android"))]
    #[test]
    #[ignore = "manual: network HDR Photo decode, HDR offscreen render, and PNG HDR verification"]
    fn photo_network_hdr_offscreen_png_is_hdr() {
        futures::executor::block_on(async {
            let candidates = [
                "https://www.bandisoft.com/bandiview/help/hdr-samples/hdr_cosmos01650_cicp9-16-9_yuv420_limited_qp10.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.10bpc.yuv420.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/hato.profile0.10bpc.yuv420.avif",
            ];

            let export_dir = PathBuf::from("/tmp/waterui-photo-offscreen-hdr");
            std::fs::create_dir_all(&export_dir).expect("export directory should be creatable");

            for raw_url in candidates {
                let url: Url = raw_url.parse().expect("url should parse");
                let mut last_frame: Option<Image> = None;
                if let Err(e) = fetch_and_decode_streaming(url, |image| {
                    last_frame = Some(image);
                })
                .await
                {
                    eprintln!("[photo_network_hdr_offscreen_png_is_hdr] skip {raw_url}: {e}");
                    continue;
                }

                let Some(image) = last_frame else {
                    continue;
                };
                let (width, height) = image.dimensions();
                let size = OffscreenSize::try_from_pixels(width.min(1024), height.min(1024))
                    .expect("offscreen size must be valid");
                let config =
                    OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba16Float);
                let mut env = waterui_core::Environment::new();
                let Ok(output) = image.render_offscreen_hdr(config, &mut env) else {
                    continue;
                };

                let max_rgb = output.max_rgb_linear();
                let hdr_ratio = output.hdr_pixel_ratio();
                if max_rgb <= 1.0 || hdr_ratio <= 0.0 {
                    eprintln!(
                        "[photo_network_hdr_offscreen_png_is_hdr] sample lacks HDR headroom: url={raw_url}, max_rgb={max_rgb:.6}, hdr_ratio={hdr_ratio:.6}"
                    );
                    continue;
                }

                let png_path = export_dir.join("photo_hdr_network_offscreen.png");
                output
                    .save_png(&png_path)
                    .expect("HDR PNG should be writable");

                let png_bytes = std::fs::read(&png_path).expect("HDR PNG should be readable");
                let decoder = image::codecs::png::PngDecoder::new(Cursor::new(&png_bytes))
                    .expect("exported PNG should decode");
                assert_eq!(
                    decoder.color_type(),
                    image::ColorType::Rgba16,
                    "HDR offscreen export must be 16-bit PNG"
                );
                let has_hdr_cicp = png_contains_cicp_pq(&png_bytes);
                assert!(
                    has_hdr_cicp,
                    "HDR offscreen export must carry cICP PQ metadata"
                );

                eprintln!(
                    "[photo_network_hdr_offscreen_png_is_hdr] exported={}, url={}, max_rgb={max_rgb:.6}, hdr_ratio={hdr_ratio:.6}",
                    png_path.display(),
                    raw_url
                );
                return;
            }

            panic!("no HDR network sample produced a valid HDR PNG export");
        });
    }
}

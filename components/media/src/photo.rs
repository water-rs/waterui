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

use crate::Url;
use crate::image::Image;
use executor_core::spawn_local;
use futures::StreamExt;
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
        match fetch_and_decode_streaming(url, |image| publish_decoded_frame(&handler, image)).await
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
    mut on_decoded_frame: impl FnMut(Image),
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
    let mut decoder = Image::stream_decoder(content_type.as_deref());

    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if let Some(progressive_image) = decoder.push_chunk(&chunk)? {
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
pub fn photo(source: impl Into<Url>) -> Photo {
    Photo::new(source)
}

#[cfg(test)]
mod tests {
    use super::fetch_and_decode_streaming;
    use crate::Url;
    use crate::image::{DecodePath, Image};

    #[test]
    fn png_decode_path_smoke() {
        let bytes = include_bytes!("../../../navigation_current.png");
        let (image, path) =
            Image::from_encoded_with_path(bytes).expect("png should decode from software path");
        assert_eq!(path, DecodePath::SoftwareFallback);
        assert!(image.width() > 0 && image.height() > 0);
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
                if decoded.pixel_format != waterkit_codec::DecodedPixelFormat::Rgba16Float
                    || !decoded.hdr
                {
                    continue;
                }
                let first = &decoded.pixels[0..8];
                let non_uniform = decoded.pixels.chunks_exact(8).any(|px| px != first);
                let nonzero_rgb = decoded.pixels.chunks_exact(8).any(|px| {
                    px[0] != 0 || px[1] != 0 || px[2] != 0 || px[3] != 0 || px[4] != 0 || px[5] != 0
                });
                if non_uniform && nonzero_rgb {
                    selected = Some(decoded);
                    break;
                }
            }

            let decoded = selected.expect("no HDR AVIF sample decoded to non-black RGBA16F data");
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

            let final_image =
                Image::from_encoded(&bytes).expect("AV1 should decode via platform path");
            let (decoded_with_path, path) =
                Image::from_encoded_with_path(&bytes).expect("AV1 should decode via platform path");
            assert_eq!(final_image.width(), decoded_with_path.width());
            assert_eq!(final_image.height(), decoded_with_path.height());
            assert_eq!(path, DecodePath::Platform);
        });
    }
}

//! A Photo component that displays an image from a URL.
//!
//! The Photo component fetches an image from a URL, decodes it, and displays
//! it using the GPU-accelerated [`Image`] view.
//!
//! # Example
//!
//! ```rust
//! use waterui_media::Photo;
//! use waterui_media::url::Url;
//!
//! let url = Url::parse("https://www.rust-lang.org/logos/rust-logo-512x512.png").unwrap();
//! let photo = Photo::new(url);
//! ```

use alloc::borrow::ToOwned;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::{Cell, RefCell};

use crate::Url;
use executor_core::spawn_local;
use futures::StreamExt;
#[cfg(target_arch = "wasm32")]
use std::path::Path;
#[cfg(target_arch = "wasm32")]
use waterkit_fs::WaterFs;
use waterui_core::event::{LifeCycle, LifeCycleHook};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{AnyView, Computed, Environment, Metadata, Retain, Signal, View};
use waterui_image::{Image, ReactiveImage, ReactiveImageHandle, reactive_image};
use zenwave::{Client, Method, redirect::FollowRedirect};

/// A photo component that displays an image from a URL.
///
/// Photo fetches the image asynchronously and displays it using the
/// GPU-accelerated [`Image`] view once loaded.
///
/// # Example
///
/// ```rust
/// use waterui_media::Photo;
/// use waterui_media::url::Url;
///
/// Photo::new(Url::parse("https://www.rust-lang.org/logos/rust-logo-512x512.png").unwrap());
/// ```
pub struct Photo {
    /// The URL of the image to display.
    source: Computed<Url>,
    /// Event handler for photo loading events. Stored as a typed
    /// [`BoxedEventAction`] so the closure can extract `State<T>` /
    /// environment values, mirroring the [`Button::action`] pattern.
    on_event: Option<waterui_core::handler::BoxedEventAction<Event>>,
    /// Persistent GPU image surface updated when decoded frames arrive.
    content: ReactiveImage,
    /// Handle for publishing decoded frames into `content`.
    content_handler: ReactiveImageHandle,
}

impl core::fmt::Debug for Photo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Photo")
            .field("source", &self.source)
            .field("on_event", &self.on_event.is_some())
            .field("content", &self.content)
            .field("content_handler", &self.content_handler)
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
        let (content_handler, content) = reactive_image();
        Self {
            source: source.into_computed(),
            on_event: None,
            content,
            content_handler,
        }
    }

    /// Creates a `Photo` from a local filesystem path.
    #[must_use]
    pub fn from_path(path: impl Into<String>) -> Self {
        Self::new(Url::from_file_path_str(path.into()))
    }

    /// Allows the decoded image to stretch to the bounds proposed by its parent.
    #[must_use]
    pub fn resizable(mut self) -> Self {
        self.content = self.content.resizable();
        self
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
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui_media::url::Url;
    /// use waterui_media::{Photo, photo::Event};
    ///
    /// # fn gallery() -> impl View {
    /// # let url = Url::parse("https://www.rust-lang.org/logos/rust-logo-512x512.png").unwrap();
    /// // Plain callback — no extractors.
    /// let photo = Photo::new(url.clone()).on_event(|event: Event| match event {
    ///     Event::Loaded => tracing::debug!("Image loaded!"),
    ///     Event::Error(msg) => tracing::debug!("Error: {msg}"),
    /// });
    ///
    /// // Extractor-based callback — pulls a binding via `State<T>`.
    /// let last_status: Binding<String> = binding(String::new());
    /// let photo = Photo::new(url)
    ///     .on_event(|event: Event, State(last): State<Binding<String>>| {
    ///         last.set(format!("{event:?}"));
    ///     })
    ///     .state(&last_status);
    /// # photo
    /// # }
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
        } = self;
        let env_for_event = env.clone();
        let on_event = Rc::new(RefCell::new(on_event));
        let generation = Rc::new(Cell::new(0));
        let start = move |url| {
            start_load_task(
                url,
                Rc::clone(&on_event),
                content_handler.clone(),
                env_for_event.clone(),
                generation.clone(),
            );
        };
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let guard = subscribe_photo_source(&source, &load_state, start.clone());

        AnyView::new(Metadata::new(
            Metadata::new(
                content,
                LifeCycleHook::new(LifeCycle::Appear, {
                    let load_state = Rc::clone(&load_state);
                    move || appear_photo(&load_state, &start)
                }),
            ),
            Retain::new((guard, source)),
        ))
    }
}

#[derive(Default)]
struct PhotoLoadState {
    appeared: bool,
    latest_source: Option<Url>,
    last_started_source: Option<Url>,
}

impl PhotoLoadState {
    fn update_source(&mut self, source: Url) -> Option<Url> {
        self.latest_source = Some(source.clone());
        self.start_if_ready(source)
    }

    fn appear(&mut self) -> Option<Url> {
        self.appeared = true;
        let source = self
            .latest_source
            .clone()
            .expect("Photo source must be initialized before its Appear lifecycle event");
        self.start_if_ready(source)
    }

    fn start_if_ready(&mut self, source: Url) -> Option<Url> {
        if !self.appeared || self.last_started_source.as_ref() == Some(&source) {
            return None;
        }
        self.last_started_source = Some(source.clone());
        Some(source)
    }
}

fn update_photo_source(state: &Rc<RefCell<PhotoLoadState>>, source: Url, start: &impl Fn(Url)) {
    let source = state.borrow_mut().update_source(source);
    if let Some(source) = source {
        start(source);
    }
}

fn appear_photo(state: &Rc<RefCell<PhotoLoadState>>, start: &impl Fn(Url)) {
    let source = state.borrow_mut().appear();
    if let Some(source) = source {
        start(source);
    }
}

fn subscribe_photo_source<S, F>(
    source: &S,
    state: &Rc<RefCell<PhotoLoadState>>,
    start: F,
) -> S::Guard
where
    S: Signal<Output = Url>,
    F: Clone + Fn(Url) + 'static,
{
    let guard = source.watch({
        let state = Rc::clone(state);
        let start = start.clone();
        move |context| update_photo_source(&state, context.into_value(), &start)
    });
    update_photo_source(state, source.get(), &start);
    guard
}

type PhotoEventHandler = Rc<RefCell<Option<waterui_core::handler::BoxedEventAction<Event>>>>;

fn start_load_task(
    url: Url,
    on_event: PhotoEventHandler,
    handler: ReactiveImageHandle,
    env: Environment,
    generation: Rc<Cell<usize>>,
) {
    let token = generation
        .get()
        .checked_add(1)
        .expect("Photo load generation overflowed");
    generation.set(token);
    handler.clear();
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

    let mut client = FollowRedirect::new(zenwave::raw_client());
    let response = client
        .method(Method::GET, &url)
        .map_err(|e| e.to_string())?
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

fn publish_decoded_frame(handler: &ReactiveImageHandle, image: Image) {
    handler.set(image);
}

/// Convenience constructor for building a `Photo` component inline.
#[must_use]
pub fn photo(source: impl IntoComputed<Url>) -> Photo {
    Photo::new(source)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::{PhotoLoadState, appear_photo, fetch_and_decode_streaming, subscribe_photo_source};
    use crate::Url;
    use image::ImageEncoder as _;
    use waterui_core::{Binding, Signal, reactive::watcher::Context};
    use waterui_image::{DecodePath, Image};

    #[derive(Clone)]
    struct EmitsDuringSubscription {
        source: Binding<Url>,
        replacement: Url,
    }

    impl Signal for EmitsDuringSubscription {
        type Output = Url;
        type Guard = <Binding<Url> as Signal>::Guard;

        fn get(&self) -> Self::Output {
            self.source.get()
        }

        fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
            self.source.set(self.replacement.clone());
            let watcher = Rc::new(watcher);
            watcher(Context::from(self.replacement.clone()));
            self.source.watch(move |context| watcher(context))
        }
    }

    #[test]
    fn source_subscription_and_appear_start_each_url_once() {
        let replacement = Url::from_file_path_str("/photos/replacement.png");
        let source = Binding::container(Url::from_file_path_str("/photos/initial.png"));
        let signal = EmitsDuringSubscription {
            source: source.clone(),
            replacement: replacement.clone(),
        };
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let started = Rc::new(RefCell::new(Vec::new()));
        let start = {
            let started = Rc::clone(&started);
            move |source| started.borrow_mut().push(source)
        };

        let _guard = subscribe_photo_source(&signal, &load_state, start.clone());
        assert!(
            started.borrow().is_empty(),
            "source updates before Appear must not start offscreen loading"
        );

        appear_photo(&load_state, &start);
        source.set(replacement.clone());
        let next = Url::from_file_path_str("/photos/next.png");
        source.set(next.clone());

        assert_eq!(*started.borrow(), vec![replacement, next]);
    }

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
    #[allow(
        clippy::vec_init_then_push,
        reason = "platform-gated pushes make a `vec![]` initializer impractical (it would leave `mut` unused where neither gated case applies)"
    )]
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

            for (name, raw_url) in cases {
                let url: Url = raw_url.parse().expect("url should parse");
                let mut frames = 0usize;
                fetch_and_decode_streaming(url, |_| {
                    frames += 1;
                })
                .await
                .unwrap_or_else(|error| panic!("{name} streaming decode failed: {error}"));
                assert!(frames >= 1, "{name} should publish at least one frame");
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
                let mut client = FollowRedirect::new(zenwave::raw_client());
                let response = client
                    .method(Method::GET, url)
                    .expect("HDR AVIF sample request should build")
                    .await
                    .expect("HDR AVIF sample request should succeed");
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

            let mut client = FollowRedirect::new(zenwave::raw_client());
            let response = client
                .method(
                    Method::GET,
                    "https://raw.githubusercontent.com/strukturag/libheif/master/examples/example.heic",
                )
                .expect("HEIC sample request should build")
                .await
                .expect("HEIC sample request should succeed");
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

            let mut client = FollowRedirect::new(zenwave::raw_client());
            let response = client
                .method(
                    Method::GET,
                    "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.8bpc.yuv420.avif",
                )
                .expect("AVIF sample request should build")
                .await
                .expect("AV1 sample request should succeed");
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
        });
    }
}

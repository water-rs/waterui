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
#[cfg(feature = "remote")]
use futures::StreamExt;
#[cfg(target_arch = "wasm32")]
use std::path::Path;
#[cfg(target_arch = "wasm32")]
use waterkit_fs::WaterFs;
use waterui_core::content_ledger::OwedContent;
use waterui_core::event::{LifeCycle, LifeCycleHook};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::reactive::watcher::BoxWatcherGuard;
use waterui_core::{AnyView, Computed, ContentLedger, Environment, Metadata, Retain, Signal, View};
use waterui_image::{
    ContentMode, Image, ReactiveImage, ReactiveImageHandle, RegistrationState, reactive_image,
};
#[cfg(feature = "remote")]
use zenwave::{Client, Method};

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
#[non_exhaustive]
pub enum Event {
    /// The picture is on screen: the source's final decoded frame registered
    /// with the scene backend. Registration settles after the frame is
    /// decoded — a frame the backend refuses reports [`Event::Error`]
    /// instead, never `Loaded`.
    Loaded,
    /// The image failed to load: the source did not decode, or the scene
    /// backend refused a frame's registration. The message is the decode
    /// error or the refusal's [`Display`](core::fmt::Display).
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

    /// Preserves the decoded picture's aspect ratio inside the bounds the
    /// parent proposes: [`ContentMode::Fit`] letterboxes, [`ContentMode::Fill`]
    /// covers the bounds and crops the overflow. Without it a resizable photo
    /// stretches on both axes independently.
    #[must_use]
    pub fn content_mode(mut self, mode: ContentMode) -> Self {
        self.content = self.content.content_mode(mode);
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
    ///     // `Event` is `#[non_exhaustive]`: later releases may add events.
    ///     _ => {}
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
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let start = {
            let load_state = Rc::clone(&load_state);
            move |url| {
                start_load_task(
                    url,
                    Rc::clone(&on_event),
                    content_handler.clone(),
                    env_for_event.clone(),
                    generation.clone(),
                    Rc::clone(&load_state),
                );
            }
        };
        // Each source owes a capture its first decoded frame from the moment
        // it is set — from mount for the initial one — not from `Appear`,
        // which a capture's mount may not have reached yet.
        let owe = {
            let env = env.clone();
            move |url: &Url| ContentLedger::owe::<Self>(&env, url.as_str().to_owned())
        };
        let guard = subscribe_photo_source(&source, &load_state, owe, start.clone());

        AnyView::new(Metadata::new(
            Metadata::new(
                content,
                LifeCycleHook::new(LifeCycle::Appear, {
                    let load_state = Rc::clone(&load_state);
                    move || appear_photo(&load_state, &start)
                }),
            ),
            Retain::new((guard, source, load_state)),
        ))
    }

    /// Resolves through transparent `Metadata` layers to the inner
    /// `ReactiveImage`; forwards its axis (non-resizable by default,
    /// both-axes after `.resizable()`).
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        self.content.stretch_axis()
    }
}

#[derive(Default)]
struct PhotoLoadState {
    appeared: bool,
    latest_source: Option<Url>,
    last_started_source: Option<Url>,
    /// The first frame of `latest_source`, owed to a capture until its
    /// registration settles. Replacing it releases the previous source's
    /// entry.
    owed: Option<OwedContent>,
    /// The watch turning the current source's registration state into its
    /// owed capture and events. Owned here so a source change drops it: a
    /// settlement for a replaced source never fires.
    registration: Option<BoxWatcherGuard>,
}

impl PhotoLoadState {
    fn update_source(
        &mut self,
        source: Url,
        owe: impl FnOnce(&Url) -> Option<OwedContent>,
    ) -> Option<Url> {
        if self.latest_source.as_ref() != Some(&source) {
            self.owed = owe(&source);
            self.registration = None;
        }
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

fn update_photo_source(
    state: &Rc<RefCell<PhotoLoadState>>,
    source: Url,
    owe: impl FnOnce(&Url) -> Option<OwedContent>,
    start: &impl Fn(Url),
) {
    let source = state.borrow_mut().update_source(source, owe);
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

fn subscribe_photo_source<S, O, F>(
    source: &S,
    state: &Rc<RefCell<PhotoLoadState>>,
    owe: O,
    start: F,
) -> S::Guard
where
    S: Signal<Output = Url>,
    O: Clone + Fn(&Url) -> Option<OwedContent> + 'static,
    F: Clone + Fn(Url) + 'static,
{
    let guard = source.watch({
        let state = Rc::clone(state);
        let owe = owe.clone();
        let start = start.clone();
        move |context| update_photo_source(&state, context.into_value(), &owe, &start)
    });
    update_photo_source(state, source.snapshot(), owe, &start);
    guard
}

/// Why a photo's source did not decode, as a capture reports it.
#[derive(Debug)]
struct PhotoLoadError(String);

impl core::fmt::Display for PhotoLoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for PhotoLoadError {}

type PhotoEventHandler = Rc<RefCell<Option<waterui_core::handler::BoxedEventAction<Event>>>>;

fn start_load_task(
    url: Url,
    on_event: PhotoEventHandler,
    handler: ReactiveImageHandle,
    env: Environment,
    generation: Rc<Cell<usize>>,
    load_state: Rc<RefCell<PhotoLoadState>>,
) {
    let (token, settlement) =
        begin_source_load(&handler, &on_event, &env, &generation, &load_state);
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
                decode_finished(
                    &settlement,
                    &generation,
                    token,
                    &load_state,
                    &on_event,
                    &env,
                );
            }
            Err(e) => {
                tracing::error!("[Photo] Failed to load: {}", e);
                if generation.get() == token {
                    // A source whose decode failed reports no later
                    // registration event.
                    let mut settlement = settlement.borrow_mut();
                    let fresh = !matches!(settlement.phase, Phase::Reported);
                    settlement.phase = Phase::Reported;
                    drop(settlement);
                    let owed = load_state.borrow_mut().owed.take();
                    if let Some(owed) = owed {
                        owed.fail(PhotoLoadError(e.clone()));
                    }
                    if fresh && let Some(on_event) = on_event.borrow_mut().as_mut() {
                        on_event(Event::Error(e), &env);
                    }
                }
            }
        }
    })
    .detach();
}

/// What the current source's registration settlements have come to, shared
/// between the registration watch and the decode task that marks `finished`
/// once the stream ends.
#[derive(Debug)]
struct Settlement {
    /// The registration state last observed on the handle:
    /// [`Pending`](RegistrationState::Pending) while the published frame's
    /// recording pass has not settled it yet.
    latest: RegistrationState,
    /// What the source's report still waits for.
    phase: Phase,
    /// The decode stream ended, making the last published frame the final
    /// one.
    finished: bool,
}

/// Which registration settlement a source's report still waits for.
#[derive(Debug)]
enum Phase {
    /// No published frame has settled yet; the first to do so resolves the
    /// owed capture, and a refusal there is the source's error.
    AwaitingFirst,
    /// The first frame resolved the owed capture; the report waits for the
    /// final frame's settlement.
    WatchingFinal,
    /// An `Event` was already reported for this source.
    Reported,
}

impl Settlement {
    const fn new() -> Self {
        Self {
            latest: RegistrationState::Pending,
            phase: Phase::AwaitingFirst,
            finished: false,
        }
    }
}

/// Starts reporting for the source the load is about to fetch: bumps the
/// load generation, clears the published frame, and installs the watch that
/// turns the handle's registration state into the source's owed capture and
/// events. The watch's guard is owned by `load_state`, so a source change
/// ends it. Returns the load's token and the settlement record the decode
/// task marks finished on.
fn begin_source_load(
    handler: &ReactiveImageHandle,
    on_event: &PhotoEventHandler,
    env: &Environment,
    generation: &Rc<Cell<usize>>,
    load_state: &Rc<RefCell<PhotoLoadState>>,
) -> (usize, Rc<RefCell<Settlement>>) {
    let token = generation
        .get()
        .checked_add(1)
        .expect("Photo load generation overflowed");
    generation.set(token);
    handler.clear();
    let settlement = Rc::new(RefCell::new(Settlement::new()));
    let guard = handler.registration().watch({
        let settlement = Rc::clone(&settlement);
        let load_state = Rc::clone(load_state);
        let on_event = Rc::clone(on_event);
        let env = env.clone();
        let generation = Rc::clone(generation);
        move |context| {
            if generation.get() != token {
                return;
            }
            let mut settlement = settlement.borrow_mut();
            settlement.latest = context.into_value();
            settle_registration(&mut settlement, &load_state, &on_event, &env);
        }
    });
    load_state.borrow_mut().registration = Some(guard);
    (token, settlement)
}

/// The decode stream ended: the frame last published is the final one, so
/// its settlement — observed already or still to come — decides the source's
/// `Loaded` report.
fn decode_finished(
    settlement: &Rc<RefCell<Settlement>>,
    generation: &Rc<Cell<usize>>,
    token: usize,
    load_state: &Rc<RefCell<PhotoLoadState>>,
    on_event: &PhotoEventHandler,
    env: &Environment,
) {
    if generation.get() != token {
        return;
    }
    let mut settlement = settlement.borrow_mut();
    settlement.finished = true;
    settle_registration(&mut settlement, load_state, on_event, env);
}

/// Applies the settlement the source stands at. The first published frame's
/// outcome resolves the capture it owes; [`Event::Loaded`] fires on the
/// final frame's registration, and [`Event::Error`] on a refusal of the
/// first or the final frame. A refused progressive frame between them is
/// superseded by the frame that replaces it, so it reports nothing. Each
/// event fires once per source.
fn settle_registration(
    settlement: &mut Settlement,
    load_state: &RefCell<PhotoLoadState>,
    on_event: &PhotoEventHandler,
    env: &Environment,
) {
    if matches!(settlement.phase, Phase::Reported) {
        return;
    }
    match settlement.latest.clone() {
        RegistrationState::Pending => {}
        RegistrationState::Registered => {
            if matches!(settlement.phase, Phase::AwaitingFirst) {
                settlement.phase = Phase::WatchingFinal;
                if let Some(owed) = load_state.borrow_mut().owed.take() {
                    owed.fulfil();
                }
            }
            if settlement.finished {
                settlement.phase = Phase::Reported;
                if let Some(on_event) = on_event.borrow_mut().as_mut() {
                    on_event(Event::Loaded, env);
                }
            }
        }
        RegistrationState::Refused(error) => {
            let first = matches!(settlement.phase, Phase::AwaitingFirst);
            if !(first || settlement.finished) {
                return;
            }
            settlement.phase = Phase::Reported;
            if first && let Some(owed) = load_state.borrow_mut().owed.take() {
                owed.fail(error.clone());
            }
            if let Some(on_event) = on_event.borrow_mut().as_mut() {
                on_event(Event::Error(error.to_string()), env);
            }
        }
    }
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

    #[cfg(feature = "remote")]
    {
        let mut client = zenwave::client();
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

    #[cfg(not(feature = "remote"))]
    {
        let _ = on_decoded_frame;
        Err(alloc::format!(
            "'{url}' is a remote source; enable the `remote` feature of `waterui-media` (or `waterui/remote`) to fetch it"
        ))
    }
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
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[cfg(feature = "remote")]
    use super::fetch_and_decode_streaming;
    use super::{
        Event, Photo, PhotoEventHandler, PhotoLoadState, appear_photo, begin_source_load,
        decode_finished, subscribe_photo_source, update_photo_source,
    };
    use crate::Url;
    use image::ImageEncoder as _;
    use waterui::graphics::draw::{Content, FontId, ImageId, ImageLimits, LayoutSize};
    use waterui::graphics::{
        FontSource, Handle, ImageData, ResourceError, ResourceHandle, Rgba8, Rgba16F, SceneBackend,
        SceneContent, SceneResources, SceneView,
    };
    use waterui_core::handler::BoxedEventAction;
    use waterui_core::{
        AnyView, Binding, ContentLedger, Environment, Signal, View, reactive::watcher::Context,
    };
    use waterui_image::{DecodePath, Image, ReactiveImage, ReactiveImageHandle, reactive_image};
    use waterui_layout::container::FixedContainer;

    #[derive(Clone)]
    struct EmitsDuringSubscription {
        source: Binding<Url>,
        replacement: Url,
    }

    impl Signal for EmitsDuringSubscription {
        type Output = Url;
        type Guard = <Binding<Url> as Signal>::Guard;

        fn snapshot(&self) -> Self::Output {
            self.source.snapshot()
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

        let _guard = subscribe_photo_source(&signal, &load_state, |_: &Url| None, start.clone());
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

    #[test]
    fn each_new_source_owes_a_capture_its_first_frame() {
        let mut env = Environment::new();
        let ledger = ContentLedger::new();
        env.insert(ledger.clone());
        let initial = Url::from_file_path_str("/photos/initial.png");
        let source = Binding::container(initial.clone());
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let owe = move |url: &Url| ContentLedger::owe::<Photo>(&env, url.as_str().to_owned());

        let _guard = subscribe_photo_source(&source, &load_state, owe, |_| {});
        assert_eq!(
            ledger
                .outstanding()
                .iter()
                .map(|entry| (entry.view(), entry.detail().to_string()))
                .collect::<Vec<_>>(),
            vec![("Photo", initial.as_str().to_owned())],
            "the initial source is owed from mount, before Appear"
        );

        source.set(initial);
        assert_eq!(
            ledger.outstanding().len(),
            1,
            "an unchanged source owes nothing new"
        );

        let next = Url::from_file_path_str("/photos/next.png");
        source.set(next.clone());
        assert_eq!(
            ledger
                .outstanding()
                .iter()
                .map(|entry| entry.detail().to_string())
                .collect::<Vec<_>>(),
            vec![next.as_str().to_owned()],
            "a new source releases the previous entry and owes its own"
        );

        load_state
            .borrow_mut()
            .owed
            .take()
            .expect("the current source is owed")
            .fulfil();
        assert_eq!(ledger.pending().snapshot(), 0);
    }

    /// A scene backend that admits every image registration, or refuses
    /// them all, so a published frame's settlement is reproducible without a
    /// GPU.
    struct TestBackend {
        admit: bool,
        attempts: Cell<u64>,
    }

    struct Texture(ImageId);

    impl ResourceHandle for Texture {
        type Id = ImageId;

        fn id(&self) -> ImageId {
            self.0
        }
    }

    impl TestBackend {
        fn admit_image(&self, width: u32, height: u32) -> Result<Handle<ImageId>, ResourceError> {
            self.attempts.set(self.attempts.get() + 1);
            if self.admit {
                Ok(Handle::new(Texture(ImageId::new(self.attempts.get()))))
            } else {
                Err(ResourceError::Image(format!(
                    "{width}x{height} refused by the test backend"
                )))
            }
        }
    }

    impl SceneBackend for TestBackend {
        fn register_font(&self, _source: FontSource) -> Result<Handle<FontId>, ResourceError> {
            Err(ResourceError::Unsupported("fonts"))
        }

        fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Handle<ImageId>, ResourceError> {
            self.admit_image(data.width(), data.height())
        }

        fn register_rgba16f(
            &self,
            data: ImageData<Rgba16F>,
        ) -> Result<Handle<ImageId>, ResourceError> {
            self.admit_image(data.width(), data.height())
        }

        fn image_limits(&self) -> ImageLimits {
            ImageLimits::UNLIMITED
        }
    }

    /// The scene content inside a `ReactiveImage`'s body — the view keeps it
    /// behind a layout container, so a test pulls it out through the public
    /// view API to drive `build_scene` itself.
    fn mounted_content(image: ReactiveImage) -> Box<dyn SceneContent> {
        let body = AnyView::new(image.body(&Environment::new()));
        let container = body
            .downcast::<FixedContainer>()
            .unwrap_or_else(|_| panic!("a reactive image mounts inside a fixed container"));
        let (_, mut contents) = container.into_inner();
        let scene = contents
            .pop()
            .and_then(|view| view.downcast::<SceneView>().ok())
            .expect("a reactive image's only content is a scene view");
        scene.into_content()
    }

    /// Records one frame of `content` against `resources`, running the
    /// registration the published frame attempts.
    fn settle_frame(content: &mut dyn SceneContent, resources: &SceneResources) {
        let mut recording = resources.recording();
        let recorded = Content::record(&LayoutSize::new(), |recorder| {
            content.build_scene(recorder, &mut recording, 8.0, 8.0);
        });
        let _held = recording.finish();
        drop(recorded);
    }

    /// An event handler recording the debug form of each event it fires, in
    /// order.
    fn recorded_events() -> (Rc<RefCell<Vec<String>>>, PhotoEventHandler) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let handler: PhotoEventHandler = Rc::new(RefCell::new(Some(Box::new({
            let events = Rc::clone(&events);
            move |event: Event, _: &Environment| {
                events.borrow_mut().push(match event {
                    Event::Loaded => String::from("Loaded"),
                    Event::Error(message) => format!("Error({message})"),
                });
            }
        })
            as BoxedEventAction<Event>)));
        (events, handler)
    }

    /// A photo load under test: the handle drives published frames, the
    /// extracted scene content runs their registration against `resources`,
    /// and `events` records what the handler fired.
    struct LoadTest {
        env: Environment,
        ledger: ContentLedger,
        handle: ReactiveImageHandle,
        content: Box<dyn SceneContent>,
        resources: SceneResources,
        generation: Rc<Cell<usize>>,
        load_state: Rc<RefCell<PhotoLoadState>>,
        events: Rc<RefCell<Vec<String>>>,
        on_event: PhotoEventHandler,
    }

    fn load_test_setup(admit: bool) -> LoadTest {
        waterui_testing::install_test_executor();
        let mut env = Environment::new();
        let ledger = ContentLedger::new();
        env.insert(ledger.clone());
        let (handle, image) = reactive_image();
        let content = mounted_content(image);
        let backend = Rc::new(TestBackend {
            admit,
            attempts: Cell::new(0),
        });
        let resources = SceneResources::new(backend);
        let generation = Rc::new(Cell::new(0));
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let (events, on_event) = recorded_events();
        LoadTest {
            env,
            ledger,
            handle,
            content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        }
    }

    #[test]
    fn a_refused_first_frame_fails_the_capture_and_reports_error() {
        let LoadTest {
            env,
            ledger,
            handle,
            mut content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        } = load_test_setup(false);
        load_state.borrow_mut().owed = ContentLedger::owe::<Photo>(&env, "refused.png".to_owned());
        let (token, settlement) =
            begin_source_load(&handle, &on_event, &env, &generation, &load_state);

        // Publish the first frame and settle its registration — refused.
        handle.set(Image::new(vec![255; 4], 1, 1));
        settle_frame(&mut *content, &resources);
        assert!(
            events.borrow().is_empty(),
            "the refusal reports from the executor, not inside the recording pass"
        );
        assert_eq!(waterui_testing::drain_parked_local_work(), 1);

        let fired = events.borrow();
        let [event] = fired.as_slice() else {
            panic!("a refused first frame reports one error: {fired:?}");
        };
        assert!(
            event.starts_with("Error(the scene backend rejected the image"),
            "{event}"
        );
        drop(fired);
        let failure = futures_lite::future::block_on(ledger.settled())
            .expect_err("the owed capture fails with the refusal");
        assert!(
            failure
                .get_source()
                .to_string()
                .contains("the scene backend rejected the image"),
            "{failure}"
        );

        // The stream ending afterwards cannot turn the report into Loaded.
        decode_finished(
            &settlement,
            &generation,
            token,
            &load_state,
            &on_event,
            &env,
        );
        assert_eq!(
            events.borrow().len(),
            1,
            "a refused source never reports Loaded"
        );
    }

    #[test]
    fn a_registered_final_frame_reports_loaded_once() {
        let LoadTest {
            env,
            ledger,
            handle,
            mut content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        } = load_test_setup(true);
        load_state.borrow_mut().owed =
            ContentLedger::owe::<Photo>(&env, "registered.png".to_owned());
        let (token, settlement) =
            begin_source_load(&handle, &on_event, &env, &generation, &load_state);

        // The first published frame registers: the owed capture is
        // fulfilled, but nothing is reported while the stream runs.
        handle.set(Image::new(vec![255; 4], 1, 1));
        settle_frame(&mut *content, &resources);
        assert_eq!(waterui_testing::drain_parked_local_work(), 1);
        assert_eq!(
            ledger.pending().snapshot(),
            0,
            "the first registered frame fulfils the owed capture"
        );
        assert!(
            events.borrow().is_empty(),
            "Loaded waits for the final frame"
        );

        // The stream ends — the published frame is final.
        decode_finished(
            &settlement,
            &generation,
            token,
            &load_state,
            &on_event,
            &env,
        );
        assert_eq!(*events.borrow(), vec![String::from("Loaded")]);

        // Re-recording the same frame reports nothing further.
        settle_frame(&mut *content, &resources);
        assert_eq!(waterui_testing::drain_parked_local_work(), 0);
        assert_eq!(events.borrow().len(), 1);
    }

    #[test]
    fn a_settlement_for_a_replaced_source_reports_nothing() {
        let LoadTest {
            env,
            ledger,
            handle,
            mut content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        } = load_test_setup(false);

        // Source A publishes a frame whose registration settles refused; the
        // settlement is queued for the executor but not delivered yet.
        load_state.borrow_mut().appeared = true;
        load_state.borrow_mut().owed = ContentLedger::owe::<Photo>(&env, "replaced.png".to_owned());
        let _ = begin_source_load(&handle, &on_event, &env, &generation, &load_state);
        handle.set(Image::new(vec![255; 4], 1, 1));
        settle_frame(&mut *content, &resources);

        // The source changes before the settlement lands: the watch is
        // dropped with the old source's load state, and the new source owes
        // its own capture.
        let next = Url::from_file_path_str("/photos/next.png");
        update_photo_source(
            &load_state,
            next.clone(),
            |url| ContentLedger::owe::<Photo>(&env, url.as_str().to_owned()),
            &{
                let handle = handle.clone();
                let on_event = Rc::clone(&on_event);
                let env = env.clone();
                let generation = Rc::clone(&generation);
                let load_state = Rc::clone(&load_state);
                move |_| {
                    let _ = begin_source_load(&handle, &on_event, &env, &generation, &load_state);
                }
            },
        );

        assert_eq!(
            waterui_testing::drain_parked_local_work(),
            1,
            "the replaced source's settlement was still delivered"
        );
        assert!(
            events.borrow().is_empty(),
            "a settlement for a replaced source reports nothing"
        );
        assert_eq!(
            ledger
                .outstanding()
                .iter()
                .map(|entry| entry.detail().to_string())
                .collect::<Vec<_>>(),
            vec![next.as_str().to_owned()],
            "the previous source's entry was released"
        );

        // And the current source's own settlement still reports.
        handle.set(Image::new(vec![255; 4], 1, 1));
        settle_frame(&mut *content, &resources);
        assert_eq!(waterui_testing::drain_parked_local_work(), 1);
        assert_eq!(events.borrow().len(), 1);
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

    #[cfg(feature = "remote")]
    #[test]
    #[ignore = "requires network access to real image URLs"]
    #[allow(
        clippy::vec_init_then_push,
        reason = "platform-gated pushes make a `vec![]` initializer impractical (it would leave `mut` unused where neither gated case applies)"
    )]
    fn streaming_decode_real_images_smoke() {
        futures_lite::future::block_on(async {
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

    #[cfg(all(
        feature = "remote",
        any(target_vendor = "apple", target_os = "android")
    ))]
    #[test]
    #[ignore = "requires network access and platform HDR AVIF decode support"]
    fn hdr_avif_decode_real_image_smoke() {
        futures_lite::future::block_on(async {
            use zenwave::{Client, Method};

            let candidates = [
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.10bpc.yuv420.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/hato.profile0.10bpc.yuv420.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile1.10bpc.yuv444.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile2.10bpc.yuv422.avif",
            ];

            let mut selected = None;
            for url in candidates {
                let mut client = zenwave::client();
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

    #[cfg(all(feature = "remote", target_vendor = "apple"))]
    #[test]
    #[ignore = "requires network access and Apple HEIC platform decode support"]
    fn heic_h265_decode_real_image_smoke() {
        futures_lite::future::block_on(async {
            use zenwave::{Client, Method};

            let mut client = zenwave::client();
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

    #[cfg(feature = "remote")]
    #[test]
    #[ignore = "requires network access and AV1/AVIF decode support"]
    fn av1_decode_fallback_probe_smoke() {
        futures_lite::future::block_on(async {
            use zenwave::{Client, Method};

            let mut client = zenwave::client();
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
